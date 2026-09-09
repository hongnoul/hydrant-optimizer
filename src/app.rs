//! Application operations shared by the CLI and terminal UI.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

use crate::{
    calendar::{self, ExportReport},
    model::*,
    optimizer,
};

pub fn optimize(
    dataset: &Dataset,
    selected: &[String],
    cancel: Option<&AtomicBool>,
) -> Result<Solution> {
    let mut requirements = Vec::new();
    let mut notices = dataset.notices.clone();
    for id in selected.iter().collect::<BTreeSet<_>>() {
        let course = dataset
            .courses
            .get(id)
            .with_context(|| format!("unknown subject {id}"))?;
        requirements.extend(course.requirements.clone());
        notices.extend(course.notices.iter().map(|n| format!("{id}: {n}")));
        if course.requirements.is_empty() {
            notices.push(format!("{id}: no known meeting components"));
        }
    }
    let mut solution = if let Some(calendar) = dataset.calendar.as_ref() {
        optimizer::solve_with_calendar(&requirements, calendar, cancel)?
    } else {
        optimizer::solve(&requirements, cancel)?
    };
    solution.unresolved.extend(notices);
    solution.unresolved.sort();
    solution.unresolved.dedup();
    Ok(solution)
}

pub fn actual_sections(
    dataset: &Dataset,
    solution: &Solution,
    members: &BTreeMap<String, String>,
) -> Result<Vec<ChosenSection>> {
    ensure!(
        solution.status == SolveStatus::OptimalKnown,
        "only a completed feasible optimization has selected sections"
    );
    for requirement in members.keys() {
        ensure!(
            solution
                .choices
                .iter()
                .any(|c| &c.requirement_id == requirement),
            "no chosen requirement {requirement}"
        );
    }
    let chosen: Vec<ChosenSection> = solution
        .choices
        .iter()
        .map(|choice| {
            let section = if let Some(id) = members.get(&choice.requirement_id) {
                choice
                    .members
                    .iter()
                    .find(|s| &s.id == id)
                    .with_context(|| {
                        format!(
                            "{id} is not a same-time member of {}",
                            choice.requirement_id
                        )
                    })?
            } else {
                choice
                    .members
                    .first()
                    .context("optimizer returned an empty member group")?
            };
            let (course, requirement) = dataset
                .courses
                .values()
                .find_map(|course| {
                    course
                        .requirements
                        .iter()
                        .find(|r| r.id == choice.requirement_id)
                        .map(|r| (course, r))
                })
                .with_context(|| format!("unknown chosen requirement {}", choice.requirement_id))?;
            Ok(ChosenSection {
                course_id: course.id.clone(),
                course_title: course.title.clone(),
                kind: requirement.kind.clone(),
                section: section.clone(),
            })
        })
        .collect::<Result<_>>()?;
    for (index, left) in chosen.iter().enumerate() {
        for right in &chosen[index + 1..] {
            ensure!(
                !left.section.incompatible_with.contains(&right.section.id)
                    && !right.section.incompatible_with.contains(&left.section.id),
                "incompatible actual sections: {}/{} {} and {}/{} {}",
                left.course_id,
                left.kind,
                left.section.id,
                right.course_id,
                right.kind,
                right.section.id
            );
        }
    }
    Ok(chosen)
}

pub fn timestamped_export_path(hint: &Path, stamp: u64) -> PathBuf {
    timestamped_export_path_with_suffix(hint, &stamp.to_string())
}

fn timestamped_export_path_with_suffix(hint: &Path, suffix: &str) -> PathBuf {
    let parent = hint
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = hint
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("schedule");
    let extension = hint
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty())
        .unwrap_or("ics");
    parent.join(format!("{stem}-{suffix}.{extension}"))
}

pub fn write_calendar(
    dataset: &Dataset,
    solution: &Solution,
    members: &BTreeMap<String, String>,
    hint: &Path,
) -> Result<ExportReport> {
    let chosen = actual_sections(dataset, solution, members)?;
    let term = dataset
        .calendar
        .as_ref()
        .context("term calendar is unavailable; cannot export without reliable semester dates")?;
    let mut report = calendar::export_ics(term, &chosen)?;
    report.notices.extend(solution.unresolved.clone());
    report.notices.sort();
    report.notices.dedup();
    let parent = hint
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Every export gets a Unix-time signature so repeats never overwrite.
    // A same-second retry appends a counter rather than reusing a name.
    let mut stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut attempt = 0u32;
    loop {
        let suffix = if attempt == 0 {
            stamp.to_string()
        } else {
            format!("{stamp}-{attempt}")
        };
        let path = timestamped_export_path_with_suffix(hint, &suffix);
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("cannot create output in {}", parent.display()))?;
        temp.write_all(report.ics.as_bytes())?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(_) => {
                report.path = path;
                return Ok(report);
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                // Same-second collision: keep the same stamp on the first
                // retry so tests can predict it, then advance the clock.
                if attempt == 0 {
                    attempt = 1;
                } else {
                    stamp = stamp.saturating_add(1);
                    attempt = 0;
                }
                continue;
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "cannot create {}: {}",
                    path.display(),
                    error.error
                ));
            }
        }
    }
}

pub fn manual_entry(
    base: &Dataset,
    course_id: &str,
    kind: &str,
    label: &str,
    room: &str,
    mut meetings: Vec<Meeting>,
    id: Option<&str>,
) -> Result<ManualEntry> {
    let course = base
        .courses
        .get(course_id)
        .with_context(|| format!("unknown subject {course_id}"))?;
    let kind = kind.trim().to_lowercase();
    ensure!(
        ["lecture", "recitation", "lab", "design"].contains(&kind.as_str())
            || kind == "pe"
            || course.requirements.iter().any(|r| r.kind == kind),
        "component must be published for this subject, or lecture, recitation, lab, design, or pe"
    );
    ensure!(!label.trim().is_empty(), "manual section needs a label");
    ensure!(
        !meetings.is_empty(),
        "manual section needs at least one meeting"
    );
    for meeting in &meetings {
        meeting.validate()?;
    }
    meetings.sort();
    meetings.dedup();
    let date_limited = meetings_have_date_limits(&meetings);
    if is_pe_kind(&kind) {
        ensure!(
            bounded_pe_meetings(&kind, &meetings),
            "PE manual section needs complete start/end date bounds"
        );
        let internal_overlap = meetings_have_internal_date_window_overlap(&meetings)
            || base.calendar.as_ref().is_some_and(|calendar| {
                meetings_have_internal_calendar_overlap(calendar, &meetings)
            });
        ensure!(
            !internal_overlap,
            "meetings within one manual section overlap"
        );
    } else {
        ensure!(
            date_limited || !meetings_have_internal_natural_overlap(&meetings),
            "meetings within one manual section overlap"
        );
    }
    let key = serde_json::to_vec(&(
        &base.term_id,
        course_id,
        &kind,
        label.trim(),
        room.trim(),
        &meetings,
    ))?;
    let id = match id {
        Some(id) => {
            ensure!(id.starts_with("manual-"), "not a manual section ID");
            id.to_owned()
        }
        None => format!("manual-{:x}", Sha256::digest(&key)),
    };
    let unsupported_reason = (!is_pe_kind(&kind) && date_limited).then(|| {
        "date-limited manual section: partial-term optimization is not supported".to_string()
    });
    Ok(ManualEntry {
        term_id: base.term_id.clone(),
        course_id: course_id.to_owned(),
        kind: kind.to_owned(),
        enabled: true,
        option: SectionOption {
            id,
            label: label.trim().to_owned(),
            room: room.trim().to_owned(),
            source: Source::Manual,
            meetings,
            incompatible_with: BTreeSet::new(),
            unsupported_reason,
        },
    })
}
