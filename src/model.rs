use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use chrono::NaiveDate;
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
pub enum Source { Api, Manual }

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
        ensure!(self.start_minute < self.end_minute && self.end_minute <= 1440,
            "meeting must satisfy 00:00 <= start < end <= 24:00");
        if let (Some(start), Some(end)) = (self.start_date, self.end_date) {
            ensure!(start <= end, "meeting start date follows end date");
        }
        Ok(())
    }

    pub fn display(&self) -> String {
        format!("{} {:02}:{:02}-{:02}:{:02}",
            ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].get(self.weekday as usize).unwrap_or(&"?"),
            self.start_minute / 60, self.start_minute % 60,
            self.end_minute / 60, self.end_minute % 60)
    }
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
pub enum SolveStatus { OptimalKnown, Infeasible, Cancelled }

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
    fn default() -> Self { Self { version: 1, entries: Vec::new() } }
}

/// Actual member selected after optimization. Switching it must preserve TimeChoice.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChosenSection {
    pub course_id: String,
    pub course_title: String,
    pub kind: String,
    pub section: SectionOption,
}
