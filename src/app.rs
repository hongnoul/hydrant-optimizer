//! Application operations shared by the CLI and terminal UI.
use std::{collections::{BTreeMap, BTreeSet}, io::Write, path::Path, sync::atomic::AtomicBool};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

use crate::{calendar::{self, ExportReport}, model::*, optimizer};

pub fn optimize(dataset: &Dataset, selected: &[String], cancel: Option<&AtomicBool>) -> Result<Solution> {
    let mut requirements = Vec::new();
    let mut notices = dataset.notices.clone();
    for id in selected.iter().collect::<BTreeSet<_>>() {
        let course = dataset.courses.get(id).with_context(|| format!("unknown subject {id}"))?;
        requirements.extend(course.requirements.clone());
        notices.extend(course.notices.iter().map(|n| format!("{id}: {n}")));
        if course.requirements.is_empty() { notices.push(format!("{id}: no known meeting components")); }
    }
    let mut solution = optimizer::solve(&requirements, cancel)?;
    solution.unresolved.extend(notices);
    solution.unresolved.sort();
    solution.unresolved.dedup();
    Ok(solution)
}

pub fn actual_sections(dataset: &Dataset, solution: &Solution, members: &BTreeMap<String, String>) -> Result<Vec<ChosenSection>> {
    ensure!(solution.status == SolveStatus::OptimalKnown, "only a completed feasible optimization has selected sections");
    for requirement in members.keys() {
        ensure!(solution.choices.iter().any(|c| &c.requirement_id == requirement), "no chosen requirement {requirement}");
    }
    solution.choices.iter().map(|choice| {
        let section = if let Some(id) = members.get(&choice.requirement_id) {
            choice.members.iter().find(|s| &s.id == id).with_context(|| format!("{id} is not a same-time member of {}", choice.requirement_id))?
        } else { choice.members.first().context("optimizer returned an empty member group")? };
        let (course, requirement) = dataset.courses.values().find_map(|course| {
            course.requirements.iter().find(|r| r.id == choice.requirement_id).map(|r| (course, r))
        }).with_context(|| format!("unknown chosen requirement {}", choice.requirement_id))?;
        Ok(ChosenSection { course_id: course.id.clone(), course_title: course.title.clone(), kind: requirement.kind.clone(), section: section.clone() })
    }).collect()
}

pub fn write_calendar(dataset: &Dataset, solution: &Solution, members: &BTreeMap<String, String>, path: &Path) -> Result<ExportReport> {
    let chosen = actual_sections(dataset, solution, members)?;
    let term = dataset.calendar.as_ref().context("term calendar is unavailable; cannot export without reliable semester dates")?;
    let mut report = calendar::export_ics(term, &chosen)?;
    report.notices.extend(solution.unresolved.clone());
    report.notices.sort();
    report.notices.dedup();
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).with_context(|| format!("cannot create output in {}", parent.display()))?;
    temp.write_all(report.ics.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path).map_err(|e| anyhow::anyhow!("cannot create {} (existing files are never overwritten): {}", path.display(), e.error))?;
    Ok(report)
}

pub fn manual_entry(base: &Dataset, course_id: &str, kind: &str, label: &str, room: &str, mut meetings: Vec<Meeting>, id: Option<&str>) -> Result<ManualEntry> {
    ensure!(base.courses.contains_key(course_id), "unknown subject {course_id}");
    ensure!(["lecture", "recitation", "lab", "design"].contains(&kind), "component must be lecture, recitation, lab, or design");
    ensure!(!label.trim().is_empty(), "manual section needs a label");
    ensure!(!meetings.is_empty(), "manual section needs at least one meeting");
    for meeting in &meetings { meeting.validate()?; }
    meetings.sort();
    meetings.dedup();
    ensure!(!meetings.windows(2).any(|ms| ms[0].weekday == ms[1].weekday && ms[0].end_minute > ms[1].start_minute), "meetings within one manual section overlap");
    let key = serde_json::to_vec(&(&base.term_id, course_id, kind, label.trim(), room.trim(), &meetings))?;
    let id = match id {
        Some(id) => { ensure!(id.starts_with("manual-"), "not a manual section ID"); id.to_owned() }
        None => format!("manual-{:x}", Sha256::digest(&key)),
    };
    let unsupported_reason = meetings.iter().any(|m| m.start_date.is_some() || m.end_date.is_some())
        .then(|| "date-limited manual section: partial-term optimization is not supported".to_string());
    Ok(ManualEntry {
        term_id: base.term_id.clone(), course_id: course_id.to_owned(), kind: kind.to_owned(), enabled: true,
        option: SectionOption { id, label: label.trim().to_owned(), room: room.trim().to_owned(), source: Source::Manual,
            meetings, incompatible_with: BTreeSet::new(), unsupported_reason },
    })
}
