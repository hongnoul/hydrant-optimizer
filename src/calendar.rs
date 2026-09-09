//! Calendar export with dated New York events.
use anyhow::{Context, Result, ensure};
use chrono::{Datelike, LocalResult, NaiveDate, TimeZone};
use chrono_tz::America::New_York;
use sha2::{Digest, Sha256};

use crate::color::{self, CourseColor};
use crate::model::{
    ChosenSection, Meeting, TermCalendar, bounded_pe_meetings, is_pe_kind,
    meetings_have_date_limits,
};

#[derive(Clone, Debug, serde::Serialize)]
pub struct ExportReport {
    pub ics: String,
    pub event_count: usize,
    pub notices: Vec<String>,
    /// Actual file written: the caller's hint plus a Unix-time suffix, e.g.
    /// `schedule-1757424600.ics`. The bare hint is never created.
    pub path: std::path::PathBuf,
}

pub fn export_ics(calendar: &TermCalendar, chosen: &[ChosenSection]) -> Result<ExportReport> {
    ensure!(calendar.start <= calendar.end, "calendar start follows end");
    ensure!(
        calendar.alternate_days.values().all(|weekday| *weekday < 7),
        "alternate calendar weekdays must be Monday through Sunday"
    );
    let mut notices = Vec::new();
    let mut events = Vec::new();
    // Same nth-distinct-course order as the TUI so ICS colors match the grid.
    let ordered_courses = color::sorted_course_ids(chosen);

    for section in chosen {
        let course_color = CourseColor::for_course(&section.course_id, &ordered_courses);
        if let Some(reason) = &section.section.unsupported_reason {
            notices.push(format!(
                "omitted {} {} {}: {reason}",
                section.course_id, section.kind, section.section.label
            ));
            continue;
        }
        if section.section.meetings.is_empty() {
            notices.push(format!(
                "omitted {} {} {}: no known meeting times",
                section.course_id, section.kind, section.section.label
            ));
            continue;
        }
        let bounded_pe = bounded_pe_meetings(&section.kind, &section.section.meetings);
        if is_pe_kind(&section.kind) && !bounded_pe {
            notices.push(format!(
                "omitted {} {} {}: PE meetings require complete start/end date bounds",
                section.course_id, section.kind, section.section.label
            ));
            continue;
        }
        if meetings_have_date_limits(&section.section.meetings) && !bounded_pe {
            notices.push(format!(
                "omitted {} {} {}: date-limited meetings are not exported by the weekly calendar exporter",
                section.course_id, section.kind, section.section.label
            ));
            continue;
        }

        let before = events.len();
        for meeting in &section.section.meetings {
            meeting.validate().with_context(|| {
                format!(
                    "invalid meeting for {} {} {}",
                    section.course_id, section.kind, section.section.label
                )
            })?;
            for date in matching_dates(calendar, meeting, bounded_pe) {
                let start = local_to_utc(date, meeting.start_minute).with_context(|| {
                    format!(
                        "cannot localize start of {} on {date}",
                        section_label(section)
                    )
                })?;
                let end = local_to_utc(date, meeting.end_minute).with_context(|| {
                    format!(
                        "cannot localize end of {} on {date}",
                        section_label(section)
                    )
                })?;
                events.push(Event {
                    uid: stable_uid(calendar, section, meeting, date),
                    start,
                    end,
                    summary: format!(
                        "{} {}",
                        section.course_id,
                        color::component_label(&section.kind)
                    ),
                    location: section.section.room.clone(),
                    description: format!(
                        "{}\n{} {} {}\nColor: {} {} (Google Calendar event colorId {})",
                        section.course_title,
                        section.course_id,
                        section.kind,
                        section.section.label,
                        course_color.gcal_name,
                        course_color.hex,
                        course_color.gcal_id,
                    ),
                    color: course_color.hex.to_string(),
                    categories: format!(
                        "hydrant-optimizer,{},{}",
                        section.course_id, course_color.gcal_name,
                    ),
                    hydrant_course: section.course_id.clone(),
                    hydrant_color_name: course_color.gcal_name.to_string(),
                    hydrant_color_hex: course_color.hex.to_string(),
                    hydrant_color_id: course_color.gcal_id.to_string(),
                });
            }
        }
        if events.len() == before {
            notices.push(format!(
                "omitted {} {} {}: no class dates matched the term calendar",
                section.course_id, section.kind, section.section.label
            ));
        }
    }

    events.sort_by(|a, b| (&a.start, &a.end, &a.uid).cmp(&(&b.start, &b.end, &b.uid)));
    events.dedup_by(|a, b| a.uid == b.uid);
    notices.sort();
    notices.dedup();
    let event_count = events.len();
    Ok(ExportReport {
        ics: render_calendar(&events),
        event_count,
        notices,
        path: std::path::PathBuf::new(),
    })
}

#[derive(Clone, Debug)]
struct Event {
    uid: String,
    start: String,
    end: String,
    summary: String,
    location: String,
    description: String,
    color: String,
    categories: String,
    hydrant_course: String,
    hydrant_color_name: String,
    hydrant_color_hex: String,
    hydrant_color_id: String,
}

fn matching_dates(calendar: &TermCalendar, meeting: &Meeting, bounded: bool) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    let mut date = if bounded {
        meeting
            .start_date
            .expect("bounded PE meeting has a start date")
            .max(calendar.start)
    } else {
        calendar.start
    };
    let end = if bounded {
        meeting
            .end_date
            .expect("bounded PE meeting has an end date")
            .min(calendar.end)
    } else {
        calendar.end
    };
    while date <= end {
        if !calendar.holidays.contains(&date) {
            let effective_weekday = calendar
                .alternate_days
                .get(&date)
                .copied()
                .unwrap_or_else(|| date.weekday().num_days_from_monday() as u8);
            if effective_weekday == meeting.weekday {
                dates.push(date);
            }
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    dates
}

fn local_to_utc(date: NaiveDate, minute: u16) -> Result<String> {
    let (date, minute) = if minute == 1440 {
        (
            date.succ_opt()
                .context("midnight extends beyond supported calendar range")?,
            0,
        )
    } else {
        (date, minute)
    };
    let local = date
        .and_hms_opt((minute / 60) as u32, (minute % 60) as u32, 0)
        .context("meeting time is outside a calendar day")?;
    let zoned = match New_York.from_local_datetime(&local) {
        LocalResult::Single(time) => time,
        LocalResult::Ambiguous(_, _) => {
            anyhow::bail!("local time is ambiguous in America/New_York")
        }
        LocalResult::None => anyhow::bail!("local time does not exist in America/New_York"),
    };
    Ok(zoned
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string())
}

fn stable_uid(
    calendar: &TermCalendar,
    section: &ChosenSection,
    meeting: &Meeting,
    date: NaiveDate,
) -> String {
    let bytes = serde_json::to_vec(&(
        calendar.start,
        calendar.end,
        &section.course_id,
        &section.kind,
        &section.section.id,
        date,
        meeting.weekday,
        meeting.start_minute,
        meeting.end_minute,
    ))
    .expect("serializing stable calendar UID input cannot fail");
    format!("{:x}@hydrant-optimizer", Sha256::digest(bytes))
}

fn render_calendar(events: &[Event]) -> String {
    let mut text = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//hydrant-optimizer//EN\r\nCALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\n",
    );
    for event in events {
        text.push_str("BEGIN:VEVENT\r\n");
        push_property(&mut text, "UID", &event.uid);
        text.push_str("DTSTAMP:19700101T000000Z\r\n");
        text.push_str("DTSTART:");
        text.push_str(&event.start);
        text.push_str("\r\n");
        text.push_str("DTEND:");
        text.push_str(&event.end);
        text.push_str("\r\n");
        push_property(&mut text, "SUMMARY", &event.summary);
        // MIT Hydrant always emits LOCATION (its `event.room` may be empty),
        // so Google/Apple Calendar show a location field consistently instead
        // of hiding it for TBA sections. Fall back to "TBA" only when no
        // room is known; otherwise preserve the raw room string verbatim.
        let location = if event.location.trim().is_empty() {
            "TBA".to_string()
        } else {
            event.location.clone()
        };
        push_property(&mut text, "LOCATION", &location);
        push_property(&mut text, "DESCRIPTION", &event.description);
        // RFC 7986 display color. Apple Calendar honors it; Google Calendar
        // ignores per-event colors on ICS import (events take the calendar
        // color instead), so CATEGORIES + DESCRIPTION + X-HYDRANT-* repeat the
        // same assignment for filtering and one-click manual recoloring.
        push_property(&mut text, "COLOR", &event.color);
        push_property(&mut text, "CATEGORIES", &event.categories);
        push_property(&mut text, "X-HYDRANT-COURSE", &event.hydrant_course);
        push_property(&mut text, "X-HYDRANT-COLOR-NAME", &event.hydrant_color_name);
        push_property(&mut text, "X-HYDRANT-COLOR", &event.hydrant_color_hex);
        push_property(
            &mut text,
            "X-HYDRANT-GCAL-COLOR-ID",
            &event.hydrant_color_id,
        );
        text.push_str("END:VEVENT\r\n");
    }
    text.push_str("END:VCALENDAR\r\n");
    text
}

fn push_property(text: &mut String, name: &str, value: &str) {
    // RFC 5545 content lines are folded at 75 octets, never inside UTF-8.
    let line = format!("{name}:{}", escape_text(value));
    let mut width = 0;
    for character in line.chars() {
        if width + character.len_utf8() > 75 {
            text.push_str("\r\n ");
            width = 1;
        }
        text.push(character);
        width += character.len_utf8();
    }
    text.push_str("\r\n");
}

fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

fn section_label(section: &ChosenSection) -> String {
    format!(
        "{} {} {}",
        section.course_id, section.kind, section.section.label
    )
}
