use std::{
    cmp::Ordering as CmpOrdering,
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, ensure};

use crate::model::{
    Meeting, Requirement, Score, SectionOption, Solution, SolveStatus, TermCalendar, TimeChoice,
    bounded_pe_meetings, calendar_meetings_overlap, is_pe_kind, meetings_have_date_limits,
    meetings_have_internal_calendar_overlap, meetings_have_internal_natural_overlap,
    natural_meetings_overlap,
};

const DAYS_PER_WEEK: usize = 7;
const MINUTES_PER_DAY: usize = 24 * 60;
const MINUTES_PER_WEEK: usize = DAYS_PER_WEEK * MINUTES_PER_DAY;
const WORD_BITS: usize = u64::BITS as usize;
const WEEK_WORDS: usize = MINUTES_PER_WEEK.div_ceil(WORD_BITS);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WeekBits([u64; WEEK_WORDS]);

impl WeekBits {
    fn empty() -> Self {
        Self([0; WEEK_WORDS])
    }

    fn from_meetings(meetings: &[Meeting]) -> Self {
        let mut bits = Self::empty();
        for meeting in meetings {
            let start = meeting.weekday as usize * MINUTES_PER_DAY + meeting.start_minute as usize;
            let end = meeting.weekday as usize * MINUTES_PER_DAY + meeting.end_minute as usize;
            for minute in start..end {
                bits.set(minute);
            }
        }
        bits
    }

    fn set(&mut self, minute: usize) {
        debug_assert!(minute < MINUTES_PER_WEEK);
        self.0[minute / WORD_BITS] |= 1_u64 << (minute % WORD_BITS);
    }

    fn contains(&self, minute: usize) -> bool {
        debug_assert!(minute < MINUTES_PER_WEEK);
        (self.0[minute / WORD_BITS] & (1_u64 << (minute % WORD_BITS))) != 0
    }

    fn intersects(&self, other: &Self) -> bool {
        self.0.iter().zip(other.0.iter()).any(|(a, b)| (a & b) != 0)
    }

    fn union(&self, other: &Self) -> Self {
        let mut out = Self::empty();
        for ((dst, a), b) in out.0.iter_mut().zip(self.0.iter()).zip(other.0.iter()) {
            *dst = *a | *b;
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    meetings: Vec<Meeting>,
    outbound: Vec<String>,
    inbound: Vec<String>,
}

#[derive(Clone, Debug)]
struct Candidate {
    choice: TimeChoice,
    bits: WeekBits,
}

#[derive(Clone, Debug)]
struct SearchGroup {
    requirement_id: String,
    kind: String,
    candidates: Vec<Candidate>,
}

#[derive(Clone, Debug)]
struct PreparedOption {
    option: SectionOption,
    meetings: Vec<Meeting>,
    searchable: bool,
}

#[derive(Clone, Debug)]
struct PreparedRequirement {
    id: String,
    kind: String,
    options: Vec<PreparedOption>,
}

/// Group one requirement by complete canonical time pattern.
///
/// `solve` performs stricter global grouping that also accounts for inbound
/// compatibility from other selected requirements. This standalone helper is
/// useful when no external pairing restrictions are in scope.
pub fn group(requirement: &Requirement) -> Result<Vec<TimeChoice>> {
    let mut buckets: BTreeMap<(Vec<Meeting>, Vec<String>), Vec<SectionOption>> = BTreeMap::new();
    for option in &requirement.options {
        let meetings = canonical_meetings(&option.meetings)?;
        if !option_is_searchable(
            &requirement.kind,
            &meetings,
            option.unsupported_reason.is_some(),
        ) {
            continue;
        }
        ensure_no_internal_overlap(&meetings, None)?;
        let outbound = option.incompatible_with.iter().cloned().collect::<Vec<_>>();
        buckets
            .entry((meetings, outbound))
            .or_default()
            .push(option.clone());
    }
    Ok(buckets
        .into_iter()
        .map(|((meetings, _), mut members)| make_choice(&requirement.id, meetings, &mut members))
        .collect())
}

pub fn solve(requirements: &[Requirement], cancel: Option<&AtomicBool>) -> Result<Solution> {
    solve_impl(requirements, None, cancel)
}

pub fn solve_with_calendar(
    requirements: &[Requirement],
    calendar: &TermCalendar,
    cancel: Option<&AtomicBool>,
) -> Result<Solution> {
    ensure!(calendar.start <= calendar.end, "calendar start follows end");
    ensure!(
        calendar.alternate_days.values().all(|weekday| *weekday < 7),
        "alternate calendar weekdays must be Monday through Sunday"
    );
    solve_impl(requirements, Some(calendar), cancel)
}

fn solve_impl(
    requirements: &[Requirement],
    calendar: Option<&TermCalendar>,
    cancel: Option<&AtomicBool>,
) -> Result<Solution> {
    if is_cancelled(cancel) {
        return Ok(cancelled_solution(Vec::new()));
    }

    let (mut groups, mut unresolved) = prepare(requirements, calendar)?;
    if is_cancelled(cancel) {
        return Ok(cancelled_solution(unresolved));
    }

    groups.sort_by(|a, b| {
        a.candidates
            .len()
            .cmp(&b.candidates.len())
            .then_with(|| a.requirement_id.cmp(&b.requirement_id))
    });

    if groups.is_empty() {
        return Ok(Solution {
            status: SolveStatus::OptimalKnown,
            choices: Vec::new(),
            score: Some(Score {
                occupied_days: 0,
                gap_minutes: 0,
            }),
            unresolved,
        });
    }

    let date_sensitive_conflicts = groups.iter().any(|group| {
        group
            .candidates
            .iter()
            .any(|candidate| meetings_have_date_limits(&candidate.choice.meetings))
    });

    let mut search = SearchState {
        groups: &groups,
        calendar,
        cancel,
        best_score: None,
        best_choices: Vec::new(),
        selected: Vec::new(),
        occupancy: WeekBits::empty(),
        date_sensitive_conflicts,
    };
    let cancelled = search.run(0);

    if cancelled || is_cancelled(cancel) {
        return Ok(cancelled_solution(unresolved));
    }

    let Some(score) = search.best_score else {
        return Ok(Solution {
            status: SolveStatus::Infeasible,
            choices: Vec::new(),
            score: None,
            unresolved,
        });
    };

    let mut choices = search.best_choices;
    choices.sort_by(|a, b| {
        a.requirement_id
            .cmp(&b.requirement_id)
            .then_with(|| a.id.cmp(&b.id))
    });
    if choices.iter().any(|choice| {
        groups
            .iter()
            .find(|group| group.requirement_id == choice.requirement_id)
            .is_some_and(|group| bounded_pe_meetings(&group.kind, &choice.meetings))
    }) {
        unresolved.push(
            "selected bounded PE meetings use a combined weekly template for score, and date bounds constrain conflicts and calendar export"
                .to_string(),
        );
        unresolved.sort();
        unresolved.dedup();
    }
    Ok(Solution {
        status: SolveStatus::OptimalKnown,
        choices,
        score: Some(score),
        unresolved,
    })
}

struct SearchState<'a> {
    groups: &'a [SearchGroup],
    calendar: Option<&'a TermCalendar>,
    cancel: Option<&'a AtomicBool>,
    best_score: Option<Score>,
    best_choices: Vec<TimeChoice>,
    selected: Vec<Candidate>,
    occupancy: WeekBits,
    date_sensitive_conflicts: bool,
}

impl SearchState<'_> {
    fn run(&mut self, group_index: usize) -> bool {
        if is_cancelled(self.cancel) {
            return true;
        }
        if group_index == self.groups.len() {
            let score = score_bits(&self.occupancy);
            if self
                .best_score
                .is_none_or(|best| compare_scores(score, best) == CmpOrdering::Less)
            {
                self.best_score = Some(score);
                self.best_choices = self
                    .selected
                    .iter()
                    .map(|candidate| candidate.choice.clone())
                    .collect();
            }
            return false;
        }

        for candidate in &self.groups[group_index].candidates {
            if is_cancelled(self.cancel) {
                return true;
            }
            if !self.date_sensitive_conflicts && self.occupancy.intersects(&candidate.bits) {
                continue;
            }
            if self.date_sensitive_conflicts
                && self
                    .selected
                    .iter()
                    .any(|selected| candidates_conflict(selected, candidate, self.calendar))
            {
                continue;
            }
            if self
                .selected
                .iter()
                .any(|selected| !choices_compatible(&selected.choice, &candidate.choice))
            {
                continue;
            }

            let old_occupancy = self.occupancy;
            self.occupancy = self.occupancy.union(&candidate.bits);
            self.selected.push(candidate.clone());
            let cancelled = self.run(group_index + 1);
            self.selected.pop();
            self.occupancy = old_occupancy;
            if cancelled {
                return true;
            }
        }
        false
    }
}

fn prepare(
    requirements: &[Requirement],
    calendar: Option<&TermCalendar>,
) -> Result<(Vec<SearchGroup>, Vec<String>)> {
    let mut seen_requirements = BTreeSet::new();
    let mut option_to_requirement = BTreeMap::new();
    let mut prepared = Vec::new();
    let mut notices = Vec::new();

    for requirement in requirements {
        ensure!(
            !requirement.id.trim().is_empty(),
            "requirement id must not be empty"
        );
        ensure!(
            seen_requirements.insert(requirement.id.clone()),
            "duplicate requirement id {}",
            requirement.id
        );

        if requirement.has_unknown_times {
            notices.push(format!(
                "{}: some options have unknown/TBA meeting times and are not optimized",
                requirement.id
            ));
        }

        let mut options = Vec::new();
        for option in &requirement.options {
            ensure!(!option.id.trim().is_empty(), "option id must not be empty");
            ensure!(
                option_to_requirement
                    .insert(option.id.clone(), requirement.id.clone())
                    .is_none(),
                "duplicate option id {}",
                option.id
            );

            let meetings = canonical_meetings(&option.meetings)?;
            if meetings.is_empty() {
                notices.push(format!(
                    "{}: option {} has unknown/TBA meeting times",
                    requirement.id, option.id
                ));
            }
            if let Some(reason) = &option.unsupported_reason {
                notices.push(format!(
                    "{}: option {} is unsupported: {}",
                    requirement.id, option.id, reason
                ));
            }
            let date_limited = meetings_have_date_limits(&meetings);
            let bounded_pe = bounded_pe_meetings(&requirement.kind, &meetings);
            if date_limited && !bounded_pe && !is_pe_kind(&requirement.kind) {
                notices.push(format!(
                    "{}: option {} has date-limited meetings; partial-term optimization is not supported",
                    requirement.id, option.id
                ));
            }
            if is_pe_kind(&requirement.kind)
                && !meetings.is_empty()
                && option.unsupported_reason.is_none()
                && !bounded_pe
            {
                notices.push(format!(
                    "{}: option {} is PE and requires complete valid start/end date bounds",
                    requirement.id, option.id
                ));
            }

            let searchable = option_is_searchable(
                &requirement.kind,
                &meetings,
                option.unsupported_reason.is_some(),
            );
            if searchable {
                ensure_no_internal_overlap(&meetings, calendar)?;
            }
            options.push(PreparedOption {
                option: option.clone(),
                meetings,
                searchable,
            });
        }
        prepared.push(PreparedRequirement {
            id: requirement.id.clone(),
            kind: requirement.kind.clone(),
            options,
        });
    }

    let searchable_ids = prepared
        .iter()
        .flat_map(|requirement| requirement.options.iter())
        .filter(|option| option.searchable)
        .map(|option| option.option.id.clone())
        .collect::<BTreeSet<_>>();

    let mut inbound: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for requirement in &prepared {
        for option in &requirement.options {
            if !option.searchable {
                continue;
            }
            for target in &option.option.incompatible_with {
                if searchable_ids.contains(target) {
                    inbound
                        .entry(target.clone())
                        .or_default()
                        .insert(option.option.id.clone());
                }
            }
        }
    }

    let mut groups = Vec::new();
    for requirement in prepared {
        let mut buckets: BTreeMap<GroupKey, Vec<SectionOption>> = BTreeMap::new();
        for option in requirement
            .options
            .into_iter()
            .filter(|option| option.searchable)
        {
            let outbound = option
                .option
                .incompatible_with
                .iter()
                .filter(|id| searchable_ids.contains(*id))
                .filter(|id| option_to_requirement.get(*id) != Some(&requirement.id))
                .cloned()
                .collect::<Vec<_>>();
            let inbound = inbound
                .get(&option.option.id)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter(|id| option_to_requirement.get(*id) != Some(&requirement.id))
                .cloned()
                .collect::<Vec<_>>();
            buckets
                .entry(GroupKey {
                    meetings: option.meetings,
                    outbound,
                    inbound,
                })
                .or_default()
                .push(option.option);
        }

        if buckets.is_empty() {
            notices.push(format!(
                "{}: no known supported meeting options; left unresolved",
                requirement.id
            ));
            continue;
        }

        let mut candidates = buckets
            .into_iter()
            .map(|(key, mut members)| {
                let choice = make_choice(&requirement.id, key.meetings, &mut members);
                Candidate {
                    bits: WeekBits::from_meetings(&choice.meetings),
                    choice,
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| {
            a.choice
                .id
                .cmp(&b.choice.id)
                .then_with(|| a.choice.meetings.cmp(&b.choice.meetings))
        });
        groups.push(SearchGroup {
            requirement_id: requirement.id,
            kind: requirement.kind,
            candidates,
        });
    }

    notices.sort();
    notices.dedup();
    Ok((groups, notices))
}

fn canonical_meetings(meetings: &[Meeting]) -> Result<Vec<Meeting>> {
    for meeting in meetings {
        meeting.validate()?;
    }
    let mut meetings = meetings.to_vec();
    meetings.sort();
    Ok(meetings)
}

fn option_is_searchable(kind: &str, meetings: &[Meeting], unsupported: bool) -> bool {
    !meetings.is_empty()
        && !unsupported
        && if is_pe_kind(kind) {
            bounded_pe_meetings(kind, meetings)
        } else {
            !meetings_have_date_limits(meetings)
        }
}

fn ensure_no_internal_overlap(meetings: &[Meeting], calendar: Option<&TermCalendar>) -> Result<()> {
    let overlap = if let Some(calendar) = calendar {
        meetings_have_internal_calendar_overlap(calendar, meetings)
    } else {
        meetings_have_internal_natural_overlap(meetings)
    };
    ensure!(!overlap, "meetings within one option overlap");
    Ok(())
}

fn candidates_conflict(
    left: &Candidate,
    right: &Candidate,
    calendar: Option<&TermCalendar>,
) -> bool {
    left.choice.meetings.iter().any(|left_meeting| {
        right
            .choice
            .meetings
            .iter()
            .any(|right_meeting| meetings_conflict(left_meeting, right_meeting, calendar))
    })
}

fn meetings_conflict(left: &Meeting, right: &Meeting, calendar: Option<&TermCalendar>) -> bool {
    if let Some(calendar) = calendar {
        calendar_meetings_overlap(calendar, left, right)
    } else {
        natural_meetings_overlap(left, right)
    }
}

fn make_choice(
    requirement_id: &str,
    meetings: Vec<Meeting>,
    members: &mut Vec<SectionOption>,
) -> TimeChoice {
    members.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.room.cmp(&b.room))
    });
    let id = members
        .first()
        .map(|member| member.id.clone())
        .unwrap_or_else(|| requirement_id.to_string());
    TimeChoice {
        id,
        requirement_id: requirement_id.to_string(),
        meetings,
        members: std::mem::take(members),
    }
}

fn choices_compatible(a: &TimeChoice, b: &TimeChoice) -> bool {
    a.members.iter().all(|left| {
        b.members.iter().all(|right| {
            !left.incompatible_with.contains(&right.id)
                && !right.incompatible_with.contains(&left.id)
        })
    })
}

fn compare_scores(a: Score, b: Score) -> CmpOrdering {
    a.occupied_days
        .cmp(&b.occupied_days)
        .then_with(|| a.gap_minutes.cmp(&b.gap_minutes))
}

fn score_bits(bits: &WeekBits) -> Score {
    let mut occupied_days = 0;
    let mut gap_minutes = 0;

    for day in 0..DAYS_PER_WEEK {
        let base = day * MINUTES_PER_DAY;
        let mut first = None;
        let mut last_exclusive = 0_u32;
        let mut occupied = 0_u32;

        for minute in 0..MINUTES_PER_DAY {
            if bits.contains(base + minute) {
                occupied += 1;
                if first.is_none() {
                    first = Some(minute as u32);
                }
                last_exclusive = minute as u32 + 1;
            }
        }

        if let Some(first) = first {
            occupied_days += 1;
            gap_minutes += last_exclusive - first - occupied;
        }
    }

    Score {
        occupied_days,
        gap_minutes,
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed))
}

fn cancelled_solution(unresolved: Vec<String>) -> Solution {
    Solution {
        status: SolveStatus::Cancelled,
        choices: Vec::new(),
        score: None,
        unresolved,
    }
}
