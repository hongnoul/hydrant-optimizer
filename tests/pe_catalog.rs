use std::collections::BTreeMap;

use hydrant_optimizer::{
    adapter, app,
    model::SolveStatus,
    tui::{AppState, Focus},
};
use serde_json::{Value, json};

fn raw() -> Value {
    serde_json::from_str(include_str!("fixtures/catalog-pe.json")).unwrap()
}
fn parse(value: &Value) -> hydrant_optimizer::model::Dataset {
    adapter::parse_catalog(&value.to_string(), "").unwrap()
}

#[test]
fn pe_quarters_are_selectable_stable_courses_with_dated_sections() {
    let full = raw();
    let dataset = parse(&full);
    assert_eq!(dataset.courses.len(), 4);
    let first = &dataset.courses["PE.1000.Q1"];
    assert!(first.title.contains("Swimming") && first.title.contains("Q1"));
    assert_eq!(first.requirements[0].kind, "pe");
    assert_eq!(first.requirements[0].options.len(), 2);
    for option in &first.requirements[0].options {
        assert!(option.unsupported_reason.is_none());
        assert!(option.label.contains("2026-10-26 to 2026-10-30"));
        assert_eq!(
            option.meetings[0].start_date.unwrap().to_string(),
            "2026-10-26"
        );
        assert_eq!(
            option.meetings[0].end_date.unwrap().to_string(),
            "2026-10-30"
        );
    }
    let mut earlier = full.clone();
    earlier["pe"].as_object_mut().unwrap().remove("2");
    let earlier = parse(&earlier);
    assert_eq!(
        earlier.courses["PE.1000.Q1"].requirements[0].options,
        first.requirements[0].options
    );
    let mut reordered = full;
    for field in ["sections", "sectionNumbers", "rawSections"] {
        reordered["pe"]["1"]["PE.1000"][field]
            .as_array_mut()
            .unwrap()
            .reverse();
    }
    assert_eq!(
        parse(&reordered).courses["PE.1000.Q1"].requirements[0].options,
        first.requirements[0].options
    );
}

#[test]
fn pe_unannounced_or_missing_dates_are_disclosed_not_invented() {
    let mut data = raw();
    data["pe"]["1"]["PE.1000"]
        .as_object_mut()
        .unwrap()
        .remove("startDate");
    let dataset = parse(&data);
    let trip = &dataset.courses["PE.2000.Q1"];
    assert!(trip.requirements[0].has_unknown_times);
    assert!(
        trip.notices
            .iter()
            .any(|notice| notice.contains("not machine-readable"))
    );
    assert!(
        dataset.courses["PE.1000.Q1"].requirements[0]
            .options
            .iter()
            .all(|option| option
                .unsupported_reason
                .as_ref()
                .unwrap()
                .contains("dates"))
    );
    let solution =
        app::optimize(&dataset, &["PE.1000.Q1".into(), "PE.2000.Q1".into()], None).unwrap();
    assert!(solution.choices.is_empty());
    assert!(!solution.unresolved.is_empty());
}

#[test]
fn pe_iap_and_malformed_catalog_boundaries_are_checked() {
    let mut iap = raw();
    let mut course = iap["pe"]["1"]["PE.1000"].clone();
    course["quarter"] = json!(5);
    iap["pe"] = json!({"5":{"PE.1000":course}});
    assert!(parse(&iap).courses["PE.1000.Q5"].title.contains("IAP"));
    for invalid in [json!([]), json!({"6":{}}), json!({"1":[]})] {
        let mut data = raw();
        data["pe"] = invalid;
        assert!(adapter::parse_catalog(&data.to_string(), "").is_err());
    }
    for (field, value) in [
        ("quarter", json!(2)),
        ("startDate", json!("bad")),
        ("endDate", json!("2026-01-01")),
        ("sectionNumbers", json!(["1"])),
        ("sections", json!([[[[999999, 2]], "Gym"]])),
    ] {
        let mut data = raw();
        data["pe"]["1"]["PE.1000"][field] = value;
        assert!(
            adapter::parse_catalog(&data.to_string(), "").is_err(),
            "accepted invalid {field}"
        );
    }
}

#[test]
fn pe_catalog_optimizes_with_academics_and_exports_only_offering_dates() {
    let dataset = parse(&raw());
    let solution = app::optimize(
        &dataset,
        &["A".into(), "PE.1000.Q1".into(), "PE.1000.Q2".into()],
        None,
    )
    .unwrap();
    assert_eq!(solution.status, SolveStatus::OptimalKnown);
    assert_eq!(solution.choices.len(), 5);
    let sections = app::actual_sections(&dataset, &solution, &BTreeMap::new()).unwrap();
    let pe: Vec<_> = sections
        .iter()
        .filter(|section| section.kind == "pe")
        .collect();
    assert_eq!(pe.len(), 2);
    assert!(
        pe.iter()
            .all(|section| section.section.meetings[0].start_minute == 660)
    );
    let temp = tempfile::tempdir().unwrap();
    let hint = temp.path().join("pe.ics");
    let report = app::write_calendar(&dataset, &solution, &BTreeMap::new(), &hint).unwrap();
    assert!(
        report
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("pe-"),
        "export must carry a Unix-time signature: {}",
        report.path.display()
    );
    assert!(!hint.exists(), "the bare hint must never be written");
    assert_eq!(report.event_count, 9); // 6 academic + Q1 Oct26 + Q2 Nov3/Nov9.
    assert!(report.ics.contains("PE.1000.Q1 pe") && report.ics.contains("PE.1000.Q2 pe"));
    let pe_events: Vec<_> = report
        .ics
        .split("BEGIN:VEVENT")
        .skip(1)
        .filter(|event| event.contains("SUMMARY:PE."))
        .collect();
    assert_eq!(pe_events.len(), 3);
    for event in pe_events {
        if event.contains("PE.1000.Q1") {
            assert!(event.contains("DTSTART:20261026T150000Z"));
        } else {
            assert!(
                event.contains("DTSTART:20261103T160000Z")
                    || event.contains("DTSTART:20261109T160000Z")
            );
        }
    }
}

#[test]
fn pe_search_and_manual_editor_prefill_preserve_published_dates() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        parse(&raw()),
        Default::default(),
        temp.path().into(),
        temp.path().join("out.ics"),
        vec!["A".into(), "PE.1000.Q1".into()],
    )
    .unwrap();
    state.query = "swimming".into();
    state.recompute_filter();
    assert_eq!(state.filtered, ["PE.1000.Q1", "PE.1000.Q2"]);
    state.focus = Focus::Subjects;
    state.open_add_manual();
    let form = state.editor.unwrap();
    assert_eq!(form.kind, "pe");
    assert_eq!(form.start_date, "2026-10-26");
    assert_eq!(form.end_date, "2026-10-30");
}
