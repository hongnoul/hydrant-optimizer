use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
};

use chrono::NaiveDate;
use hydrant_optimizer::{model::*, optimizer};

const MON: u8 = 0;
const TUE: u8 = 1;
const WED: u8 = 2;
const THU: u8 = 3;
const SUN: u8 = 6;

#[derive(Clone, Debug)]
struct OracleChoice {
    requirement_id: String,
    option: SectionOption,
    meetings: Vec<Meeting>,
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

fn dated_meeting(weekday: u8, start: u16, end: u16) -> Meeting {
    Meeting {
        weekday,
        start_minute: start,
        end_minute: end,
        start_date: Some(NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()),
        end_date: Some(NaiveDate::from_ymd_opt(2026, 10, 30).unwrap()),
    }
}

fn dated_range_meeting(
    weekday: u8,
    start: u16,
    end: u16,
    start_date: (i32, u32, u32),
    end_date: (i32, u32, u32),
) -> Meeting {
    Meeting {
        weekday,
        start_minute: start,
        end_minute: end,
        start_date: Some(
            NaiveDate::from_ymd_opt(start_date.0, start_date.1, start_date.2).unwrap(),
        ),
        end_date: Some(NaiveDate::from_ymd_opt(end_date.0, end_date.1, end_date.2).unwrap()),
    }
}

fn option(id: &str, meetings: Vec<Meeting>) -> SectionOption {
    SectionOption {
        id: id.to_string(),
        label: id.to_string(),
        room: format!("room-{id}"),
        source: Source::Manual,
        meetings,
        incompatible_with: BTreeSet::new(),
        unsupported_reason: None,
    }
}

fn incompatible(mut option: SectionOption, ids: &[&str]) -> SectionOption {
    option.incompatible_with = ids.iter().map(|id| (*id).to_string()).collect();
    option
}

fn unsupported(mut option: SectionOption, reason: &str) -> SectionOption {
    option.unsupported_reason = Some(reason.to_string());
    option
}

fn requirement(id: &str, options: Vec<SectionOption>) -> Requirement {
    Requirement {
        id: id.to_string(),
        kind: "lecture".to_string(),
        options,
        has_unknown_times: false,
    }
}

fn unknown_requirement(id: &str, options: Vec<SectionOption>) -> Requirement {
    Requirement {
        id: id.to_string(),
        kind: "recitation".to_string(),
        options,
        has_unknown_times: true,
    }
}

fn choice_for<'a>(solution: &'a Solution, requirement_id: &str) -> &'a TimeChoice {
    solution
        .choices
        .iter()
        .find(|choice| choice.requirement_id == requirement_id)
        .unwrap_or_else(|| panic!("missing choice for {requirement_id}"))
}

fn member_ids(choice: &TimeChoice) -> Vec<String> {
    choice
        .members
        .iter()
        .map(|member| member.id.clone())
        .collect()
}

fn canonical(mut meetings: Vec<Meeting>) -> Vec<Meeting> {
    meetings.sort();
    meetings
}

fn independently_supported(option: &SectionOption) -> Option<Vec<Meeting>> {
    if option.unsupported_reason.is_some() || option.meetings.is_empty() {
        return None;
    }
    let mut meetings = option.meetings.clone();
    for meeting in &meetings {
        if meeting.weekday >= 7
            || meeting.start_minute >= meeting.end_minute
            || meeting.end_minute > 1440
            || meeting.start_date.is_some()
            || meeting.end_date.is_some()
        {
            return None;
        }
    }
    meetings.sort();
    if meetings
        .windows(2)
        .any(|pair| pair[0].weekday == pair[1].weekday && pair[0].end_minute > pair[1].start_minute)
    {
        return None;
    }
    Some(meetings)
}

fn independent_domains(requirements: &[Requirement]) -> Vec<(String, Vec<OracleChoice>)> {
    let mut domains = Vec::new();
    for requirement in requirements {
        let mut choices = requirement
            .options
            .iter()
            .filter_map(|option| {
                independently_supported(option).map(|meetings| OracleChoice {
                    requirement_id: requirement.id.clone(),
                    option: option.clone(),
                    meetings,
                })
            })
            .collect::<Vec<_>>();
        choices.sort_by(|a, b| a.option.id.cmp(&b.option.id));
        if !choices.is_empty() {
            domains.push((requirement.id.clone(), choices));
        }
    }
    domains.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.0.cmp(&b.0)));
    domains
}

fn intervals_overlap(a: &[Meeting], b: &[Meeting]) -> bool {
    a.iter().any(|left| {
        b.iter().any(|right| {
            left.weekday == right.weekday
                && left.start_minute.max(right.start_minute) < left.end_minute.min(right.end_minute)
        })
    })
}

fn pair_incompatible(a: &SectionOption, b: &SectionOption) -> bool {
    a.incompatible_with.contains(&b.id) || b.incompatible_with.contains(&a.id)
}

fn feasible_with_selected(candidate: &OracleChoice, selected: &[OracleChoice]) -> bool {
    selected.iter().all(|other| {
        !intervals_overlap(&candidate.meetings, &other.meetings)
            && !pair_incompatible(&candidate.option, &other.option)
    })
}

fn independent_score_from_meetings<'a>(meetings: impl IntoIterator<Item = &'a Meeting>) -> Score {
    let mut by_day: [Vec<(u16, u16)>; 7] = std::array::from_fn(|_| Vec::new());
    for meeting in meetings {
        by_day[meeting.weekday as usize].push((meeting.start_minute, meeting.end_minute));
    }

    let mut occupied_days = 0;
    let mut gap_minutes = 0;
    for intervals in &mut by_day {
        if intervals.is_empty() {
            continue;
        }
        intervals.sort();
        occupied_days += 1;
        let first = intervals.first().unwrap().0;
        let last = intervals.last().unwrap().1;
        let occupied = intervals
            .iter()
            .map(|(start, end)| u32::from(*end - *start))
            .sum::<u32>();
        gap_minutes += u32::from(last - first) - occupied;
    }
    Score {
        occupied_days,
        gap_minutes,
    }
}

fn independent_score(selected: &[OracleChoice]) -> Score {
    independent_score_from_meetings(selected.iter().flat_map(|choice| choice.meetings.iter()))
}

fn oracle_optima(requirements: &[Requirement]) -> Option<(Score, Vec<Vec<OracleChoice>>)> {
    let domains = independent_domains(requirements);

    fn visit(
        index: usize,
        domains: &[(String, Vec<OracleChoice>)],
        selected: &mut Vec<OracleChoice>,
        feasible: &mut Vec<(Score, Vec<OracleChoice>)>,
    ) {
        if index == domains.len() {
            feasible.push((independent_score(selected), selected.clone()));
            return;
        }
        for candidate in &domains[index].1 {
            if feasible_with_selected(candidate, selected) {
                selected.push(candidate.clone());
                visit(index + 1, domains, selected, feasible);
                selected.pop();
            }
        }
    }

    let mut feasible = Vec::new();
    visit(0, &domains, &mut Vec::new(), &mut feasible);
    let best = feasible
        .iter()
        .map(|(score, _)| *score)
        .min_by_key(|score| (score.occupied_days, score.gap_minutes))?;
    Some((
        best,
        feasible
            .into_iter()
            .filter_map(|(score, assignment)| (score == best).then_some(assignment))
            .collect(),
    ))
}

fn assert_solution_matches_oracle(requirements: &[Requirement]) -> Solution {
    let expected = oracle_optima(requirements);
    let solution = optimizer::solve(requirements, None).unwrap();
    match expected {
        Some((score, assignments)) => {
            assert_eq!(solution.status, SolveStatus::OptimalKnown);
            assert_eq!(solution.score, Some(score));
            assert_returned_assignment_is_supported_feasible_and_scored(requirements, &solution);
            assert_returned_assignment_matches_oracle_choice(&assignments[0], &solution);
            assert_all_alternatives_match_oracle(requirements, &assignments, &solution);
        }
        None => {
            assert_eq!(solution.status, SolveStatus::Infeasible);
            assert_eq!(solution.score, None);
            assert!(solution.choices.is_empty());
            assert!(solution.alternatives.is_empty());
            assert!(!solution.alternatives_truncated);
        }
    }
    solution
}

fn choice_ids(choices: &[TimeChoice]) -> Vec<(String, String)> {
    choices
        .iter()
        .map(|choice| (choice.requirement_id.clone(), choice.id.clone()))
        .collect()
}

fn assert_all_alternatives_match_oracle(
    requirements: &[Requirement],
    expected: &[Vec<OracleChoice>],
    solution: &Solution,
) {
    assert!(!solution.alternatives_truncated);
    assert_eq!(
        serde_json::to_value(&solution.choices).unwrap(),
        serde_json::to_value(&solution.alternatives[0]).unwrap()
    );
    let expected_members = expected
        .iter()
        .map(|assignment| {
            let mut ids = assignment
                .iter()
                .map(|choice| (choice.requirement_id.clone(), choice.option.id.clone()))
                .collect::<Vec<_>>();
            ids.sort();
            ids
        })
        .collect::<BTreeSet<_>>();
    let mut actual_members = BTreeSet::new();
    let mut grouped_ids = BTreeSet::new();
    for choices in &solution.alternatives {
        let ids = choice_ids(choices);
        assert!(ids.is_sorted(), "choices must be in requirement/id order");
        assert!(grouped_ids.insert(ids), "duplicate grouped timetable");
        let active = Solution {
            choices: choices.clone(),
            ..solution.clone()
        };
        assert_returned_assignment_is_supported_feasible_and_scored(requirements, &active);

        // Expand the returned member groups independently. Their union must be
        // exactly the oracle's complete set of optimal actual-section tuples.
        let mut expanded = vec![Vec::new()];
        for choice in choices {
            expanded = expanded
                .into_iter()
                .flat_map(|prefix| {
                    choice.members.iter().map(move |member| {
                        let mut ids = prefix.clone();
                        ids.push((choice.requirement_id.clone(), member.id.clone()));
                        ids
                    })
                })
                .collect();
        }
        for ids in expanded {
            assert!(actual_members.insert(ids), "duplicate actual-section tuple");
        }
    }
    assert_eq!(actual_members, expected_members);
}

fn assert_returned_assignment_matches_oracle_choice(
    expected_assignment: &[OracleChoice],
    solution: &Solution,
) {
    for expected in expected_assignment {
        let choice = choice_for(solution, &expected.requirement_id);
        assert!(
            choice
                .members
                .iter()
                .any(|member| member.id == expected.option.id),
            "returned assignment for {} did not retain expected oracle option {}",
            expected.requirement_id,
            expected.option.id
        );
        assert_eq!(choice.meetings, expected.meetings);
    }
}

fn assert_returned_assignment_is_supported_feasible_and_scored(
    requirements: &[Requirement],
    solution: &Solution,
) {
    let mut option_requirement = std::collections::BTreeMap::new();
    for requirement in requirements {
        for option in &requirement.options {
            option_requirement.insert(option.id.clone(), requirement.id.clone());
        }
    }

    for choice in &solution.choices {
        assert!(
            !choice.members.is_empty(),
            "optimizer returned an empty member group"
        );
        let canonical_choice_meetings = canonical(choice.meetings.clone());
        assert_eq!(choice.meetings, canonical_choice_meetings);
        for member in &choice.members {
            assert_eq!(
                option_requirement.get(&member.id),
                Some(&choice.requirement_id),
                "member belongs to a different requirement"
            );
            assert_eq!(
                independently_supported(member).as_deref(),
                Some(choice.meetings.as_slice()),
                "member does not preserve the chosen same-time pattern"
            );
        }
    }

    for (i, left) in solution.choices.iter().enumerate() {
        for right in solution.choices.iter().skip(i + 1) {
            assert!(!intervals_overlap(&left.meetings, &right.meetings));
            for left_member in &left.members {
                for right_member in &right.members {
                    assert!(!pair_incompatible(left_member, right_member));
                }
            }
        }
    }

    let score = independent_score_from_meetings(
        solution
            .choices
            .iter()
            .flat_map(|choice| choice.meetings.iter()),
    );
    assert_eq!(solution.score, Some(score));
}

#[test]
fn empty_and_initially_empty_supported_domains_are_unresolved_not_infeasible() {
    let empty = optimizer::solve(&[], None).unwrap();
    assert_eq!(empty.status, SolveStatus::OptimalKnown);
    assert_eq!(
        empty.score,
        Some(Score {
            occupied_days: 0,
            gap_minutes: 0,
        })
    );
    assert!(empty.choices.is_empty());
    assert_eq!(empty.alternatives.len(), 1);
    assert!(empty.alternatives[0].is_empty());
    assert!(!empty.alternatives_truncated);
    assert!(empty.unresolved.is_empty());

    let requirements = vec![unknown_requirement(
        "A/recitation",
        vec![
            option("tba", Vec::new()),
            unsupported(
                option("half", vec![dated_meeting(MON, 9 * 60, 10 * 60)]),
                "half-term data",
            ),
            option(
                "split-partial-overlap",
                vec![
                    dated_range_meeting(WED, 9 * 60, 10 * 60, (2026, 10, 26), (2026, 10, 30)),
                    dated_range_meeting(WED, 9 * 60, 10 * 60, (2026, 11, 2), (2026, 11, 6)),
                ],
            ),
        ],
    )];
    let solution = assert_solution_matches_oracle(&requirements);
    assert!(solution.choices.is_empty());
    assert!(
        solution
            .unresolved
            .iter()
            .any(|notice| notice.contains("A/recitation") && notice.contains("unknown"))
    );
    assert!(
        solution
            .unresolved
            .iter()
            .any(|notice| notice.contains("date-limited") || notice.contains("half-term"))
    );
    assert!(
        solution
            .unresolved
            .iter()
            .any(|notice| notice.contains("no known supported"))
    );
}

#[test]
fn exhaustive_oracle_covers_adjacency_atomic_bundles_exact_minutes_and_boundaries() {
    let requirements = vec![
        requirement(
            "A/lecture",
            vec![
                option("a1", vec![meeting(MON, 9 * 60, 10 * 60)]),
                option("a2", vec![meeting(TUE, 9 * 60, 9 * 60 + 45)]),
            ],
        ),
        requirement(
            "B/lecture",
            vec![
                option("b1", vec![meeting(MON, 10 * 60, 11 * 60)]),
                option("b2", vec![meeting(MON, 9 * 60 + 30, 10 * 60 + 30)]),
            ],
        ),
        requirement(
            "C/lab",
            vec![option(
                "c1",
                vec![meeting(WED, 0, 1), meeting(SUN, 23 * 60 + 59, 24 * 60)],
            )],
        ),
    ];

    let solution = assert_solution_matches_oracle(&requirements);
    assert_eq!(member_ids(choice_for(&solution, "A/lecture")), vec!["a1"]);
    assert_eq!(member_ids(choice_for(&solution, "B/lecture")), vec!["b1"]);
    assert_eq!(member_ids(choice_for(&solution, "C/lab")), vec!["c1"]);
    assert_eq!(
        solution.score,
        Some(Score {
            occupied_days: 3,
            gap_minutes: 0,
        })
    );
}

#[test]
fn day_priority_later_gap_filling_and_equal_day_subsets_are_exact() {
    let day_priority = vec![
        requirement("A", vec![option("a", vec![meeting(MON, 9 * 60, 10 * 60)])]),
        requirement(
            "B",
            vec![
                option("b-same-day", vec![meeting(MON, 15 * 60, 16 * 60)]),
                option("b-no-gap", vec![meeting(TUE, 9 * 60, 10 * 60)]),
            ],
        ),
    ];
    let solution = assert_solution_matches_oracle(&day_priority);
    assert_eq!(member_ids(choice_for(&solution, "B")), vec!["b-same-day"]);
    assert_eq!(
        solution.score,
        Some(Score {
            occupied_days: 1,
            gap_minutes: 5 * 60,
        })
    );

    let later_gap_fill = vec![
        requirement("A", vec![option("a", vec![meeting(MON, 9 * 60, 10 * 60)])]),
        requirement("B", vec![option("b", vec![meeting(MON, 15 * 60, 16 * 60)])]),
        requirement(
            "C",
            vec![
                option("c-fill-later", vec![meeting(MON, 11 * 60, 15 * 60)]),
                option("c-extra-day", vec![meeting(THU, 9 * 60, 10 * 60)]),
            ],
        ),
    ];
    let solution = assert_solution_matches_oracle(&later_gap_fill);
    assert_eq!(member_ids(choice_for(&solution, "C")), vec!["c-fill-later"]);
    assert_eq!(
        solution.score,
        Some(Score {
            occupied_days: 1,
            gap_minutes: 60,
        })
    );

    let equal_day_subsets = vec![
        requirement(
            "A",
            vec![
                option("a-mon", vec![meeting(MON, 9 * 60, 10 * 60)]),
                option("a-tue", vec![meeting(TUE, 9 * 60, 10 * 60)]),
            ],
        ),
        requirement(
            "B",
            vec![
                option("b-mon", vec![meeting(MON, 17 * 60, 18 * 60)]),
                option("b-tue", vec![meeting(TUE, 10 * 60, 11 * 60)]),
            ],
        ),
    ];
    let solution = assert_solution_matches_oracle(&equal_day_subsets);
    assert_eq!(member_ids(choice_for(&solution, "A")), vec!["a-tue"]);
    assert_eq!(member_ids(choice_for(&solution, "B")), vec!["b-tue"]);
    assert_eq!(
        solution.score,
        Some(Score {
            occupied_days: 1,
            gap_minutes: 0,
        })
    );
}

#[test]
fn grouping_retains_members_but_splits_outbound_and_inbound_incompatibilities() {
    let requirements = vec![
        requirement(
            "A/lecture",
            vec![
                incompatible(option("a1", vec![meeting(MON, 9 * 60, 10 * 60)]), &["b1"]),
                option("a2", vec![meeting(MON, 9 * 60, 10 * 60)]),
                option("a3", vec![meeting(MON, 9 * 60, 10 * 60)]),
                option("a4", vec![meeting(MON, 9 * 60, 10 * 60)]),
            ],
        ),
        requirement(
            "B/lecture",
            vec![incompatible(
                option("b1", vec![meeting(TUE, 9 * 60, 10 * 60)]),
                &["a3"],
            )],
        ),
    ];

    let solution = assert_solution_matches_oracle(&requirements);
    assert_eq!(member_ids(choice_for(&solution, "B/lecture")), vec!["b1"]);
    assert_eq!(
        member_ids(choice_for(&solution, "A/lecture")),
        vec!["a2", "a4"]
    );
    assert_eq!(
        choice_for(&solution, "A/lecture").meetings,
        vec![meeting(MON, 9 * 60, 10 * 60)]
    );
}

#[test]
fn deterministic_ties_and_public_group_helper_are_stable() {
    let tied = vec![requirement(
        "A",
        vec![
            option("z-later-id", vec![meeting(TUE, 9 * 60, 10 * 60)]),
            option("a-earlier-id", vec![meeting(MON, 9 * 60, 10 * 60)]),
        ],
    )];
    let first = assert_solution_matches_oracle(&tied);
    let second = optimizer::solve(&tied, None).unwrap();
    assert_eq!(first.choices[0].id, "a-earlier-id");
    assert_eq!(first.choices[0].id, second.choices[0].id);
    assert_eq!(first.alternatives.len(), 2);
    assert_eq!(first.alternatives[1][0].id, "z-later-id");
    assert_eq!(
        serde_json::to_value(&first.alternatives).unwrap(),
        serde_json::to_value(&second.alternatives).unwrap()
    );

    let grouped_requirement = requirement(
        "G",
        vec![
            option("g2", vec![meeting(WED, 13 * 60 + 5, 14 * 60)]),
            option("g1", vec![meeting(WED, 13 * 60 + 5, 14 * 60)]),
        ],
    );
    let groups = optimizer::group(&grouped_requirement).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(member_ids(&groups[0]), vec!["g1", "g2"]);
    assert_eq!(groups[0].meetings, vec![meeting(WED, 13 * 60 + 5, 14 * 60)]);
}

#[test]
fn infeasible_invalid_bounds_and_cancellation_never_claim_false_optimum() {
    let conflict = vec![
        requirement("A", vec![option("a", vec![meeting(MON, 9 * 60, 10 * 60)])]),
        requirement(
            "B",
            vec![option("b", vec![meeting(MON, 9 * 60 + 30, 10 * 60)])],
        ),
    ];
    assert_solution_matches_oracle(&conflict);

    let invalid = vec![requirement(
        "bad",
        vec![option("bad-option", vec![meeting(MON, 12 * 60, 11 * 60)])],
    )];
    assert!(optimizer::solve(&invalid, None).is_err());

    let internal_overlap = vec![requirement(
        "overlap",
        vec![option(
            "bad-bundle",
            vec![
                meeting(MON, 9 * 60, 10 * 60),
                meeting(MON, 9 * 60 + 30, 11 * 60),
            ],
        )],
    )];
    assert!(optimizer::solve(&internal_overlap, None).is_err());

    let cancelled_flag = AtomicBool::new(true);
    let cancelled = optimizer::solve(&conflict, Some(&cancelled_flag)).unwrap();
    assert_eq!(cancelled.status, SolveStatus::Cancelled);
    assert_eq!(cancelled.score, None);
    assert!(cancelled.choices.is_empty());
    assert!(cancelled.alternatives.is_empty());
    assert!(!cancelled.alternatives_truncated);

    cancelled_flag.store(false, Ordering::Relaxed);
    let completed = optimizer::solve(&conflict, Some(&cancelled_flag)).unwrap();
    assert_eq!(completed.status, SolveStatus::Infeasible);
}

#[test]
fn alternatives_preserve_distinct_grouped_choices_even_with_identical_occupancy() {
    let mut requirements = vec![
        requirement(
            "B",
            vec![
                option("b-late", vec![meeting(MON, 600, 660)]),
                option("b-early", vec![meeting(MON, 540, 600)]),
            ],
        ),
        requirement(
            "A",
            vec![
                option("a-late", vec![meeting(MON, 600, 660)]),
                option("a-early-room2", vec![meeting(MON, 540, 600)]),
                option("a-early-room1", vec![meeting(MON, 540, 600)]),
            ],
        ),
    ];
    let solution = optimizer::solve(&requirements, None).unwrap();
    let (score, expected) = oracle_optima(&requirements).unwrap();
    assert_eq!(solution.score, Some(score));
    assert_all_alternatives_match_oracle(&requirements, &expected, &solution);
    assert_eq!(solution.alternatives.len(), 2);
    assert_eq!(solution.alternatives[0][0].members.len(), 2);
    assert_eq!(solution.alternatives[1][0].id, "a-late");

    requirements.reverse();
    for requirement in &mut requirements {
        requirement.options.reverse();
    }
    let reordered = optimizer::solve(&requirements, None).unwrap();
    assert_eq!(
        serde_json::to_value(solution).unwrap(),
        serde_json::to_value(reordered).unwrap()
    );
}

#[test]
fn equally_optimal_pairing_groups_remain_distinct_and_safe_for_every_member() {
    let requirements = vec![
        requirement(
            "A",
            vec![
                incompatible(option("a1", vec![meeting(MON, 540, 600)]), &["b2"]),
                option("a2", vec![meeting(MON, 540, 600)]),
                option("a3", vec![meeting(MON, 540, 600)]),
            ],
        ),
        requirement(
            "B",
            vec![
                incompatible(option("b1", vec![meeting(MON, 600, 660)]), &["a2", "a3"]),
                option("b2", vec![meeting(MON, 600, 660)]),
            ],
        ),
    ];
    let solution = assert_solution_matches_oracle(&requirements);
    assert_eq!(solution.alternatives.len(), 2);
    assert_eq!(member_ids(&solution.alternatives[1][0]), vec!["a2", "a3"]);
}

#[test]
fn alternatives_match_exhaustive_oracle_on_generated_small_domains() {
    for seed in 0..64_u32 {
        let mut state = seed + 1;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 16
        };
        let mut requirements = Vec::new();
        for group in 0..3 {
            let mut options = Vec::new();
            for index in 0..3 {
                let weekday = (next() % 3) as u8;
                let start = 540 + (next() % 4) as u16 * 30;
                let mut choice = option(
                    &format!("{group}-{index}"),
                    vec![meeting(weekday, start, start + 30)],
                );
                if group < 2 && next() % 3 == 0 {
                    choice
                        .incompatible_with
                        .insert(format!("{}-{}", group + 1, next() % 3));
                }
                options.push(choice);
            }
            requirements.push(requirement(&format!("G{group}"), options));
        }
        let solution = optimizer::solve(&requirements, None).unwrap();
        if let Some((score, expected)) = oracle_optima(&requirements) {
            assert_eq!(solution.status, SolveStatus::OptimalKnown, "seed {seed}");
            assert_eq!(solution.score, Some(score), "seed {seed}");
            assert_all_alternatives_match_oracle(&requirements, &expected, &solution);
        } else {
            assert_eq!(solution.status, SolveStatus::Infeasible, "seed {seed}");
            assert!(solution.alternatives.is_empty());
            assert!(!solution.alternatives_truncated);
        }
    }
}

#[test]
fn alternative_limit_is_deterministic_and_disclosed_only_when_exceeded() {
    let cap = optimizer::MAX_TIMETABLE_ALTERNATIVES;
    let mut requirement = requirement(
        "A",
        (0..cap)
            .map(|index| {
                let start = index as u16;
                option(
                    &format!("a{index:04}"),
                    vec![meeting(MON, start, start + 1)],
                )
            })
            .collect(),
    );
    let exact = optimizer::solve(&[requirement.clone()], None).unwrap();
    assert_eq!(exact.alternatives.len(), cap);
    assert!(!exact.alternatives_truncated);
    assert!(exact.unresolved.is_empty());

    requirement
        .options
        .push(option("z-extra", vec![meeting(TUE, 540, 600)]));
    requirement.options.reverse();
    let truncated = optimizer::solve(&[requirement], None).unwrap();
    assert_eq!(truncated.alternatives.len(), cap);
    assert!(truncated.alternatives_truncated);
    assert!(
        truncated
            .unresolved
            .iter()
            .any(|notice| { notice.contains(&cap.to_string()) && notice.contains("omitted") })
    );
    assert_eq!(
        serde_json::to_value(&truncated.alternatives).unwrap(),
        serde_json::to_value(&exact.alternatives).unwrap()
    );
    assert_eq!(truncated.score, exact.score);
}

#[test]
fn better_score_after_full_alternative_buffer_clears_old_choices_and_truncation() {
    let mut options = (0..=optimizer::MAX_TIMETABLE_ALTERNATIVES)
        .map(|index| {
            let start = index as u16;
            option(
                &format!("a{index:04}"),
                vec![meeting(MON, start, start + 1), meeting(TUE, 540, 600)],
            )
        })
        .collect::<Vec<_>>();
    options.extend([
        option("z-best1", vec![meeting(WED, 540, 600)]),
        option("z-best2", vec![meeting(THU, 540, 600)]),
    ]);
    let solution = optimizer::solve(&[requirement("A", options)], None).unwrap();
    assert_eq!(solution.alternatives.len(), 2);
    assert_eq!(solution.choices[0].id, "z-best1");
    assert_eq!(solution.alternatives[1][0].id, "z-best2");
    assert_eq!(solution.score.unwrap().occupied_days, 1);
    assert!(!solution.alternatives_truncated);
    assert!(solution.unresolved.is_empty());
}

#[test]
fn dated_alternatives_retain_date_identity_and_respect_calendar_conflicts() {
    let first_quarter = dated_range_meeting(MON, 540, 600, (2026, 9, 1), (2026, 10, 1));
    let mut requirements = vec![
        requirement("A", vec![option("a", vec![first_quarter.clone()])]),
        requirement(
            "B",
            vec![
                option("b-conflict", vec![first_quarter]),
                option(
                    "b-q2",
                    vec![dated_range_meeting(
                        MON,
                        540,
                        600,
                        (2026, 10, 2),
                        (2026, 11, 1),
                    )],
                ),
                option(
                    "b-q3",
                    vec![dated_range_meeting(
                        MON,
                        540,
                        600,
                        (2026, 11, 2),
                        (2026, 12, 1),
                    )],
                ),
            ],
        ),
    ];
    for requirement in &mut requirements {
        requirement.kind = "pe".to_string();
    }
    let calendar = TermCalendar {
        start: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        end: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        holidays: BTreeSet::new(),
        alternate_days: Default::default(),
    };
    for solution in [
        optimizer::solve(&requirements, None).unwrap(),
        optimizer::solve_with_calendar(&requirements, &calendar, None).unwrap(),
    ] {
        assert_eq!(solution.status, SolveStatus::OptimalKnown);
        assert_eq!(solution.alternatives.len(), 2);
        assert_eq!(solution.alternatives[0][1].id, "b-q2");
        assert_eq!(solution.alternatives[1][1].id, "b-q3");
        assert_eq!(
            solution.score,
            Some(Score {
                occupied_days: 1,
                gap_minutes: 0
            })
        );
        assert!(!solution.alternatives_truncated);
    }
}

#[test]
fn solution_json_roundtrips_alternatives_and_accepts_legacy_payloads() {
    let solution = optimizer::solve(
        &[requirement(
            "A",
            vec![
                option("a", vec![meeting(MON, 540, 600)]),
                option("b", vec![meeting(TUE, 540, 600)]),
            ],
        )],
        None,
    )
    .unwrap();
    let json = serde_json::to_value(&solution).unwrap();
    let decoded: Solution = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), json);

    let mut legacy_json = json;
    legacy_json.as_object_mut().unwrap().remove("alternatives");
    legacy_json
        .as_object_mut()
        .unwrap()
        .remove("alternatives_truncated");
    let legacy: Solution = serde_json::from_value(legacy_json).unwrap();
    assert!(legacy.alternatives.is_empty());
    assert!(!legacy.alternatives_truncated);
    assert_eq!(choice_ids(&legacy.choices), choice_ids(&solution.choices));
    assert_eq!(legacy.score, solution.score);
    assert_eq!(legacy.status, solution.status);
}
