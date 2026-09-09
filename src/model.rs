use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dataset {
    pub term_id: String,
    pub last_updated: String,
    pub calendar: Option<TermCalendar>,
    pub courses: BTreeMap<String, Course>,
    #[serde(default)]
    pub notices: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TermCalendar {
    pub start: NaiveDate,
    pub end: NaiveDate,
    #[serde(default)]
    pub holidays: BTreeSet<NaiveDate>,
    /// Date -> weekday whose timetable is followed (Monday = 0).
    #[serde(default)]
    pub alternate_days: BTreeMap<NaiveDate, u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Course {
    pub id: String,
    pub title: String,
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub notices: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub kind: String,
    pub options: Vec<SectionOption>,
    pub has_unknown_times: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Api,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionOption {
    pub id: String,
    pub label: String,
    pub room: String,
    pub source: Source,
    pub meetings: Vec<Meeting>,
    #[serde(default)]
    pub incompatible_with: BTreeSet<String>,
    #[serde(default)]
    pub unsupported_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Meeting {
    pub weekday: u8,
    pub start_minute: u16,
    pub end_minute: u16,
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
}

impl Meeting {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.weekday < 7, "weekday must be Monday through Sunday");
        ensure!(
            self.start_minute < self.end_minute && self.end_minute <= 1440,
            "meeting must satisfy 00:00 <= start < end <= 24:00"
        );
        if let (Some(start), Some(end)) = (self.start_date, self.end_date) {
            ensure!(start <= end, "meeting start date follows end date");
        }
        Ok(())
    }

    pub fn display(&self) -> String {
        format!(
            "{} {:02}:{:02}-{:02}:{:02}",
            ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                .get(self.weekday as usize)
                .unwrap_or(&"?"),
            self.start_minute / 60,
            self.start_minute % 60,
            self.end_minute / 60,
            self.end_minute % 60
        )
    }
}

pub fn is_pe_kind(kind: &str) -> bool {
    !kind.trim().is_empty() && kind.trim().eq_ignore_ascii_case("pe")
}

pub fn meeting_has_date_limit(meeting: &Meeting) -> bool {
    meeting.start_date.is_some() || meeting.end_date.is_some()
}

pub fn meetings_have_date_limits(meetings: &[Meeting]) -> bool {
    meetings.iter().any(meeting_has_date_limit)
}

pub fn meeting_has_complete_date_bounds(meeting: &Meeting) -> bool {
    matches!(
        (meeting.start_date, meeting.end_date),
        (Some(start), Some(end)) if start <= end
    )
}

pub fn bounded_pe_meetings(kind: &str, meetings: &[Meeting]) -> bool {
    is_pe_kind(kind)
        && !meetings.is_empty()
        && meetings
            .iter()
            .all(|meeting| meeting_has_complete_date_bounds(meeting) && meeting.validate().is_ok())
}

pub fn natural_meetings_overlap(left: &Meeting, right: &Meeting) -> bool {
    left.weekday == right.weekday
        && left.start_minute.max(right.start_minute) < left.end_minute.min(right.end_minute)
        && natural_date_windows_overlap(left, right)
}

pub fn meetings_have_internal_natural_overlap(meetings: &[Meeting]) -> bool {
    meetings.iter().enumerate().any(|(index, left)| {
        meetings
            .iter()
            .skip(index + 1)
            .any(|right| natural_meetings_overlap(left, right))
    })
}

pub fn calendar_meetings_overlap(calendar: &TermCalendar, left: &Meeting, right: &Meeting) -> bool {
    if left.weekday != right.weekday
        || left.start_minute.max(right.start_minute) >= left.end_minute.min(right.end_minute)
    {
        return false;
    }
    let Some((left_start, left_end)) = clipped_meeting_window(calendar, left) else {
        return false;
    };
    let Some((right_start, right_end)) = clipped_meeting_window(calendar, right) else {
        return false;
    };
    let mut date = left_start.max(right_start);
    let end = left_end.min(right_end);
    while date <= end {
        if !calendar.holidays.contains(&date) && effective_weekday(calendar, date) == left.weekday {
            return true;
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    false
}

pub fn meetings_have_internal_calendar_overlap(
    calendar: &TermCalendar,
    meetings: &[Meeting],
) -> bool {
    meetings.iter().enumerate().any(|(index, left)| {
        meetings
            .iter()
            .skip(index + 1)
            .any(|right| calendar_meetings_overlap(calendar, left, right))
    })
}

pub fn date_window_meetings_overlap(left: &Meeting, right: &Meeting) -> bool {
    left.weekday == right.weekday
        && left.start_minute.max(right.start_minute) < left.end_minute.min(right.end_minute)
        && date_windows_overlap(left, right)
}

pub fn meetings_have_internal_date_window_overlap(meetings: &[Meeting]) -> bool {
    meetings.iter().enumerate().any(|(index, left)| {
        meetings
            .iter()
            .skip(index + 1)
            .any(|right| date_window_meetings_overlap(left, right))
    })
}

fn clipped_meeting_window(
    calendar: &TermCalendar,
    meeting: &Meeting,
) -> Option<(NaiveDate, NaiveDate)> {
    let start = meeting
        .start_date
        .unwrap_or(calendar.start)
        .max(calendar.start);
    let end = meeting.end_date.unwrap_or(calendar.end).min(calendar.end);
    (start <= end).then_some((start, end))
}

fn effective_weekday(calendar: &TermCalendar, date: NaiveDate) -> u8 {
    calendar
        .alternate_days
        .get(&date)
        .copied()
        .unwrap_or_else(|| date.weekday().num_days_from_monday() as u8)
}

fn natural_date_windows_overlap(left: &Meeting, right: &Meeting) -> bool {
    match (
        left.start_date,
        left.end_date,
        right.start_date,
        right.end_date,
    ) {
        (Some(left_start), Some(left_end), Some(right_start), Some(right_end)) => {
            natural_weekday_occurs(
                left.weekday,
                left_start.max(right_start),
                left_end.min(right_end),
            )
        }
        (Some(start), Some(end), None, None) | (None, None, Some(start), Some(end)) => {
            natural_weekday_occurs(left.weekday, start, end)
        }
        _ => true,
    }
}

fn date_windows_overlap(left: &Meeting, right: &Meeting) -> bool {
    match (
        left.start_date,
        left.end_date,
        right.start_date,
        right.end_date,
    ) {
        (Some(left_start), Some(left_end), Some(right_start), Some(right_end)) => {
            left_start <= right_end && right_start <= left_end
        }
        _ => true,
    }
}

fn natural_weekday_occurs(weekday: u8, mut date: NaiveDate, end: NaiveDate) -> bool {
    while date <= end {
        if date.weekday().num_days_from_monday() as u8 == weekday {
            return true;
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    false
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeChoice {
    pub id: String,
    pub requirement_id: String,
    pub meetings: Vec<Meeting>,
    pub members: Vec<SectionOption>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Score {
    pub occupied_days: u8,
    pub gap_minutes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    OptimalKnown,
    Infeasible,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Solution {
    pub status: SolveStatus,
    pub choices: Vec<TimeChoice>,
    pub score: Option<Score>,
    pub unresolved: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualEntry {
    pub term_id: String,
    pub course_id: String,
    pub kind: String,
    pub option: SectionOption,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualStore {
    pub version: u32,
    pub entries: Vec<ManualEntry>,
}

impl Default for ManualStore {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

/// Actual member selected after optimization. Switching it must preserve TimeChoice.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChosenSection {
    pub course_id: String,
    pub course_title: String,
    pub kind: String,
    pub section: SectionOption,
}
