//! Hydrant raw JSON adapter.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use chrono::{Datelike, NaiveDate};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::model::{Course, Dataset, Meeting, Requirement, SectionOption, Source, TermCalendar};

const KNOWN_KINDS: [&str; 4] = ["lecture", "recitation", "lab", "design"];

pub fn parse_catalog(catalog: &str, term: &str) -> Result<Dataset> {
    let catalog_json: Value = serde_json::from_str(catalog).context("catalog JSON is malformed")?;
    let term_json: Value = if term.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(term).context("term JSON is malformed")?
    };

    let catalog_term = catalog_json.get("termInfo").cloned().unwrap_or(Value::Null);
    let term_info = select_term_info(&catalog_term, &term_json)?;
    let term_id = string_field(&term_info, "urlName")
        .or_else(|| string_field(&catalog_term, "urlName"))
        .context("term metadata is missing urlName")?;
    let calendar = parse_calendar(&term_info)
        .with_context(|| format!("invalid calendar metadata for {term_id}"))?;

    let classes = catalog_json
        .get("classes")
        .and_then(Value::as_object)
        .context("catalog is missing classes object")?;
    ensure!(!classes.is_empty(), "catalog contains no classes");

    let mut courses = BTreeMap::new();
    let mut notices = Vec::new();
    for (key, raw_course) in classes {
        let course = parse_course(key, raw_course, &calendar, &term_info)
            .with_context(|| format!("invalid course {key}"))?;
        if courses.insert(course.id.clone(), course).is_some() {
            bail!("duplicate course id in catalog: {key}");
        }
    }
    ensure!(!courses.is_empty(), "catalog contains no usable classes");

    let last_updated =
        string_field(&catalog_json, "lastUpdated").unwrap_or_else(|| "unknown".to_string());
    if last_updated == "unknown" {
        notices.push("catalog did not include lastUpdated metadata".to_string());
    }

    Ok(Dataset {
        term_id,
        last_updated,
        calendar: Some(calendar),
        courses,
        notices,
    })
}

fn select_term_info(catalog_term: &Value, term_json: &Value) -> Result<Value> {
    let catalog_url = string_field(catalog_term, "urlName");
    if !term_json.is_null() {
        if term_json.get("urlName").is_some() {
            let url = string_field(term_json, "urlName")
                .context("term metadata urlName is not a string")?;
            if let Some(expected) = &catalog_url {
                ensure!(
                    url == *expected,
                    "term metadata {url} does not match catalog term {expected}"
                );
            }
            return Ok(term_json.clone());
        }
        if let Some(expected) = &catalog_url {
            for key in ["semester", "preSemester"] {
                if let Some(candidate) = term_json.get(key)
                    && string_field(candidate, "urlName").as_deref() == Some(expected.as_str())
                {
                    return Ok(candidate.clone());
                }
            }
            if let Some(object) = term_json.as_object() {
                for candidate in object.values() {
                    if candidate.is_object()
                        && string_field(candidate, "urlName").as_deref() == Some(expected.as_str())
                    {
                        return Ok(candidate.clone());
                    }
                }
            }
            bail!("term metadata does not contain catalog term {expected}");
        }
        if let Some(candidate) = term_json.get("semester") {
            return Ok(candidate.clone());
        }
    }
    ensure!(
        catalog_term.is_object(),
        "catalog is missing termInfo and no matching term metadata was supplied"
    );
    Ok(catalog_term.clone())
}

fn parse_calendar(term_info: &Value) -> Result<TermCalendar> {
    let start = parse_date_field(term_info, "startDate")?;
    let end = parse_date_field(term_info, "endDate")?;
    ensure!(start <= end, "term startDate follows endDate");

    let mut holidays = BTreeSet::new();
    if let Some(value) = term_info.get("holidayDates") {
        let values = value.as_array().context("holidayDates must be an array")?;
        for value in values {
            let text = value
                .as_str()
                .context("holidayDates entries must be strings")?;
            holidays
                .insert(parse_date(text).with_context(|| format!("invalid holiday date {text}"))?);
        }
    }

    let mut alternate_days = BTreeMap::new();
    for (field, weekday) in [
        ("mondayScheduleDate", 0),
        ("tuesdayScheduleDate", 1),
        ("wednesdayScheduleDate", 2),
        ("thursdayScheduleDate", 3),
        ("fridayScheduleDate", 4),
        ("saturdayScheduleDate", 5),
        ("sundayScheduleDate", 6),
    ] {
        add_alternate_day(term_info, field, weekday, &mut alternate_days)?;
        let plural = format!("{field}s");
        add_alternate_day(term_info, &plural, weekday, &mut alternate_days)?;
    }

    Ok(TermCalendar {
        start,
        end,
        holidays,
        alternate_days,
    })
}

fn add_alternate_day(
    term_info: &Value,
    field: &str,
    weekday: u8,
    alternate_days: &mut BTreeMap<NaiveDate, u8>,
) -> Result<()> {
    let Some(value) = term_info.get(field) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    if let Some(text) = value.as_str() {
        insert_alternate_day(
            alternate_days,
            parse_date(text).with_context(|| format!("invalid {field}"))?,
            weekday,
        )?;
        return Ok(());
    }
    if let Some(values) = value.as_array() {
        for value in values {
            let text = value
                .as_str()
                .with_context(|| format!("{field} entries must be strings"))?;
            insert_alternate_day(
                alternate_days,
                parse_date(text).with_context(|| format!("invalid {field}"))?,
                weekday,
            )?;
        }
        return Ok(());
    }
    bail!("{field} must be a date string or array of date strings")
}

fn insert_alternate_day(
    days: &mut BTreeMap<NaiveDate, u8>,
    date: NaiveDate,
    weekday: u8,
) -> Result<()> {
    if let Some(previous) = days.insert(date, weekday) {
        ensure!(
            previous == weekday,
            "conflicting alternate weekday schedules for {date}"
        );
    }
    Ok(())
}

fn parse_course(
    key: &str,
    raw: &Value,
    calendar: &TermCalendar,
    term_info: &Value,
) -> Result<Course> {
    let Some(object) = raw.as_object() else {
        bail!("course {key} is not an object");
    };
    let id = string_field(raw, "number").unwrap_or_else(|| key.to_string());
    ensure!(!id.trim().is_empty(), "course number is empty");
    let raw_title = string_field(raw, "name")
        .or_else(|| string_field(raw, "title"))
        .unwrap_or_default();
    let title = if raw_title.trim().is_empty() {
        id.clone()
    } else {
        raw_title.trim().to_string()
    };

    let mut kinds = Vec::new();
    if let Some(value) = object.get("sectionKinds") {
        let values = value.as_array().context("sectionKinds must be an array")?;
        for value in values {
            let kind = value
                .as_str()
                .context("sectionKinds entries must be strings")?
                .trim()
                .to_lowercase();
            if !kind.is_empty() && !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
    }
    for kind in KNOWN_KINDS {
        let field = sections_field(kind);
        if let Some(value) = object.get(&field) {
            ensure!(value.is_array(), "{field} must be an array");
        }
        if object
            .get(&field)
            .and_then(Value::as_array)
            .is_some_and(|sections| !sections.is_empty())
            && !kinds.iter().any(|existing| existing == kind)
        {
            kinds.push(kind.to_string());
        }
    }

    let date_limit = course_date_limit(raw, calendar, term_info)?;
    let unsupported_reason = date_limit.unsupported.then(|| {
        if date_limit.start.is_some() || date_limit.end.is_some() {
            "date-limited API section: partial-term optimization is not supported".to_string()
        } else {
            "partial-term API section: weekly optimization is not supported".to_string()
        }
    });

    let mut requirements = Vec::new();
    let mut notices = Vec::new();
    for kind in kinds {
        let field = sections_field(&kind);
        let raw_sections = object
            .get(&field)
            .map(|value| {
                value
                    .as_array()
                    .with_context(|| format!("{field} must be an array"))
            })
            .transpose()?;
        let raw_labels = object
            .get(&format!("{kind}RawSections"))
            .and_then(Value::as_array);
        let mut options = Vec::new();
        let mut seen_raw_options = BTreeSet::new();
        if let Some(sections) = raw_sections {
            for (index, section) in sections.iter().enumerate() {
                let option = parse_section(
                    &id,
                    &kind,
                    index,
                    section,
                    raw_labels,
                    &date_limit,
                    unsupported_reason.clone(),
                )
                .with_context(|| format!("invalid {id} {kind} section {}", index + 1))?;
                if seen_raw_options.insert((
                    option.meetings.clone(),
                    option.room.clone(),
                    option.unsupported_reason.clone(),
                )) {
                    options.push(option);
                }
            }
        }
        options.sort_by(|a, b| a.id.cmp(&b.id));
        let has_unknown_times =
            options.is_empty() || options.iter().any(|option| option.meetings.is_empty());
        if unsupported_reason.is_some() && !options.is_empty() {
            notices.push(format!("{kind}: date-limited published options are preserved but omitted from weekly optimization"));
        }
        if options.is_empty() {
            notices.push(format!("{kind}: no known meeting times"));
        }
        requirements.push(Requirement {
            id: format!("{id}/{kind}"),
            kind,
            options,
            has_unknown_times,
        });
    }

    Ok(Course {
        id,
        title,
        requirements,
        notices,
    })
}

fn parse_section(
    course_id: &str,
    kind: &str,
    index: usize,
    section: &Value,
    raw_labels: Option<&Vec<Value>>,
    date_limit: &DateLimit,
    unsupported_reason: Option<String>,
) -> Result<SectionOption> {
    let items = section
        .as_array()
        .context("section must be [meetings, room]")?;
    ensure!(
        items.len() == 2,
        "section must include exactly meetings and room"
    );
    let meetings_value = items.first().context("missing meetings")?;
    let room = items
        .get(1)
        .and_then(Value::as_str)
        .context("section room must be a string")?
        .trim()
        .to_string();
    let mut meetings = Vec::new();
    let raw_meetings = meetings_value
        .as_array()
        .context("section meetings must be an array")?;
    for meeting in raw_meetings {
        let pair = meeting
            .as_array()
            .context("raw meeting must be [slot, length]")?;
        ensure!(pair.len() == 2, "raw meeting must be [slot, length]");
        let slot = pair[0]
            .as_i64()
            .context("raw meeting slot must be an integer")?;
        let length = pair[1]
            .as_i64()
            .context("raw meeting length must be an integer")?;
        ensure!(
            (0..7 * 34).contains(&slot),
            "raw meeting slot must be within the encoded week"
        );
        ensure!(
            (1..=48).contains(&length),
            "raw meeting length must be positive and at most one day"
        );
        let weekday = (slot / 34) as u8;
        let start_minute = 360 + 30 * (slot % 34);
        let end_minute = start_minute + 30 * length;
        ensure!(end_minute <= 1440, "raw meeting extends past midnight");
        let meeting = Meeting {
            weekday,
            start_minute: start_minute as u16,
            end_minute: end_minute as u16,
            start_date: date_limit.start,
            end_date: date_limit.end,
        };
        meeting.validate()?;
        meetings.push(meeting);
    }
    meetings.sort();
    meetings.dedup();
    let mut option_unsupported = unsupported_reason;
    if meetings.is_empty() {
        option_unsupported.get_or_insert_with(|| "section has no known meeting times".to_string());
    }
    let label = raw_labels
        .and_then(|labels| labels.get(index))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{} {}", title_case(kind), index + 1));

    Ok(SectionOption {
        id: api_option_id(course_id, kind, &meetings, &room),
        label,
        room,
        source: Source::Api,
        meetings,
        incompatible_with: BTreeSet::new(),
        unsupported_reason: option_unsupported,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct DateLimit {
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    unsupported: bool,
}

fn course_date_limit(raw: &Value, calendar: &TermCalendar, term_info: &Value) -> Result<DateLimit> {
    let mut limit = DateLimit::default();
    let has_quarter_info = raw.get("quarterInfo").is_some();
    match raw.get("half") {
        Some(Value::Number(number)) => {
            ensure!(
                matches!(number.as_i64(), Some(0..=2)),
                "half must be a boolean or 0, 1, or 2"
            );
            limit.unsupported = number.as_i64() != Some(0);
            if !has_quarter_info && number.as_i64() == Some(1) {
                limit.start = Some(calendar.start);
                limit.end = optional_date_field(term_info, "h1EndDate")?;
            } else if !has_quarter_info && number.as_i64() == Some(2) {
                limit.start = optional_date_field(term_info, "h2StartDate")?;
                limit.end = Some(calendar.end);
            }
        }
        Some(Value::Bool(true)) => {
            limit.unsupported = true;
        }
        None | Some(Value::Bool(false)) => {}
        _ => bail!("half must be a boolean or 0, 1, or 2"),
    }

    if let Some(quarter) = raw.get("quarterInfo").filter(|value| !value.is_null()) {
        ensure!(quarter.is_object(), "quarterInfo must be an object");
        limit.unsupported = true;
        if let Some(start) = quarter.get("start") {
            limit.start =
                Some(parse_month_day(start, calendar).context("invalid quarterInfo start")?);
        }
        if let Some(end) = quarter.get("end") {
            limit.end = Some(parse_month_day(end, calendar).context("invalid quarterInfo end")?);
        }
    }
    if let (Some(start), Some(end)) = (limit.start, limit.end) {
        ensure!(start <= end, "course date limit starts after it ends");
    }
    Ok(limit)
}

fn api_option_id(course_id: &str, kind: &str, meetings: &[Meeting], room: &str) -> String {
    let bytes = serde_json::to_vec(&(course_id, kind, meetings, room))
        .expect("serializing stable API option ID input cannot fail");
    format!("api-{:x}", Sha256::digest(bytes))
}

fn parse_month_day(value: &Value, calendar: &TermCalendar) -> Result<NaiveDate> {
    let array = value
        .as_array()
        .context("month/day value must be [month, day]")?;
    ensure!(array.len() == 2, "month/day value must be [month, day]");
    let month = u32::try_from(array[0].as_u64().context("month must be an integer")?)?;
    let day = u32::try_from(array[1].as_u64().context("day must be an integer")?)?;
    let mut date =
        NaiveDate::from_ymd_opt(calendar.start.year(), month, day).context("invalid month/day")?;
    if date < calendar.start && calendar.end.year() != calendar.start.year() {
        date = NaiveDate::from_ymd_opt(calendar.end.year(), month, day)
            .context("invalid month/day")?;
    }
    Ok(date)
}

fn parse_date_field(value: &Value, field: &str) -> Result<NaiveDate> {
    let text = string_field(value, field).with_context(|| format!("missing {field}"))?;
    parse_date(&text).with_context(|| format!("invalid {field}: {text}"))
}

fn optional_date_field(value: &Value, field: &str) -> Result<Option<NaiveDate>> {
    let Some(raw) = value.get(field).filter(|raw| !raw.is_null()) else {
        return Ok(None);
    };
    let text = raw
        .as_str()
        .with_context(|| format!("{field} must be a date string"))?;
    Ok(Some(
        parse_date(text).with_context(|| format!("invalid {field}: {text}"))?,
    ))
}

fn parse_date(text: &str) -> Result<NaiveDate> {
    Ok(NaiveDate::parse_from_str(text, "%Y-%m-%d")?)
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field)?.as_str().map(ToOwned::to_owned)
}

fn sections_field(kind: &str) -> String {
    format!("{kind}Sections")
}

fn title_case(kind: &str) -> String {
    let mut chars = kind.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Section".to_string(),
    }
}
