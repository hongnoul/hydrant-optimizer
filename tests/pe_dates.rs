mod support;
use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, NaiveDate};
use hydrant_optimizer::{app, calendar, model::*, optimizer, storage};
use tempfile::TempDir;

const MON: u8 = 0;
const TUE: u8 = 1;

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn term_calendar() -> TermCalendar {
    TermCalendar {
        start: date("2026-10-26"),
        end: date("2026-11-09"),
        holidays: BTreeSet::from([date("2026-11-02")]),
        alternate_days: BTreeMap::from([(date("2026-11-03"), MON)]),
    }
}

fn meeting(weekday: u8, start: u16, end: u16) -> Meeting {
    Meeting {
        weekday,
        start_minute: start,
        end_minute: end,
        start_date: None,
        end_date: None,
    }
}

fn pe_meeting(weekday: u8, start: u16, end: u16, start_date: &str, end_date: &str) -> Meeting {
    Meeting {
        weekday,
        start_minute: start,
        end_minute: end,
        start_date: Some(date(start_date)),
        end_date: Some(date(end_date)),
    }
}

fn option(id: &str, meetings: Vec<Meeting>) -> SectionOption {
    SectionOption {
        id: id.to_string(),
        label: id.to_string(),
        room: format!("room-{id}"),
        source: Source::Api,
        meetings,
        incompatible_with: BTreeSet::new(),
        unsupported_reason: None,
    }
}

fn manual_option(id: &str, meetings: Vec<Meeting>) -> SectionOption {
    SectionOption {
        source: Source::Manual,
        ..option(id, meetings)
    }
}

fn requirement(id: &str, kind: &str, options: Vec<SectionOption>) -> Requirement {
    Requirement {
        id: id.to_string(),
        kind: kind.to_string(),
        options,
        has_unknown_times: false,
    }
}

fn course(id: &str, title: &str, requirements: Vec<Requirement>) -> Course {
    Course {
        id: id.to_string(),
        title: title.to_string(),
        requirements,
        notices: Vec::new(),
    }
}

fn dataset(calendar: Option<TermCalendar>, courses: Vec<Course>) -> Dataset {
    Dataset {
        term_id: "f26".to_string(),
        last_updated: "test".to_string(),
        calendar,
        courses: courses
            .into_iter()
            .map(|course| (course.id.clone(), course))
            .collect(),
        notices: Vec::new(),
    }
}

fn choice_for<'a>(solution: &'a Solution, requirement_id: &str) -> &'a TimeChoice {
    solution
        .choices
        .iter()
        .find(|choice| choice.requirement_id == requirement_id)
        .unwrap_or_else(|| panic!("missing choice for {requirement_id}"))
}

fn oracle_dates(calendar: &TermCalendar, meeting: &Meeting) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    let mut date = meeting
        .start_date
        .unwrap_or(calendar.start)
        .max(calendar.start);
    let end = meeting.end_date.unwrap_or(calendar.end).min(calendar.end);
    while date <= end {
        let effective_weekday = calendar
            .alternate_days
            .get(&date)
            .copied()
            .unwrap_or_else(|| date.weekday().num_days_from_monday() as u8);
        if !calendar.holidays.contains(&date) && effective_weekday == meeting.weekday {
            dates.push(date);
        }
        date = date.succ_opt().unwrap();
    }
    dates
}

fn oracle_conflict(calendar: &TermCalendar, left: &Meeting, right: &Meeting) -> bool {
    left.weekday == right.weekday
        && left.start_minute.max(right.start_minute) < left.end_minute.min(right.end_minute)
        && oracle_dates(calendar, left)
            .iter()
            .any(|date| oracle_dates(calendar, right).contains(date))
}

fn chosen_pe(option: SectionOption) -> ChosenSection {
    ChosenSection {
        course_id: "PE.0413".to_string(),
        course_title: "Backpacking".to_string(),
        kind: "pe".to_string(),
        section: option,
    }
}

#[derive(Clone, Debug)]
struct OracleCandidate {
    requirement_id: String,
    option_id: String,
    meetings: Vec<Meeting>,
}

fn independent_cartesian_best(
    requirements: &[Requirement],
    calendar: &TermCalendar,
) -> Option<(Score, BTreeMap<String, String>)> {
    let mut domains = Vec::new();
    for requirement in requirements {
        let mut choices = Vec::new();
        for option in &requirement.options {
            if option.unsupported_reason.is_some() || option.meetings.is_empty() {
                continue;
            }
            let valid = if requirement.kind == "pe" {
                option.meetings.iter().all(|meeting| {
                    meeting.validate().is_ok()
                        && meeting.start_date.is_some()
                        && meeting.end_date.is_some()
                })
            } else {
                option.meetings.iter().all(|meeting| {
                    meeting.validate().is_ok()
                        && meeting.start_date.is_none()
                        && meeting.end_date.is_none()
                })
            };
            if !valid {
                continue;
            }
            if option.meetings.iter().enumerate().any(|(index, left)| {
                option
                    .meetings
                    .iter()
                    .skip(index + 1)
                    .any(|right| oracle_conflict(calendar, left, right))
            }) {
                continue;
            }
            choices.push(OracleCandidate {
                requirement_id: requirement.id.clone(),
                option_id: option.id.clone(),
                meetings: option.meetings.clone(),
            });
        }
        choices.sort_by(|left, right| left.option_id.cmp(&right.option_id));
        if choices.is_empty() {
            return None;
        }
        domains.push(choices);
    }

    fn visit(
        domains: &[Vec<OracleCandidate>],
        index: usize,
        selected: &mut Vec<OracleCandidate>,
        best: &mut Option<(Score, BTreeMap<String, String>)>,
        calendar: &TermCalendar,
    ) {
        if index == domains.len() {
            let score = independent_union_week_score(
                selected
                    .iter()
                    .flat_map(|candidate| candidate.meetings.iter()),
            );
            let ids = selected
                .iter()
                .map(|candidate| {
                    (
                        candidate.requirement_id.clone(),
                        candidate.option_id.clone(),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            if best.as_ref().is_none_or(|(old_score, _)| {
                (score.occupied_days, score.gap_minutes)
                    < (old_score.occupied_days, old_score.gap_minutes)
            }) {
                *best = Some((score, ids));
            }
            return;
        }
        for candidate in &domains[index] {
            if selected.iter().all(|other| {
                !candidate.meetings.iter().any(|left| {
                    other
                        .meetings
                        .iter()
                        .any(|right| oracle_conflict(calendar, left, right))
                })
            }) {
                selected.push(candidate.clone());
                visit(domains, index + 1, selected, best, calendar);
                selected.pop();
            }
        }
    }

    let mut selected = Vec::new();
    let mut best = None;
    visit(&domains, 0, &mut selected, &mut best, calendar);
    best
}

fn independent_union_week_score<'a>(meetings: impl IntoIterator<Item = &'a Meeting>) -> Score {
    let mut occupied_minutes = [[false; 24 * 60]; 7];
    for meeting in meetings {
        for minute in meeting.start_minute..meeting.end_minute {
            occupied_minutes[meeting.weekday as usize][minute as usize] = true;
        }
    }
    let mut occupied_days = 0;
    let mut gap_minutes = 0;
    for day in occupied_minutes {
        let first = day.iter().position(|occupied| *occupied);
        let last = day.iter().rposition(|occupied| *occupied);
        if let (Some(first), Some(last)) = (first, last) {
            occupied_days += 1;
            let occupied = day.iter().filter(|occupied| **occupied).count() as u32;
            gap_minutes += (last - first + 1) as u32 - occupied;
        }
    }
    Score {
        occupied_days,
        gap_minutes,
    }
}

#[test]
fn independent_cartesian_oracle_matches_mixed_academic_pe_calendar_solution() {
    let calendar = term_calendar();
    let requirements = vec![
        requirement(
            "18.01/lecture",
            "lecture",
            vec![option("18.01.A", vec![meeting(MON, 8 * 60, 9 * 60)])],
        ),
        requirement(
            "PE.0413.Q1/pe",
            "pe",
            vec![option(
                "PE.0413.Q1",
                vec![pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30")],
            )],
        ),
        requirement(
            "PE.0413.Q2/pe",
            "pe",
            vec![
                option(
                    "PE.0413.Q1.OVERLAP",
                    vec![pe_meeting(
                        MON,
                        9 * 60 + 30,
                        10 * 60 + 30,
                        "2026-10-26",
                        "2026-10-30",
                    )],
                ),
                option(
                    "PE.0413.Q2",
                    vec![pe_meeting(
                        MON,
                        9 * 60 + 30,
                        10 * 60 + 30,
                        "2026-11-03",
                        "2026-11-09",
                    )],
                ),
            ],
        ),
        requirement(
            "21M.000/lecture",
            "lecture",
            vec![
                option(
                    "21M.000.CONFLICT",
                    vec![meeting(MON, 9 * 60 + 45, 10 * 60 + 15)],
                ),
                option("21M.000.SAFE", vec![meeting(MON, 10 * 60 + 30, 11 * 60)]),
            ],
        ),
    ];

    let (expected_score, expected_ids) = independent_cartesian_best(&requirements, &calendar)
        .expect("the independent oracle should find a feasible assignment");
    let solution = optimizer::solve_with_calendar(&requirements, &calendar, None).unwrap();
    let actual_ids = solution
        .choices
        .iter()
        .map(|choice| (choice.requirement_id.clone(), choice.id.clone()))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert_eq!(solution.score, Some(expected_score));
    assert_eq!(actual_ids, expected_ids);
    assert_eq!(actual_ids["PE.0413.Q2/pe"], "PE.0413.Q2");
    assert_eq!(actual_ids["21M.000/lecture"], "21M.000.SAFE");
}

#[test]
fn disjoint_pe_quarters_can_share_weekly_time_with_union_template_notice() {
    let calendar = term_calendar();
    let q1 = pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30");
    let q2 = pe_meeting(MON, 9 * 60, 10 * 60, "2026-11-04", "2026-11-09");
    assert!(!oracle_conflict(&calendar, &q1, &q2));

    let requirements = vec![
        requirement("PE.0413.Q1/pe", "pe", vec![option("PE.0413.Q1", vec![q1])]),
        requirement("PE.0413.Q2/pe", "pe", vec![option("PE.0413.Q2", vec![q2])]),
    ];
    let solution = optimizer::solve_with_calendar(&requirements, &calendar, None).unwrap();
    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert_eq!(solution.choices.len(), 2);
    assert_eq!(
        solution.score,
        Some(Score {
            occupied_days: 1,
            gap_minutes: 0,
        })
    );
    assert!(
        solution
            .unresolved
            .iter()
            .any(|notice| notice.contains("combined weekly template"))
    );

    let natural_solution = optimizer::solve(&requirements, None).unwrap();
    assert_eq!(natural_solution.status, SolveStatus::OptimalKnown);
    assert_eq!(natural_solution.choices.len(), 2);
}

#[test]
fn overlapping_pe_dates_conflict_but_exact_minute_adjacency_is_allowed() {
    let calendar = term_calendar();
    let left = pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-11-06");
    let right = pe_meeting(MON, 9 * 60 + 30, 10 * 60 + 30, "2026-10-30", "2026-11-09");
    assert!(oracle_conflict(&calendar, &left, &right));
    let conflict = vec![
        requirement("PE.LEFT/pe", "pe", vec![option("PE.LEFT.Q1", vec![left])]),
        requirement(
            "PE.RIGHT/pe",
            "pe",
            vec![option("PE.RIGHT.Q1", vec![right])],
        ),
    ];
    assert_eq!(
        optimizer::solve_with_calendar(&conflict, &calendar, None)
            .unwrap()
            .status,
        SolveStatus::Infeasible
    );

    let adjacent = vec![
        requirement(
            "PE.LEFT/pe",
            "pe",
            vec![option(
                "PE.LEFT.Q1",
                vec![pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-11-06")],
            )],
        ),
        requirement(
            "PE.RIGHT/pe",
            "pe",
            vec![option(
                "PE.RIGHT.Q1",
                vec![pe_meeting(
                    MON,
                    10 * 60,
                    11 * 60,
                    "2026-10-30",
                    "2026-11-09",
                )],
            )],
        ),
    ];
    let solution = optimizer::solve_with_calendar(&adjacent, &calendar, None).unwrap();
    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert_eq!(solution.choices.len(), 2);
}

#[test]
fn grouping_is_date_sensitive_only_for_bounded_pe() {
    let pe = requirement(
        "PE.0413/pe",
        "pe",
        vec![
            option(
                "PE.0413.Q1",
                vec![pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30")],
            ),
            option(
                "PE.0413.Q2",
                vec![pe_meeting(MON, 9 * 60, 10 * 60, "2026-11-04", "2026-11-09")],
            ),
        ],
    );
    let groups = optimizer::group(&pe).unwrap();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().all(|group| group.members.len() == 1));

    let non_pe = requirement(
        "P/lecture",
        "lecture",
        vec![option(
            "partial",
            vec![pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30")],
        )],
    );
    assert!(optimizer::group(&non_pe).unwrap().is_empty());
}

#[test]
fn app_uses_calendar_to_reject_academic_pe_conflicts_and_select_safe_option() {
    let base = dataset(
        Some(term_calendar()),
        vec![
            course(
                "18.01",
                "Calculus",
                vec![requirement(
                    "18.01/lecture",
                    "lecture",
                    vec![option("18.01.A", vec![meeting(MON, 9 * 60, 10 * 60)])],
                )],
            ),
            course(
                "PE.0413",
                "Backpacking",
                vec![requirement(
                    "PE.0413/pe",
                    "pe",
                    vec![
                        option(
                            "PE.0413.Q1",
                            vec![pe_meeting(
                                MON,
                                9 * 60 + 30,
                                10 * 60 + 30,
                                "2026-10-26",
                                "2026-10-30",
                            )],
                        ),
                        option(
                            "PE.0413.Q1.TUE",
                            vec![pe_meeting(TUE, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30")],
                        ),
                    ],
                )],
            ),
        ],
    );
    let solution = app::optimize(&base, &["18.01".into(), "PE.0413".into()], None).unwrap();
    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert_eq!(choice_for(&solution, "PE.0413/pe").id, "PE.0413.Q1.TUE");
}

#[test]
fn holiday_and_alternate_weekday_boundaries_affect_pe_collisions() {
    let calendar = TermCalendar {
        start: date("2026-11-02"),
        end: date("2026-11-03"),
        holidays: BTreeSet::from([date("2026-11-02")]),
        alternate_days: BTreeMap::from([(date("2026-11-03"), MON)]),
    };
    let academic = requirement(
        "18.01/lecture",
        "lecture",
        vec![option("18.01.A", vec![meeting(MON, 9 * 60, 10 * 60)])],
    );

    let holiday_only = vec![
        academic.clone(),
        requirement(
            "PE.HOLIDAY/pe",
            "pe",
            vec![option(
                "PE.HOLIDAY.Q2",
                vec![pe_meeting(
                    MON,
                    9 * 60 + 30,
                    10 * 60 + 30,
                    "2026-11-02",
                    "2026-11-02",
                )],
            )],
        ),
    ];
    assert_eq!(
        optimizer::solve_with_calendar(&holiday_only, &calendar, None)
            .unwrap()
            .status,
        SolveStatus::OptimalKnown
    );

    let alternate_only = vec![
        academic,
        requirement(
            "PE.ALTERNATE/pe",
            "pe",
            vec![option(
                "PE.ALTERNATE.Q2",
                vec![pe_meeting(
                    MON,
                    9 * 60 + 30,
                    10 * 60 + 30,
                    "2026-11-03",
                    "2026-11-03",
                )],
            )],
        ),
    ];
    assert_eq!(
        optimizer::solve(&alternate_only, None).unwrap().status,
        SolveStatus::OptimalKnown,
        "without a term calendar, the solver falls back to natural weekdays"
    );
    assert_eq!(
        optimizer::solve_with_calendar(&alternate_only, &calendar, None)
            .unwrap()
            .status,
        SolveStatus::Infeasible
    );
}

#[test]
fn export_bounded_pe_is_clipped_to_bounds_and_term_with_midnight_handling() {
    let calendar = term_calendar();
    let section = option(
        "PE.0413.Q1",
        vec![pe_meeting(
            MON,
            23 * 60,
            24 * 60,
            "2026-10-01",
            "2026-11-03",
        )],
    );
    let report = calendar::export_ics(&calendar, &[chosen_pe(section)]).unwrap();
    assert_eq!(report.event_count, 2);
    assert!(report.notices.is_empty());
    assert_eq!(report.ics.matches("BEGIN:VEVENT").count(), 1);
    let expanded = support::expand_calendar(&report.ics);
    assert!(expanded.contains("DTSTART:20261027T030000Z"));
    assert!(expanded.contains("DTEND:20261027T040000Z"));
    assert!(expanded.contains("DTSTART:20261104T040000Z"));
    assert!(expanded.contains("DTEND:20261104T050000Z"));
    assert!(!report.ics.contains("20261109"));
}

#[test]
fn invalid_or_incomplete_pe_dates_are_rejected_or_left_unresolved() {
    let calendar = term_calendar();
    let mut incomplete = pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30");
    incomplete.end_date = None;
    let requirements = vec![requirement(
        "PE.BAD/pe",
        "pe",
        vec![option("PE.BAD.Q1", vec![incomplete.clone()])],
    )];
    let solution = optimizer::solve_with_calendar(&requirements, &calendar, None).unwrap();
    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert!(solution.choices.is_empty());
    assert!(
        solution
            .unresolved
            .iter()
            .any(|notice| notice.contains("complete valid start/end date bounds"))
    );

    let base = dataset(
        Some(calendar),
        vec![course(
            "PE.BAD",
            "Bad PE",
            vec![requirement("PE.BAD/pe", "pe", vec![])],
        )],
    );
    assert!(
        app::manual_entry(
            &base,
            "PE.BAD",
            "pe",
            "No dates",
            "",
            vec![meeting(MON, 9 * 60, 10 * 60)],
            None
        )
        .is_err()
    );
    assert!(
        app::manual_entry(
            &base,
            "PE.BAD",
            "pe",
            "Alternate internal clash",
            "",
            vec![
                pe_meeting(MON, 9 * 60, 10 * 60, "2026-11-03", "2026-11-03"),
                pe_meeting(MON, 9 * 60 + 30, 10 * 60 + 30, "2026-11-03", "2026-11-03"),
            ],
            None
        )
        .is_err()
    );
    let report = calendar::export_ics(
        &term_calendar(),
        &[chosen_pe(option(
            "PE.BAD.NODATES",
            vec![meeting(MON, 9 * 60, 10 * 60)],
        ))],
    )
    .unwrap();
    assert_eq!(report.event_count, 0);
    assert!(
        report
            .notices
            .iter()
            .any(|notice| notice.contains("PE meetings require complete"))
    );
    assert!(
        app::manual_entry(
            &base,
            "PE.BAD",
            "pe",
            "Incomplete",
            "",
            vec![incomplete],
            None
        )
        .is_err()
    );
    assert!(
        Meeting {
            weekday: MON,
            start_minute: 9 * 60,
            end_minute: 10 * 60,
            start_date: Some(date("2026-10-30")),
            end_date: Some(date("2026-10-26")),
        }
        .validate()
        .is_err()
    );

    let temp = TempDir::new().unwrap();
    let invalid_entries = [
        ManualEntry {
            term_id: "f26".into(),
            course_id: "PE.BAD".into(),
            kind: "pe".into(),
            option: manual_option("manual-pe-no-dates", vec![meeting(MON, 9 * 60, 10 * 60)]),
            enabled: true,
        },
        ManualEntry {
            term_id: "f26".into(),
            course_id: "PE.BAD".into(),
            kind: "pe".into(),
            option: manual_option(
                "manual-pe-incomplete",
                vec![Meeting {
                    weekday: MON,
                    start_minute: 9 * 60,
                    end_minute: 10 * 60,
                    start_date: Some(date("2026-10-26")),
                    end_date: None,
                }],
            ),
            enabled: true,
        },
        ManualEntry {
            term_id: "f26".into(),
            course_id: "PE.BAD".into(),
            kind: "pe".into(),
            option: {
                let mut option = manual_option(
                    "manual-pe-unsupported",
                    vec![pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30")],
                );
                option.unsupported_reason = Some("must be none for bounded PE".into());
                option
            },
            enabled: true,
        },
        ManualEntry {
            term_id: "f26".into(),
            course_id: "PE.BAD".into(),
            kind: "pe".into(),
            option: manual_option(
                "manual-pe-overlap",
                vec![
                    pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30"),
                    pe_meeting(MON, 9 * 60 + 30, 10 * 60 + 30, "2026-10-26", "2026-10-30"),
                ],
            ),
            enabled: true,
        },
        ManualEntry {
            term_id: "f26".into(),
            course_id: "PE.BAD".into(),
            kind: "pe".into(),
            option: manual_option(
                "manual-pe-alt-overlap",
                vec![
                    pe_meeting(MON, 9 * 60, 10 * 60, "2026-11-03", "2026-11-03"),
                    pe_meeting(MON, 9 * 60 + 30, 10 * 60 + 30, "2026-11-03", "2026-11-03"),
                ],
            ),
            enabled: true,
        },
    ];
    for entry in invalid_entries {
        assert!(
            storage::save_manual(
                temp.path(),
                &ManualStore {
                    version: 1,
                    entries: vec![entry],
                },
            )
            .is_err()
        );
    }
}

#[test]
fn pe_manual_roundtrip_overlay_supports_bounded_entries_with_no_unsupported_reason() {
    let temp = TempDir::new().unwrap();
    let base = dataset(
        Some(term_calendar()),
        vec![course(
            "PE.0413",
            "Backpacking",
            vec![Requirement {
                id: "PE.0413/pe".into(),
                kind: "pe".into(),
                options: Vec::new(),
                has_unknown_times: true,
            }],
        )],
    );
    let entry = app::manual_entry(
        &base,
        "PE.0413",
        "pe",
        "Trip",
        "Trail",
        vec![pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30")],
        None,
    )
    .unwrap();
    assert_eq!(entry.kind, "pe");
    assert_eq!(entry.option.unsupported_reason, None);

    let store = ManualStore {
        version: 1,
        entries: vec![entry.clone()],
    };
    storage::save_manual(temp.path(), &store).unwrap();
    let loaded = storage::load_manual(temp.path()).unwrap();
    assert_eq!(loaded.entries[0].option.unsupported_reason, None);
    let merged = storage::apply_manual(&base, &loaded).unwrap();
    let pe_requirement = &merged.courses["PE.0413"].requirements[0];
    assert!(!pe_requirement.has_unknown_times);
    assert_eq!(pe_requirement.options[0].id, entry.option.id);

    let solution = app::optimize(&merged, &["PE.0413".to_string()], None).unwrap();
    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert_eq!(solution.choices.len(), 1);
}

#[test]
fn non_pe_date_limited_behavior_stays_omitted_and_unsupported() {
    let dated = pe_meeting(MON, 9 * 60, 10 * 60, "2026-10-26", "2026-10-30");
    let requirements = vec![requirement(
        "P/lecture",
        "lecture",
        vec![option("partial", vec![dated.clone()])],
    )];
    let solution = optimizer::solve(&requirements, None).unwrap();
    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert!(solution.choices.is_empty());
    assert!(
        solution
            .unresolved
            .iter()
            .any(|notice| notice.contains("partial-term optimization is not supported"))
    );

    let mut manual = option("manual-dated", vec![dated]);
    manual.source = Source::Manual;
    manual.unsupported_reason =
        Some("date-limited manual section: partial-term optimization is not supported".into());
    let report = calendar::export_ics(
        &term_calendar(),
        &[ChosenSection {
            course_id: "P".into(),
            course_title: "Partial".into(),
            kind: "lecture".into(),
            section: manual,
        }],
    )
    .unwrap();
    assert_eq!(report.event_count, 0);
    assert!(
        report
            .notices
            .iter()
            .any(|notice| notice.contains("partial-term"))
    );
}
