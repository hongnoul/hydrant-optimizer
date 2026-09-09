use std::{collections::BTreeSet, fs};

use chrono::NaiveDate;
use hydrant_optimizer::{adapter, calendar, model::*, storage};
use serde_json::json;
use tempfile::TempDir;

fn fixture_dataset() -> Dataset {
    adapter::parse_catalog(
        include_str!("fixtures/catalog.json"),
        include_str!("fixtures/term.json"),
    )
    .unwrap()
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn manual_option(id: &str, weekday: u8, start_minute: u16, end_minute: u16) -> SectionOption {
    SectionOption {
        id: id.to_string(),
        label: "Manual".to_string(),
        room: "Room M".to_string(),
        source: Source::Manual,
        meetings: vec![Meeting {
            weekday,
            start_minute,
            end_minute,
            start_date: None,
            end_date: None,
        }],
        incompatible_with: BTreeSet::new(),
        unsupported_reason: None,
    }
}

fn chosen(section: SectionOption) -> ChosenSection {
    ChosenSection {
        course_id: "A".to_string(),
        course_title: "Algorithms".to_string(),
        kind: "lecture".to_string(),
        section,
    }
}

#[test]
fn adapter_parses_fixture_calendar_blank_titles_partials_and_deduplicates_exact_raw_sections() {
    let catalog = r#"{
      "lastUpdated":"2026-09-08 22:01",
      "termInfo":{"urlName":"f26","startDate":"2026-10-26","endDate":"2026-11-09","holidayDates":["2026-11-02"],"mondayScheduleDate":"2026-11-03"},
      "classes":{
        "D":{"number":"D","name":"","half":false,"sectionKinds":["lecture"],"lectureSections":[[[[6,2]],"R1"],[[[6,2]],"R1"],[[[6,2]],"R2"]]},
        "P":{"number":"P","name":"Partial","half":1,"quarterInfo":{"end":[10,30]},"sectionKinds":["lecture"],"lectureSections":[[[[40,2]],"PR"]]},
        "U":{"number":"U","name":"Unknown","half":false,"sectionKinds":["recitation"],"recitationSections":[]}
      }
    }"#;
    let dataset = adapter::parse_catalog(catalog, r#"{"semester":{"urlName":"f26","startDate":"2026-10-26","endDate":"2026-11-09","holidayDates":["2026-11-02"],"mondayScheduleDate":"2026-11-03"}}"#).unwrap();
    let calendar = dataset.calendar.as_ref().unwrap();
    assert_eq!(dataset.term_id, "f26");
    assert_eq!(calendar.start, date("2026-10-26"));
    assert_eq!(calendar.holidays, BTreeSet::from([date("2026-11-02")]));
    assert_eq!(calendar.alternate_days.get(&date("2026-11-03")), Some(&0));

    let duplicate_course = &dataset.courses["D"];
    assert_eq!(duplicate_course.title, "D");
    let options = &duplicate_course.requirements[0].options;
    assert_eq!(
        options.len(),
        2,
        "exact duplicate raw sections should be collapsed, but different rooms retained"
    );
    assert_ne!(options[0].id, options[1].id);
    assert_eq!(options[0].meetings, options[1].meetings);
    assert_eq!(options[0].meetings[0].weekday, 0);
    assert_eq!(options[0].meetings[0].start_minute, 9 * 60);
    assert_eq!(options[0].meetings[0].end_minute, 10 * 60);

    let partial = &dataset.courses["P"].requirements[0].options[0];
    assert!(
        partial
            .unsupported_reason
            .as_deref()
            .unwrap()
            .contains("partial-term")
    );
    assert_eq!(partial.meetings[0].end_date, Some(date("2026-10-30")));
    assert!(dataset.courses["U"].requirements[0].has_unknown_times);
    assert!(
        dataset.courses["U"]
            .notices
            .iter()
            .any(|notice| notice.contains("no known"))
    );
}

#[test]
#[ignore = "requires f26 snapshot paths or HYDRANT_REAL_DATA_DIR with a fetched cache"]
fn real_f26_catalog_snapshot_parses_when_available() {
    let dataset = if let Some(dir) = std::env::var_os("HYDRANT_REAL_DATA_DIR") {
        storage::load_dataset(std::path::Path::new(&dir), true).unwrap()
    } else {
        let catalog_path =
            std::env::var_os("HYDRANT_REAL_CATALOG").expect("set HYDRANT_REAL_CATALOG");
        let term_path = std::env::var_os("HYDRANT_REAL_TERM").expect("set HYDRANT_REAL_TERM");
        adapter::parse_catalog(
            &fs::read_to_string(catalog_path).unwrap(),
            &fs::read_to_string(term_path).unwrap(),
        )
        .unwrap()
    };
    assert_eq!(dataset.term_id, "f26");
    assert!(dataset.courses.len() >= 2_000);
    assert_eq!(
        dataset.courses["SWE.010"].title, "SWE.010",
        "blank Hydrant titles fall back to the subject id"
    );
    assert!(
        dataset
            .calendar
            .as_ref()
            .unwrap()
            .holidays
            .contains(&date("2026-11-26"))
    );
    let duplicate = &dataset.courses["15.273"]
        .requirements
        .iter()
        .find(|r| r.kind == "lecture")
        .unwrap()
        .options;
    let unique_ids: BTreeSet<_> = duplicate.iter().map(|option| option.id.as_str()).collect();
    assert_eq!(
        unique_ids.len(),
        duplicate.len(),
        "API option IDs must be unique after exact duplicate collapse"
    );
}

#[test]
fn storage_parse_meetings_accepts_aliases_and_rejects_bad_ranges() {
    let meetings =
        storage::parse_meetings("Mon 09:00-10:00; th 13:05-14:10;Su 00:00-01:00").unwrap();
    assert_eq!(meetings.len(), 3);
    assert_eq!(meetings[0].weekday, 0);
    assert_eq!(meetings[1].weekday, 3);
    assert_eq!(meetings[1].start_minute, 13 * 60 + 5);
    assert_eq!(meetings[2].weekday, 6);
    assert!(storage::parse_meetings("Mon 10:00-09:00").is_err());
    assert!(storage::parse_meetings("Xday 09:00-10:00").is_err());
    assert!(storage::parse_meetings("Mon 24:00-25:00").is_err());
}

#[test]
fn storage_offline_cache_loads_validated_envelope_and_rejects_corruption() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let cache = json!({
        "version": 1,
        "fetched_at": "2026-09-08T22:01:00Z",
        "catalog_url": "https://hydrant.mit.edu/latest.json",
        "term_url": "https://hydrant.mit.edu/latestTerm.json",
        "catalog": include_str!("fixtures/catalog.json"),
        "term": include_str!("fixtures/term.json")
    });
    fs::write(
        dir.join("catalog-cache.json"),
        serde_json::to_vec_pretty(&cache).unwrap(),
    )
    .unwrap();
    let dataset = storage::load_dataset(dir, true).unwrap();
    assert_eq!(dataset.term_id, "f26");
    assert_eq!(dataset.courses.len(), 5);

    fs::write(dir.join("catalog-cache.json"), b"not json").unwrap();
    assert!(
        storage::load_dataset(dir, true)
            .unwrap_err()
            .to_string()
            .contains("offline cache")
    );

    let mut bad_url = cache;
    bad_url["catalog_url"] = json!("http://hydrant.mit.edu/latest.json");
    fs::write(
        dir.join("catalog-cache.json"),
        serde_json::to_vec_pretty(&bad_url).unwrap(),
    )
    .unwrap();
    assert!(storage::load_dataset(dir, true).is_err());
}

#[test]
fn storage_manual_round_trip_merge_scope_and_invalid_write_protection() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let base = fixture_dataset();
    assert!(
        base.courses["A"]
            .requirements
            .iter()
            .find(|r| r.kind == "recitation")
            .unwrap()
            .has_unknown_times
    );

    let enabled = ManualEntry {
        term_id: "f26".to_string(),
        course_id: "A".to_string(),
        kind: "recitation".to_string(),
        option: manual_option("manual-recitation", 0, 11 * 60, 12 * 60),
        enabled: true,
    };
    let mut disabled_other_term = enabled.clone();
    disabled_other_term.term_id = "sp27".to_string();
    disabled_other_term.option.id = "manual-other-term".to_string();
    disabled_other_term.enabled = false;
    let store = ManualStore {
        version: 1,
        entries: vec![enabled.clone(), disabled_other_term],
    };
    storage::save_manual(dir, &store).unwrap();
    let saved_text = fs::read_to_string(dir.join("manual.json")).unwrap();
    assert_eq!(storage::load_manual(dir).unwrap().entries.len(), 2);

    let merged = storage::apply_manual(&base, &store).unwrap();
    let recitation = merged.courses["A"]
        .requirements
        .iter()
        .find(|r| r.kind == "recitation")
        .unwrap();
    assert!(!recitation.has_unknown_times);
    assert_eq!(
        recitation
            .options
            .iter()
            .filter(|option| option.source == Source::Manual)
            .count(),
        1
    );
    assert!(
        !merged.courses["A"]
            .notices
            .iter()
            .any(|notice| notice == "recitation: no known meeting times")
    );

    let invalid = ManualStore {
        version: 2,
        entries: vec![enabled],
    };
    assert!(storage::save_manual(dir, &invalid).is_err());
    assert_eq!(
        fs::read_to_string(dir.join("manual.json")).unwrap(),
        saved_text,
        "invalid saves must not replace the existing manual store"
    );
}

#[test]
fn storage_manual_preserves_date_limited_entries_but_keeps_unknown_unresolved() {
    let base = fixture_dataset();
    let mut option = manual_option("manual-dated", 0, 11 * 60, 12 * 60);
    option.meetings[0].start_date = Some(date("2026-10-26"));
    option.meetings[0].end_date = Some(date("2026-10-30"));
    option.unsupported_reason =
        Some("date-limited manual section: partial-term optimization is not supported".to_string());
    let store = ManualStore {
        version: 1,
        entries: vec![ManualEntry {
            term_id: "f26".to_string(),
            course_id: "A".to_string(),
            kind: "recitation".to_string(),
            option,
            enabled: true,
        }],
    };
    let merged = storage::apply_manual(&base, &store).unwrap();
    let recitation = merged.courses["A"]
        .requirements
        .iter()
        .find(|r| r.kind == "recitation")
        .unwrap();
    assert!(recitation.has_unknown_times);
    assert_eq!(recitation.options.len(), 1);
    assert!(
        merged.courses["A"]
            .notices
            .iter()
            .any(|notice| notice.contains("date-limited manual"))
    );
}

#[test]
fn calendar_export_enumerates_actual_new_york_dates_holidays_alternates_dst_and_stable_ids() {
    let dataset = fixture_dataset();
    let calendar = dataset.calendar.as_ref().unwrap();
    let option = dataset.courses["A"]
        .requirements
        .iter()
        .find(|r| r.kind == "lecture")
        .unwrap()
        .options[0]
        .clone();
    let report = calendar::export_ics(calendar, &[chosen(option)]).unwrap();
    assert_eq!(report.event_count, 3);
    assert!(report.notices.is_empty());
    assert!(
        report.ics.contains("DTSTART:20261026T130000Z"),
        "Oct 26 is EDT"
    );
    assert!(
        report.ics.contains("DTSTART:20261103T140000Z"),
        "Nov 3 alternate Monday is EST after DST ends"
    );
    assert!(report.ics.contains("DTSTART:20261109T140000Z"));
    assert!(
        !report.ics.contains("DTSTART:20261102"),
        "holiday Monday should be skipped"
    );
    let parsed: icalendar::Calendar = report.ics.parse().unwrap();
    assert_eq!(parsed.events().count(), report.event_count);
    let again = calendar::export_ics(
        calendar,
        &[chosen(
            dataset.courses["A"].requirements[0].options[0].clone(),
        )],
    )
    .unwrap();
    assert_eq!(
        report.ics, again.ics,
        "calendar output should be deterministic"
    );
}

#[test]
fn calendar_export_reports_explicit_omissions() {
    let dataset = fixture_dataset();
    let calendar = dataset.calendar.as_ref().unwrap();
    let mut unsupported = manual_option("manual-unsupported", 0, 9 * 60, 10 * 60);
    unsupported.unsupported_reason = Some("partial-term optimization is not supported".to_string());
    let mut dated = manual_option("manual-dated", 2, 9 * 60, 10 * 60);
    dated.meetings[0].start_date = Some(date("2026-10-26"));
    dated.unsupported_reason = None;
    let empty = SectionOption {
        id: "manual-empty".to_string(),
        label: "Empty".to_string(),
        room: String::new(),
        source: Source::Manual,
        meetings: Vec::new(),
        incompatible_with: BTreeSet::new(),
        unsupported_reason: None,
    };
    let report = calendar::export_ics(
        calendar,
        &[chosen(unsupported), chosen(dated), chosen(empty)],
    )
    .unwrap();
    assert_eq!(report.event_count, 0);
    assert_eq!(report.notices.len(), 3);
    assert!(
        report
            .notices
            .iter()
            .any(|notice| notice.contains("partial-term"))
    );
    assert!(
        report
            .notices
            .iter()
            .any(|notice| notice.contains("date-limited"))
    );
    assert!(
        report
            .notices
            .iter()
            .any(|notice| notice.contains("no known"))
    );
}

#[test]
fn adapter_uses_official_half_dates_and_never_invents_midpoints() {
    let base = fixture_dataset();
    let partial = &base.courses["P"].requirements[0].options[0];
    assert_eq!(partial.meetings[0].start_date, Some(date("2026-10-26")));
    assert_eq!(partial.meetings[0].end_date, Some(date("2026-10-30")));
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    raw["termInfo"].as_object_mut().unwrap().remove("h1EndDate");
    let without_bounds = adapter::parse_catalog(&raw.to_string(), "").unwrap();
    let partial = &without_bounds.courses["P"].requirements[0].options[0];
    assert!(partial.meetings[0].end_date.is_none());
    assert!(partial.unsupported_reason.is_some());
    raw["classes"]["P"]["half"] = json!(2);
    let second_half = adapter::parse_catalog(&raw.to_string(), "").unwrap();
    let partial = &second_half.courses["P"].requirements[0].options[0];
    assert_eq!(partial.meetings[0].start_date, Some(date("2026-11-02")));
    assert_eq!(partial.meetings[0].end_date, Some(date("2026-11-09")));
}

#[test]
fn adapter_rejects_corrupt_metadata_and_integer_wraparound() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    for (pointer, bad) in [
        ("/classes/B/lectureSections/0/0/0/0", json!(34 * 256)),
        ("/classes/B/lectureSections/0/0/0/1", json!(i64::MAX)),
        ("/classes/B/lectureSections/0/0/0/0", json!(-1)),
        ("/classes/B/lectureSections/0/1", json!(7)),
        ("/classes/B/sectionKinds", json!("lecture")),
        ("/classes/B/lectureSections", json!({})),
        ("/termInfo/holidayDates", json!("2026-11-02")),
        ("/termInfo/startDate", json!("2027-01-01")),
        ("/termInfo/h1EndDate", json!(42)),
    ] {
        let mut corrupt = raw.clone();
        *corrupt.pointer_mut(pointer).unwrap() = bad;
        assert!(
            adapter::parse_catalog(&corrupt.to_string(), "").is_err(),
            "{pointer}"
        );
    }
    let mut mismatched: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/term.json")).unwrap();
    mismatched["semester"]["urlName"] = json!("sp27");
    assert!(adapter::parse_catalog(&raw.to_string(), &mismatched.to_string()).is_err());
    let mut conflicting = raw;
    conflicting["termInfo"]["tuesdayScheduleDate"] = json!("2026-11-03");
    assert!(adapter::parse_catalog(&conflicting.to_string(), "").is_err());
}

#[test]
fn storage_only_enabled_supported_manual_sections_resolve_unknowns() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    raw["classes"]["A"]["lectureSections"]
        .as_array_mut()
        .unwrap()
        .push(json!([[], "TBA"]));
    let base = adapter::parse_catalog(&raw.to_string(), "").unwrap();
    let unknown = |dataset: &Dataset| {
        dataset.courses["A"]
            .requirements
            .iter()
            .find(|r| r.kind == "lecture")
            .unwrap()
            .has_unknown_times
    };
    assert!(unknown(&base));
    assert!(unknown(
        &storage::apply_manual(&base, &ManualStore::default()).unwrap()
    ));
    let mut store = ManualStore {
        version: 1,
        entries: vec![ManualEntry {
            term_id: "f26".into(),
            course_id: "A".into(),
            kind: "lecture".into(),
            option: manual_option("manual-resolved", 1, 600, 660),
            enabled: false,
        }],
    };
    assert!(unknown(&storage::apply_manual(&base, &store).unwrap()));
    store.entries[0].enabled = true;
    store.entries[0].term_id = "sp27".into();
    assert!(unknown(&storage::apply_manual(&base, &store).unwrap()));
    store.entries[0].term_id = "f26".into();
    store.entries[0].option.unsupported_reason = Some("unsupported".into());
    assert!(unknown(&storage::apply_manual(&base, &store).unwrap()));
    store.entries[0].option.unsupported_reason = None;
    assert!(!unknown(&storage::apply_manual(&base, &store).unwrap()));
    assert!(unknown(&base), "merging never mutates raw API data");
}

#[test]
fn storage_rejects_invalid_manual_records_without_clobbering_valid_store() {
    let temp = TempDir::new().unwrap();
    assert!(
        storage::load_manual(temp.path())
            .unwrap()
            .entries
            .is_empty()
    );
    let valid = ManualStore {
        version: 1,
        entries: vec![ManualEntry {
            term_id: "f26".into(),
            course_id: "A".into(),
            kind: "lecture".into(),
            option: manual_option("manual-valid", 1, 600, 660),
            enabled: true,
        }],
    };
    storage::save_manual(temp.path(), &valid).unwrap();
    let path = temp.path().join("manual.json");
    let before = fs::read(&path).unwrap();
    for mutation in 0..8 {
        let mut store = valid.clone();
        match mutation {
            0 => store.entries[0].option.source = Source::Api,
            1 => store.entries[0].kind = " Lecture".into(),
            2 => store.entries[0].option.meetings[0].weekday = 7,
            3 => store.entries[0].option.meetings[0].end_minute = 599,
            4 => store.entries[0].option.id = "manual-".into(),
            5 => store.entries[0].option.meetings[0].start_date = Some(date("2026-10-27")),
            6 => {
                let mut duplicate = store.entries[0].clone();
                duplicate.course_id = "B".into();
                store.entries.push(duplicate);
            }
            7 => store.entries[0].option.meetings.clear(),
            _ => unreachable!(),
        }
        assert!(
            storage::save_manual(temp.path(), &store).is_err(),
            "mutation {mutation}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    fs::write(&path, "not json").unwrap();
    assert!(storage::load_manual(temp.path()).is_err());
}

#[test]
fn storage_meeting_parser_handles_midnight_adjacency_and_rejects_bad_syntax() {
    assert_eq!(
        storage::parse_meetings("Mon 23:00-24:00").unwrap()[0].end_minute,
        1440
    );
    assert_eq!(
        storage::parse_meetings("Mon 09:00-10:00;Mon 10:00-11:00")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        storage::parse_meetings("Mon 09:00-10:00;Mon 09:00-10:00")
            .unwrap()
            .len(),
        1
    );
    for text in [
        "",
        "Mon 24:01-24:02",
        "Mon 09:0-10:00",
        "Mon +9:00-10:00",
        "Mon 09:00-10:00;",
    ] {
        assert!(storage::parse_meetings(text).is_err(), "{text}");
    }
}

#[test]
fn storage_accepts_arbitrary_normalized_manual_kinds_without_dataset_context() {
    let temp = TempDir::new().unwrap();
    let valid = ManualStore {
        version: 1,
        entries: vec![ManualEntry {
            term_id: "f26".into(),
            course_id: "A".into(),
            kind: "seminar session".into(),
            option: manual_option("manual-seminar", 1, 600, 660),
            enabled: true,
        }],
    };
    storage::save_manual(temp.path(), &valid).unwrap();
    assert_eq!(
        storage::load_manual(temp.path()).unwrap().entries[0].kind,
        "seminar session"
    );

    for kind in ["", "seminar session ", "Seminar Session"] {
        let mut invalid = valid.clone();
        invalid.entries[0].kind = kind.into();
        assert!(
            storage::save_manual(temp.path(), &invalid).is_err(),
            "{kind:?}"
        );
    }
}

#[test]
fn storage_manual_validation_allows_overlaps_only_for_date_limited_entries_like_app() {
    let temp = TempDir::new().unwrap();
    let mut overlapping = manual_option("manual-overlap", 0, 9 * 60, 11 * 60);
    overlapping.meetings.push(Meeting {
        weekday: 0,
        start_minute: 10 * 60,
        end_minute: 12 * 60,
        start_date: Some(date("2026-11-02")),
        end_date: Some(date("2026-11-09")),
    });
    overlapping.meetings[0].start_date = Some(date("2026-10-26"));
    overlapping.meetings[0].end_date = Some(date("2026-10-30"));
    overlapping.unsupported_reason =
        Some("date-limited manual section: partial-term optimization is not supported".to_string());
    let valid = ManualStore {
        version: 1,
        entries: vec![ManualEntry {
            term_id: "f26".into(),
            course_id: "A".into(),
            kind: "lecture".into(),
            option: overlapping,
            enabled: true,
        }],
    };
    storage::save_manual(temp.path(), &valid).unwrap();

    let mut invalid = valid;
    for meeting in &mut invalid.entries[0].option.meetings {
        meeting.start_date = None;
        meeting.end_date = None;
    }
    invalid.entries[0].option.unsupported_reason = None;
    assert!(storage::save_manual(temp.path(), &invalid).is_err());
}

#[test]
fn storage_online_failure_falls_back_with_notice_age_and_preserves_cache() {
    // A refused proxy exercises the actual CLI/network fallback without reaching the internet.
    let temp = TempDir::new().unwrap();
    let cache = serde_json::to_vec(&json!({
        "version": 1, "fetched_at": "2026-09-08T22:01:00Z",
        "catalog_url": "https://hydrant.mit.edu/latest.json", "term_url": "https://hydrant.mit.edu/latestTerm.json",
        "catalog": include_str!("fixtures/catalog.json"), "term": include_str!("fixtures/term.json")
    })).unwrap();
    let path = temp.path().join("catalog-cache.json");
    assert!(storage::load_dataset(temp.path(), true).is_err());
    fs::write(&path, &cache).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hydrant-optimizer"))
        .arg("--data-dir")
        .arg(temp.path())
        .args(["search", "A"])
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("https_proxy", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("all_proxy", "http://127.0.0.1:1")
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let notices = String::from_utf8(output.stderr).unwrap();
    assert!(notices.contains("online refresh failed"), "{notices}");
    assert!(
        notices.contains("using cached catalog") && notices.contains("age "),
        "{notices}"
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Algorithms")
    );
    assert_eq!(fs::read(path).unwrap(), cache);
}

#[test]
fn calendar_folds_utf8_escapes_text_handles_midnight_and_duplicate_events() {
    let calendar = TermCalendar {
        start: date("2026-10-26"),
        end: date("2026-10-26"),
        holidays: Default::default(),
        alternate_days: Default::default(),
    };
    let mut option = manual_option("manual-utf8", 0, 1380, 1440);
    option.label = format!("{}\\;,\r\nBEGIN:INJECTED", "漢🙂".repeat(30));
    option.room = "Room;One,Two\\Three\rFour".into();
    let selected = chosen(option);
    let report = calendar::export_ics(&calendar, &[selected.clone(), selected]).unwrap();
    assert_eq!(report.event_count, 1);
    assert!(report.ics.contains("DTEND:20261027T040000Z"));
    assert!(report.ics.split("\r\n").all(|line| line.len() <= 75));
    let unfolded = report.ics.replace("\r\n ", "");
    assert!(unfolded.contains("\\\\\\;\\,\\nBEGIN:INJECTED"));
    assert!(!unfolded.contains("\r\nBEGIN:INJECTED"));
    assert!(unfolded.contains("LOCATION:Room\\;One\\,Two\\\\Three\\nFour"));
    let parsed: icalendar::Calendar = report.ics.parse().unwrap();
    assert_eq!(parsed.events().count(), 1);
}

#[test]
fn calendar_rejects_invalid_calendars_and_ambiguous_or_nonexistent_local_times() {
    let mut calendar = TermCalendar {
        start: date("2026-11-01"),
        end: date("2026-11-01"),
        holidays: Default::default(),
        alternate_days: Default::default(),
    };
    assert!(
        calendar::export_ics(
            &calendar,
            &[chosen(manual_option("manual-fall", 6, 90, 105))]
        )
        .is_err()
    );
    calendar.start = date("2026-03-08");
    calendar.end = calendar.start;
    assert!(
        calendar::export_ics(
            &calendar,
            &[chosen(manual_option("manual-spring", 6, 150, 165))]
        )
        .is_err()
    );
    calendar.end = date("2026-03-07");
    assert!(calendar::export_ics(&calendar, &[]).is_err());
    calendar.end = calendar.start;
    calendar.alternate_days.insert(calendar.start, 7);
    assert!(calendar::export_ics(&calendar, &[]).is_err());
}

fn chosen_for(course_id: &str, weekday: u8) -> ChosenSection {
    ChosenSection {
        course_id: course_id.to_string(),
        course_title: format!("{course_id} title"),
        kind: "lecture".to_string(),
        section: manual_option(&format!("manual-{course_id}"), weekday, 540, 600),
    }
}

#[test]
fn calendar_exports_meeting_details_without_color_metadata() {
    let calendar = TermCalendar {
        start: date("2026-10-26"),
        end: date("2026-10-26"),
        holidays: Default::default(),
        alternate_days: Default::default(),
    };
    let report =
        calendar::export_ics(&calendar, &[chosen_for("B", 0), chosen_for("A", 0)]).unwrap();
    assert_eq!(report.event_count, 2);
    let unfolded = report.ics.replace("\r\n ", "");
    for course_id in ["A", "B"] {
        assert!(unfolded.contains(&format!("SUMMARY:{course_id} Lec\r\n")));
        assert!(
            unfolded.contains(&format!(
                "DESCRIPTION:{course_id} title\\n{course_id} lecture Manual\r\n"
            )),
            "description must contain only course and section details:\n{unfolded}"
        );
    }
    assert_eq!(unfolded.matches("LOCATION:Room M\r\n").count(), 2);
    for removed in ["color", "categories:", "x-hydrant-", "google"] {
        assert!(
            !unfolded.to_ascii_lowercase().contains(removed),
            "unexpected export metadata {removed}:\n{unfolded}"
        );
    }
    assert!(report.ics.split("\r\n").all(|line| line.len() <= 75));
    let parsed: icalendar::Calendar = report.ics.parse().unwrap();
    assert_eq!(parsed.events().count(), report.event_count);
}

#[test]
fn calendar_always_emits_location_with_room_or_tba_like_hydrant() {
    let calendar = TermCalendar {
        start: date("2026-10-26"),
        end: date("2026-10-26"),
        holidays: Default::default(),
        alternate_days: Default::default(),
    };
    let mut with_room = chosen_for("B", 0);
    with_room.section.room = "34-304".to_string();
    let mut tba = chosen_for("A", 0);
    tba.course_id = "TBA-101".to_string();
    tba.section.room = "   ".to_string();
    // Same Monday slot: both export (overlap is allowed at export; the
    // optimizer prevents it). Distinct courses keep assertions unambiguous.
    let report = calendar::export_ics(&calendar, &[with_room, tba]).unwrap();
    assert_eq!(report.event_count, 2);
    let unfolded = report.ics.replace("\r\n ", "");
    assert!(
        unfolded.contains("LOCATION:34-304"),
        "room number must be in LOCATION:\n{unfolded}"
    );
    assert!(
        unfolded.contains("LOCATION:TBA"),
        "missing rooms must still emit LOCATION:TBA like Hydrant:\n{unfolded}"
    );
    assert_eq!(unfolded.matches("LOCATION:").count(), 2);
}
