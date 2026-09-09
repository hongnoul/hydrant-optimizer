//! Network/cache storage and manual overlay handling.
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{adapter, model::*};

const CACHE_VERSION: u32 = 1;
const MANUAL_VERSION: u32 = 1;
const CACHE_FILE: &str = "catalog-cache.json";
const MANUAL_FILE: &str = "manual.json";
const LATEST_CATALOG_URL: &str = "https://hydrant.mit.edu/latest.json";
const LATEST_TERM_URL: &str = "https://hydrant.mit.edu/latestTerm.json";

#[derive(Debug, Serialize, Deserialize)]
struct CacheEnvelope {
    version: u32,
    fetched_at: String,
    catalog_url: String,
    term_url: String,
    catalog: String,
    term: String,
}

pub fn load_dataset(dir: &Path, offline: bool) -> Result<Dataset> {
    if offline {
        let envelope = read_cache(dir)
            .with_context(|| format!("offline cache is unavailable in {}", dir.display()))?;
        return dataset_from_cache(&envelope).context("offline cache is corrupt or incompatible");
    }

    match fetch_dataset() {
        Ok((mut dataset, envelope)) => {
            write_cache(dir, &envelope)
                .with_context(|| format!("cannot write validated cache in {}", dir.display()))?;
            dataset.notices.sort();
            dataset.notices.dedup();
            Ok(dataset)
        }
        Err(fetch_error) => {
            let envelope = read_cache(dir).with_context(|| {
                format!(
                    "online fetch failed ({fetch_error:#}) and no valid cache was available in {}",
                    dir.display()
                )
            })?;
            let mut dataset = dataset_from_cache(&envelope).with_context(|| {
                format!("online fetch failed ({fetch_error:#}) and cached catalog is corrupt")
            })?;
            dataset
                .notices
                .push(cache_fallback_notice(&fetch_error, &envelope));
            dataset.notices.sort();
            dataset.notices.dedup();
            Ok(dataset)
        }
    }
}

pub fn load_manual(dir: &Path) -> Result<ManualStore> {
    let path = manual_path(dir);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ManualStore::default());
        }
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let store: ManualStore =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    validate_manual_store(&store).with_context(|| format!("invalid {}", path.display()))?;
    Ok(store)
}

pub fn save_manual(dir: &Path, store: &ManualStore) -> Result<()> {
    validate_manual_store(store)?;
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let path = manual_path(dir);
    let bytes = serde_json::to_vec_pretty(store)?;
    atomic_write(&path, &bytes).with_context(|| format!("write {}", path.display()))
}

pub fn apply_manual(base: &Dataset, store: &ManualStore) -> Result<Dataset> {
    validate_manual_store(store)?;
    let mut dataset = base.clone();
    for entry in store
        .entries
        .iter()
        .filter(|entry| entry.term_id == base.term_id && entry.enabled)
    {
        let calendar = dataset.calendar.clone();
        let course = dataset.courses.get_mut(&entry.course_id).with_context(|| {
            format!(
                "manual section {} references unknown subject {} in {}",
                entry.option.id, entry.course_id, entry.term_id
            )
        })?;
        let requirement_id = format!("{}/{}", entry.course_id, entry.kind);
        let requirement_index = match course
            .requirements
            .iter()
            .position(|requirement| requirement.kind == entry.kind)
        {
            Some(index) => index,
            None => {
                ensure!(
                    matches!(
                        entry.kind.as_str(),
                        "lecture" | "recitation" | "lab" | "design" | "pe"
                    ),
                    "manual section {} cannot create unknown component {}",
                    entry.option.id,
                    entry.kind
                );
                course.requirements.push(Requirement {
                    id: requirement_id,
                    kind: entry.kind.clone(),
                    options: Vec::new(),
                    has_unknown_times: true,
                });
                course.requirements.len() - 1
            }
        };

        let requirement = &mut course.requirements[requirement_index];
        ensure!(
            requirement.id == format!("{}/{}", entry.course_id, entry.kind),
            "manual section {} targets inconsistent component {}",
            entry.option.id,
            entry.kind
        );
        ensure!(
            !requirement
                .options
                .iter()
                .any(|option| option.id == entry.option.id),
            "duplicate section id after manual merge: {}",
            entry.option.id
        );
        if is_pe_kind(&entry.kind)
            && let Some(calendar) = calendar.as_ref()
        {
            ensure!(
                !meetings_have_internal_calendar_overlap(calendar, &entry.option.meetings),
                "manual section {} has overlapping PE meetings in the term calendar",
                entry.option.id
            );
        }
        requirement.options.push(entry.option.clone());
        requirement.options.sort_by(|a, b| a.id.cmp(&b.id));

        if is_supported_option(&entry.kind, &entry.option) {
            requirement.has_unknown_times = false;
            let unresolved_notice = format!("{}: no known meeting times", entry.kind);
            course.notices.retain(|notice| notice != &unresolved_notice);
        }
        if entry.option.unsupported_reason.is_some() {
            let notice = format!(
                "{}: date-limited manual options are preserved but omitted from weekly optimization",
                entry.kind
            );
            if !course.notices.contains(&notice) {
                course.notices.push(notice);
            }
        }
    }

    for course in dataset.courses.values_mut() {
        course.requirements.sort_by(|a, b| a.id.cmp(&b.id));
        course.notices.sort();
        course.notices.dedup();
    }
    Ok(dataset)
}

pub fn parse_meetings(text: &str) -> Result<Vec<Meeting>> {
    let mut meetings = Vec::new();
    for (index, part) in text.split(';').enumerate() {
        let part = part.trim();
        ensure!(!part.is_empty(), "meeting {} is empty", index + 1);
        let mut fields = part.split_whitespace();
        let day = fields
            .next()
            .with_context(|| format!("meeting {} is missing weekday", index + 1))?;
        let range = fields
            .next()
            .with_context(|| format!("meeting {} is missing time range", index + 1))?;
        ensure!(
            fields.next().is_none(),
            "meeting {} has extra text",
            index + 1
        );
        let (start, end) = range
            .split_once('-')
            .with_context(|| format!("meeting {} time range must be START-END", index + 1))?;
        let meeting = Meeting {
            weekday: parse_weekday(day)
                .with_context(|| format!("invalid weekday in meeting {}", index + 1))?,
            start_minute: parse_time(start)
                .with_context(|| format!("invalid start time in meeting {}", index + 1))?,
            end_minute: parse_time(end)
                .with_context(|| format!("invalid end time in meeting {}", index + 1))?,
            start_date: None,
            end_date: None,
        };
        meeting
            .validate()
            .with_context(|| format!("invalid time range in meeting {}", index + 1))?;
        meetings.push(meeting);
    }
    ensure!(!meetings.is_empty(), "at least one meeting is required");
    meetings.sort();
    meetings.dedup();
    Ok(meetings)
}

fn fetch_dataset() -> Result<(Dataset, CacheEnvelope)> {
    ensure_https(LATEST_CATALOG_URL)?;
    ensure_https(LATEST_TERM_URL)?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .user_agent("hydrant-optimizer/0.1")
        .timeout(Duration::from_secs(20))
        .build()
        .context("build HTTP client")?;
    let catalog = fetch_url(&client, LATEST_CATALOG_URL)?;
    let term = fetch_url(&client, LATEST_TERM_URL)?;
    let dataset =
        adapter::parse_catalog(&catalog, &term).context("downloaded catalog failed validation")?;
    let envelope = CacheEnvelope {
        version: CACHE_VERSION,
        fetched_at: Utc::now().to_rfc3339(),
        catalog_url: LATEST_CATALOG_URL.to_string(),
        term_url: LATEST_TERM_URL.to_string(),
        catalog,
        term,
    };
    Ok((dataset, envelope))
}

fn fetch_url(client: &reqwest::blocking::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    ensure!(status.is_success(), "GET {url} returned {status}");
    const MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;
    let mut bytes = Vec::new();
    response
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read response body from {url}"))?;
    ensure!(
        bytes.len() as u64 <= MAX_BODY_BYTES,
        "GET {url} exceeded the catalog size limit"
    );
    String::from_utf8(bytes).with_context(|| format!("response from {url} is not UTF-8"))
}

fn read_cache(dir: &Path) -> Result<CacheEnvelope> {
    let path = cache_path(dir);
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let envelope: CacheEnvelope =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    validate_cache_envelope(&envelope)?;
    Ok(envelope)
}

fn write_cache(dir: &Path, envelope: &CacheEnvelope) -> Result<()> {
    validate_cache_envelope(envelope)?;
    dataset_from_cache(envelope).context("refusing to cache invalid catalog")?;
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let bytes = serde_json::to_vec_pretty(envelope)?;
    atomic_write(&cache_path(dir), &bytes)
}

fn dataset_from_cache(envelope: &CacheEnvelope) -> Result<Dataset> {
    validate_cache_envelope(envelope)?;
    adapter::parse_catalog(&envelope.catalog, &envelope.term)
}

fn validate_cache_envelope(envelope: &CacheEnvelope) -> Result<()> {
    ensure!(
        envelope.version == CACHE_VERSION,
        "unsupported cache version {}",
        envelope.version
    );
    ensure_https(&envelope.catalog_url)?;
    ensure_https(&envelope.term_url)?;
    ensure!(
        envelope.catalog_url == LATEST_CATALOG_URL && envelope.term_url == LATEST_TERM_URL,
        "cache source URLs do not match the public Hydrant endpoints"
    );
    ensure!(
        !envelope.catalog.trim().is_empty(),
        "cached catalog is empty"
    );
    ensure!(
        !envelope.term.trim().is_empty(),
        "cached term metadata is empty"
    );
    DateTime::parse_from_rfc3339(&envelope.fetched_at)
        .context("cache fetched_at must be RFC3339")?;
    Ok(())
}

fn cache_fallback_notice(fetch_error: &anyhow::Error, envelope: &CacheEnvelope) -> String {
    let age = DateTime::parse_from_rfc3339(&envelope.fetched_at)
        .ok()
        .map(|time| {
            let duration = Utc::now().signed_duration_since(time.with_timezone(&Utc));
            if duration.num_days() != 0 {
                format!("{} days", duration.num_days())
            } else {
                format!("{} hours", duration.num_hours().max(0))
            }
        })
        .unwrap_or_else(|| "unknown age".to_string());
    format!(
        "online refresh failed: {fetch_error:#}; using cached catalog fetched {} (age {age})",
        envelope.fetched_at
    )
}

fn validate_manual_store(store: &ManualStore) -> Result<()> {
    ensure!(
        store.version == MANUAL_VERSION,
        "unsupported manual store version {}",
        store.version
    );
    let mut seen = BTreeSet::new();
    for entry in &store.entries {
        validate_manual_entry(entry)?;
        ensure!(
            seen.insert((entry.term_id.as_str(), entry.option.id.as_str())),
            "duplicate manual entry {} for {}/{} in {}",
            entry.option.id,
            entry.course_id,
            entry.kind,
            entry.term_id
        );
    }
    Ok(())
}

fn validate_manual_entry(entry: &ManualEntry) -> Result<()> {
    ensure!(
        !entry.term_id.trim().is_empty(),
        "manual entry term_id is empty"
    );
    ensure!(
        !entry.course_id.trim().is_empty(),
        "manual entry course_id is empty"
    );
    ensure!(
        !entry.kind.trim().is_empty() && entry.kind.trim().to_lowercase() == entry.kind,
        "manual entry kind must be nonempty, trimmed, and lowercase"
    );
    ensure!(
        entry.option.source == Source::Manual,
        "manual entry {} must have manual source",
        entry.option.id
    );
    ensure!(
        entry.option.id.starts_with("manual-")
            && entry.option.id.len() > "manual-".len()
            && !entry.option.id.chars().any(char::is_whitespace),
        "manual entry id must start with manual- and contain a nonempty identifier without whitespace"
    );
    ensure!(
        !entry.option.label.trim().is_empty(),
        "manual entry {} label is empty",
        entry.option.id
    );
    ensure!(
        !entry.option.meetings.is_empty(),
        "manual entry {} has no meetings",
        entry.option.id
    );
    ensure!(
        !entry.option.incompatible_with.contains(&entry.option.id),
        "manual entry cannot be incompatible with itself"
    );
    for meeting in &entry.option.meetings {
        meeting
            .validate()
            .with_context(|| format!("manual entry {} has invalid meeting", entry.option.id))?;
    }
    let mut sorted = entry.option.meetings.clone();
    sorted.sort();
    ensure!(
        sorted == entry.option.meetings,
        "manual entry {} meetings must be sorted",
        entry.option.id
    );
    let date_limited = meetings_have_date_limits(&entry.option.meetings);
    if is_pe_kind(&entry.kind) {
        ensure!(
            bounded_pe_meetings(&entry.kind, &entry.option.meetings),
            "manual entry {} with kind pe must have complete start/end date bounds",
            entry.option.id
        );
        ensure!(
            entry.option.unsupported_reason.is_none(),
            "manual entry {} with kind pe and complete bounds must not be marked unsupported",
            entry.option.id
        );
        ensure!(
            !meetings_have_internal_date_window_overlap(&entry.option.meetings),
            "manual entry {} has overlapping meetings",
            entry.option.id
        );
    } else if date_limited {
        ensure!(
            entry.option.unsupported_reason.is_some(),
            "manual entry {} with date bounds must be marked unsupported",
            entry.option.id
        );
    } else {
        ensure!(
            !meetings_have_internal_natural_overlap(&entry.option.meetings),
            "manual entry {} has overlapping meetings",
            entry.option.id
        );
    }
    Ok(())
}

fn is_supported_option(kind: &str, option: &SectionOption) -> bool {
    option.unsupported_reason.is_none()
        && !option.meetings.is_empty()
        && if is_pe_kind(kind) {
            bounded_pe_meetings(kind, &option.meetings)
        } else {
            !meetings_have_date_limits(&option.meetings)
        }
}

fn parse_weekday(text: &str) -> Result<u8> {
    match text.trim().to_ascii_lowercase().as_str() {
        "m" | "mon" | "monday" => Ok(0),
        "t" | "tu" | "tue" | "tues" | "tuesday" => Ok(1),
        "w" | "wed" | "weds" | "wednesday" => Ok(2),
        "r" | "th" | "thu" | "thur" | "thurs" | "thursday" => Ok(3),
        "f" | "fri" | "friday" => Ok(4),
        "sa" | "sat" | "saturday" => Ok(5),
        "su" | "sun" | "sunday" => Ok(6),
        _ => bail!("unknown weekday {text}"),
    }
}

fn parse_time(text: &str) -> Result<u16> {
    let (hour, minute) = text.trim().split_once(':').context("time must be HH:MM")?;
    ensure!(
        (1..=2).contains(&hour.len())
            && minute.len() == 2
            && hour
                .bytes()
                .chain(minute.bytes())
                .all(|byte| byte.is_ascii_digit()),
        "time must be H:MM or HH:MM"
    );
    let hour: u16 = hour.parse().context("hour must be numeric")?;
    let minute: u16 = minute.parse().context("minute must be numeric")?;
    ensure!(hour <= 24, "hour must be 0 through 24");
    ensure!(minute < 60, "minute must be 0 through 59");
    ensure!(hour < 24 || minute == 0, "24 hour is only valid as 24:00");
    Ok(hour * 60 + minute)
}

fn ensure_https(url: &str) -> Result<()> {
    ensure!(
        url.starts_with("https://"),
        "only public HTTPS URLs are allowed: {url}"
    );
    Ok(())
}

fn cache_path(dir: &Path) -> PathBuf {
    dir.join(CACHE_FILE)
}

fn manual_path(dir: &Path) -> PathBuf {
    dir.join(MANUAL_FILE)
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary file in {}", parent.display()))?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("persist {}: {}", path.display(), error.error))
}
