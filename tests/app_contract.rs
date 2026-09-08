use std::collections::BTreeMap;

use chrono::NaiveDate;
use hydrant_optimizer::{adapter, app, model::*, storage};

fn dataset() -> Dataset {
    adapter::parse_catalog(
        include_str!("fixtures/catalog.json"),
        include_str!("fixtures/term.json"),
    )
    .unwrap()
}

#[test]
fn manual_validation_ids_and_edit_identity() {
    let base = dataset();
    let meetings = storage::parse_meetings("Sun 23:01-24:00;Mon 09:05-09:55").unwrap();
    let a =
        app::manual_entry(&base, "A", "lab", "Evening", "Room", meetings.clone(), None).unwrap();
    let b =
        app::manual_entry(&base, "A", "lab", "Evening", "Room", meetings.clone(), None).unwrap();
    assert_eq!(a.option.id, b.option.id);
    let changed = app::manual_entry(
        &base,
        "A",
        "lab",
        "Revised",
        "Other",
        meetings.clone(),
        Some(&a.option.id),
    )
    .unwrap();
    assert_eq!(changed.option.id, a.option.id);
    assert!(app::manual_entry(&base, "unknown", "lab", "x", "", meetings.clone(), None).is_err());
    assert!(app::manual_entry(&base, "A", "unknown", "x", "", meetings.clone(), None).is_err());
    assert!(app::manual_entry(&base, "A", "lab", "", "", meetings.clone(), None).is_err());
    assert!(app::manual_entry(&base, "A", "lab", "x", "", Vec::new(), None).is_err());
    assert!(app::manual_entry(&base, "A", "lab", "x", "", meetings, Some("api-id")).is_err());
    let mut overlaps = storage::parse_meetings("Mon 09:00-10:00").unwrap();
    overlaps.extend(storage::parse_meetings("Mon 09:30-10:30").unwrap());
    assert!(app::manual_entry(&base, "A", "lab", "x", "", overlaps, None).is_err());
}

#[test]
fn missing_calendar_and_invalid_member_never_create_output() {
    let mut base = dataset();
    let solution = app::optimize(&base, &["B".to_owned()], None).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("must-not-exist.ics");
    let bad = BTreeMap::from([("B/lecture".to_string(), "not-a-member".to_string())]);
    assert!(app::write_calendar(&base, &solution, &bad, &output).is_err());
    assert!(!output.exists());
    base.calendar = None;
    assert!(app::write_calendar(&base, &solution, &BTreeMap::new(), &output).is_err());
    assert!(!output.exists());
}

#[test]
fn interval_bounds_and_date_order_are_checked_without_panics() {
    let mut m = Meeting {
        weekday: 0,
        start_minute: 0,
        end_minute: 1440,
        start_date: None,
        end_date: None,
    };
    m.validate().unwrap();
    m.weekday = 7;
    assert!(m.validate().is_err());
    m.weekday = 0;
    m.start_minute = 1440;
    assert!(m.validate().is_err());
    m.start_minute = 0;
    m.end_minute = 1441;
    assert!(m.validate().is_err());
    m.end_minute = 1440;
    m.start_date = Some(NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
    m.end_date = Some(NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
    assert!(m.validate().is_err());
}

#[test]
fn duplicate_subject_selection_does_not_double_schedule() {
    let base = dataset();
    let one = app::optimize(&base, &["B".to_string()], None).unwrap();
    let two = app::optimize(&base, &["B".to_string(), "B".to_string()], None).unwrap();
    assert_eq!(one.score, two.score);
    assert_eq!(one.choices.len(), two.choices.len());
}

#[test]
fn known_nonstandard_component_accepts_manual_times() {
    let mut base = dataset();
    base.courses
        .get_mut("A")
        .unwrap()
        .requirements
        .push(Requirement {
            id: "A/tutorial".to_string(),
            kind: "tutorial".to_string(),
            options: vec![],
            has_unknown_times: true,
        });
    let entry = app::manual_entry(
        &base,
        "A",
        " Tutorial ",
        "Tutor",
        "",
        storage::parse_meetings("Tue 11:00-12:00").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(entry.kind, "tutorial");
    let merged = storage::apply_manual(
        &base,
        &ManualStore {
            version: 1,
            entries: vec![entry],
        },
    )
    .unwrap();
    let solution = app::optimize(&merged, &["A".to_string()], None).unwrap();
    assert!(
        solution
            .choices
            .iter()
            .any(|choice| choice.requirement_id == "A/tutorial")
    );
}

#[test]
fn actual_member_resolution_rejects_incompatibilities_in_either_direction() {
    let base = dataset();
    let solution = app::optimize(&base, &["A".to_string(), "B".to_string()], None).unwrap();
    app::actual_sections(&base, &solution, &BTreeMap::new()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    for reverse in [false, true] {
        // Defense against an invalid externally supplied or stale TimeChoice, independent of grouping.
        let mut invalid = solution.clone();
        let a = invalid
            .choices
            .iter()
            .position(|c| c.requirement_id == "A/lecture")
            .unwrap();
        let b = invalid
            .choices
            .iter()
            .position(|c| c.requirement_id == "B/lecture")
            .unwrap();
        let aid = invalid.choices[a].members[1].id.clone();
        let bid = invalid.choices[b].members[0].id.clone();
        if reverse {
            invalid.choices[b].members[0]
                .incompatible_with
                .insert(aid.clone());
        } else {
            invalid.choices[a].members[1].incompatible_with.insert(bid);
        }
        app::actual_sections(&base, &invalid, &BTreeMap::new()).unwrap();
        let selected = BTreeMap::from([("A/lecture".to_string(), aid)]);
        let output = temp.path().join(format!("unsafe-{reverse}.ics"));
        assert!(app::actual_sections(&base, &invalid, &selected).is_err());
        assert!(app::write_calendar(&base, &invalid, &selected, &output).is_err());
        assert!(!output.exists());
    }
}
