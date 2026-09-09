//! Actual executable integration tests. Synthetic fixtures supplement live acceptance.
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn invoke(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hydrant-optimizer"))
        .args([
            "--data-dir",
            dir.to_str().unwrap(),
            "--catalog",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json"),
            "--term",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
        ])
        .args(args)
        .output()
        .unwrap()
}
fn json_ok(dir: &Path, args: &[&str]) -> Value {
    let mut args = args.to_vec();
    args.push("--json");
    let output = invoke(dir, &args);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn actual_cli_selection_manual_entry_optimization_switch_and_export() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let found = json_ok(dir, &["search", "algo"]);
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(found[0]["id"], "A");
    let before = json_ok(dir, &["optimize", "A", "B"]);
    assert_eq!(before["solution"]["status"], "optimal_known");
    assert!(
        before["solution"]["unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("recitation"))
    );

    let saved = json_ok(
        dir,
        &[
            "manual",
            "add",
            "--course",
            "A",
            "--kind",
            "recitation",
            "--label",
            "Study; group, α",
            "--room",
            "Room Z",
            "--meetings",
            "Mon 11:00-12:00",
        ],
    );
    let id = saved["id"].as_str().unwrap();
    assert!(id.starts_with("manual-"));
    let after = json_ok(dir, &["optimize", "A", "B"]);
    assert_eq!(after["solution"]["score"]["occupied_days"], 1);
    assert_eq!(after["solution"]["score"]["gap_minutes"], 0);
    assert_eq!(after["sections"].as_array().unwrap().len(), 3);
    let group = after["solution"]["choices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["requirement_id"] == "A/lecture")
        .unwrap();
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
    let member = group["members"][1]["id"].as_str().unwrap();
    let mapping = format!("A/lecture={member}");
    let hint = dir.join("schedule.ics");
    let exported = json_ok(
        dir,
        &[
            "--output",
            hint.to_str().unwrap(),
            "optimize",
            "A",
            "B",
            "--member",
            &mapping,
            "--export",
        ],
    );
    assert_eq!(exported["solution"]["score"], after["solution"]["score"]);
    assert!(
        exported["sections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["section"]["id"] == member)
    );
    let path = Path::new(exported["export"]["path"].as_str().unwrap()).to_path_buf();
    assert_eq!(path.parent().unwrap(), dir);
    assert!(
        path.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("schedule-"),
        "export must carry a Unix-time signature: {}",
        path.display()
    );
    assert_eq!(path.extension().unwrap(), "ics");
    assert!(!hint.exists(), "the bare hint must never be written");
    let text = fs::read_to_string(&path).unwrap();
    let calendar: icalendar::Calendar = text.parse().unwrap();
    assert!(calendar.events().count() > 0);
    assert!(text.contains("DTSTART:20261026T130000Z"));
    assert!(text.contains("DTSTART:20261103T140000Z"));
    assert!(!text.contains("DTSTART:20261102"));
    // A repeat export must mint a fresh Unix-time file, never overwrite.
    let again = json_ok(
        dir,
        &[
            "--output",
            hint.to_str().unwrap(),
            "optimize",
            "A",
            "B",
            "--export",
        ],
    );
    let second = Path::new(again["export"]["path"].as_str().unwrap()).to_path_buf();
    assert_ne!(second, path);
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert!(
        fs::read_to_string(&second)
            .unwrap()
            .contains("DTSTART:20261026T130000Z")
    );

    json_ok(dir, &["manual", "disable", id]);
    let disabled = json_ok(dir, &["optimize", "A", "B"]);
    assert_eq!(disabled["sections"].as_array().unwrap().len(), 2);
    json_ok(
        dir,
        &[
            "manual",
            "add",
            "--id",
            id,
            "--course",
            "A",
            "--kind",
            "recitation",
            "--label",
            "Revised",
            "--meetings",
            "Mon 11:05-12:00",
        ],
    );
    assert_eq!(
        json_ok(dir, &["manual", "list"])["entries"][0]["enabled"],
        false
    );
    assert_eq!(
        json_ok(dir, &["optimize", "A", "B"])["sections"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let retained = fs::read(dir.join("manual.json")).unwrap();
    for (course, kind) in [("B", "recitation"), ("A", "lab")] {
        let output = invoke(
            dir,
            &[
                "manual",
                "add",
                "--id",
                id,
                "--course",
                course,
                "--kind",
                kind,
                "--meetings",
                "Mon 11:05-12:00",
            ],
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot change"));
        assert_eq!(fs::read(dir.join("manual.json")).unwrap(), retained);
    }
    json_ok(dir, &["manual", "enable", id]);
    let revised = json_ok(dir, &["optimize", "A", "B"]);
    assert_eq!(revised["solution"]["score"]["gap_minutes"], 5);
    assert_eq!(
        json_ok(dir, &["manual", "list"])["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn actual_cli_failure_statuses_and_unknown_time_disclosures() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let conflict = invoke(dir, &["--json", "optimize", "A", "C"]);
    assert_eq!(conflict.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&conflict.stdout).unwrap();
    assert_eq!(v["solution"]["status"], "infeasible");
    assert!(!invoke(dir, &["optimize", "NOT-A-SUBJECT"]).status.success());
    assert!(
        !invoke(
            dir,
            &[
                "manual",
                "add",
                "--course",
                "A",
                "--kind",
                "lab",
                "--meetings",
                "Mon 12:00-11:00"
            ]
        )
        .status
        .success()
    );
    assert!(
        !invoke(dir, &["optimize", "A", "--member", "A/lecture=wrong"])
            .status
            .success()
    );
    let partial = json_ok(dir, &["optimize", "P", "E"]);
    assert_eq!(partial["solution"]["score"]["occupied_days"], 0);
    assert!(
        !partial["solution"]["unresolved"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let no_cache = Command::new(env!("CARGO_BIN_EXE_hydrant-optimizer"))
        .args([
            "--data-dir",
            dir.to_str().unwrap(),
            "--offline",
            "search",
            "A",
        ])
        .output()
        .unwrap();
    assert!(!no_cache.status.success());
    fs::write(dir.join("manual.json"), b"malformed").unwrap();
    assert!(!invoke(dir, &["manual", "list"]).status.success());
}

#[test]
fn infeasible_export_preserves_status_and_never_touches_output() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("infeasible.ics");
    for existing in [false, true] {
        if existing {
            fs::write(&path, "existing calendar must survive").unwrap();
        }
        for json in [false, true] {
            let mut args = vec![
                "--output",
                path.to_str().unwrap(),
                "optimize",
                "A",
                "C",
                "--export",
            ];
            if json {
                args.push("--json");
            }
            let output = invoke(temp.path(), &args);
            assert_eq!(output.status.code(), Some(2), "{:?}", output);
            if json {
                let result: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(result["solution"]["status"], "infeasible");
                assert!(result["solution"]["score"].is_null());
                assert!(result["sections"].as_array().unwrap().is_empty());
                assert!(result["export"].is_null());
                assert!(
                    !result["solution"]["unresolved"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            } else {
                assert!(String::from_utf8_lossy(&output.stdout).contains("Infeasible:"));
                assert!(String::from_utf8_lossy(&output.stderr).contains("recitation"));
            }
            if existing {
                assert_eq!(
                    fs::read_to_string(&path).unwrap(),
                    "existing calendar must survive"
                );
            } else {
                assert!(!path.exists());
            }
        }
    }
}

#[test]
fn dated_manual_options_remain_visible_but_do_not_enter_weekly_search() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    json_ok(
        dir,
        &[
            "manual",
            "add",
            "--course",
            "A",
            "--kind",
            "recitation",
            "--meetings",
            "Mon 11:00-12:00",
            "--start-date",
            "2026-10-26",
            "--end-date",
            "2026-10-30",
        ],
    );
    let output = json_ok(dir, &["optimize", "A"]);
    assert_eq!(output["sections"].as_array().unwrap().len(), 1);
    assert!(
        output["solution"]["unresolved"]
            .to_string()
            .contains("date")
    );
    let list = json_ok(dir, &["manual", "list"]);
    assert_eq!(
        list["entries"][0]["option"]["meetings"][0]["start_date"],
        "2026-10-26"
    );
}
