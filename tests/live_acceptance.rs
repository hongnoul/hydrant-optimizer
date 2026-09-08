//! Opt-in real public-service / actual-executable acceptance, not a mock server.
//! Run: cargo test --test live_acceptance -- --ignored --nocapture
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn invoke(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hydrant-optimizer"))
        .args(["--data-dir", dir.to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap()
}
fn checked(dir: &Path, args: &[&str]) -> Value {
    let output = invoke(dir, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
#[ignore = "requires live public Hydrant HTTPS access"]
fn live_catalog_manual_refresh_optimize_switch_and_export() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let fresh = checked(dir, &["refresh"]);
    assert!(fresh["courses"].as_u64().unwrap() > 0);
    println!(
        "LIVE_FETCH term={} courses={}",
        fresh["term"], fresh["courses"]
    );
    let found = checked(dir, &["--offline", "search", "6.1200"]);
    let subject = found
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "6.1200")
        .expect("live test subject unavailable in this term");
    assert!(
        subject["title"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("mathematics")
    );
    let before = checked(dir, &["--offline", "optimize", "6.1200"]);
    assert_eq!(before["solution"]["status"], "optimal_known");
    let selected = &before["sections"][0];
    let kind = selected["kind"].as_str().unwrap();
    let meetings = selected["section"]["meetings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let day = m["weekday"].as_u64().unwrap() as usize;
            let start = m["start_minute"].as_u64().unwrap();
            let end = m["end_minute"].as_u64().unwrap();
            format!(
                "{} {:02}:{:02}-{:02}:{:02}",
                ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][day],
                start / 60,
                start % 60,
                end / 60,
                end % 60
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let saved = checked(
        dir,
        &[
            "--offline",
            "manual",
            "add",
            "--course",
            "6.1200",
            "--kind",
            kind,
            "--label",
            "Local acceptance alternative",
            "--room",
            "LOCAL TEST ROOM",
            "--meetings",
            &meetings,
        ],
    );
    let id = saved["id"].as_str().unwrap();
    let manual_before = fs::read(dir.join("manual.json")).unwrap();
    checked(dir, &["refresh"]);
    assert_eq!(fs::read(dir.join("manual.json")).unwrap(), manual_before);
    let cache_before = fs::read(dir.join("catalog-cache.json")).unwrap();
    // Only this child gets an unreachable proxy. Exercise the real HTTP failure path,
    // without changing the user's network settings or substituting a fake server.
    let blocked = |path: &Path, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_hydrant-optimizer"))
            .args(["--data-dir", path.to_str().unwrap(), "--json"])
            .args(args)
            .env("HTTPS_PROXY", "http://127.0.0.1:0")
            .env("https_proxy", "http://127.0.0.1:0")
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .output()
            .unwrap()
    };
    let fallback = blocked(dir, &["refresh"]);
    assert!(
        fallback.status.success(),
        "{}",
        String::from_utf8_lossy(&fallback.stderr)
    );
    let fallback: Value = serde_json::from_slice(&fallback.stdout).unwrap();
    assert!(
        fallback["notices"]
            .to_string()
            .contains("using cached catalog")
    );
    assert!(fallback["notices"].to_string().contains("age"));
    let search = blocked(dir, &["search", "6.1200"]);
    assert!(search.status.success());
    assert!(String::from_utf8_lossy(&search.stderr).contains("using cached catalog"));
    assert!(
        serde_json::from_slice::<Value>(&search.stdout)
            .unwrap()
            .is_array()
    );
    assert_eq!(
        fs::read(dir.join("catalog-cache.json")).unwrap(),
        cache_before
    );
    assert_eq!(fs::read(dir.join("manual.json")).unwrap(), manual_before);
    let empty = TempDir::new().unwrap();
    assert!(!blocked(empty.path(), &["refresh"]).status.success());
    assert!(!empty.path().join("catalog-cache.json").exists());
    let after = checked(dir, &["--offline", "optimize", "6.1200"]);
    assert_eq!(before["solution"]["score"], after["solution"]["score"]);
    let mapping = format!("6.1200/{kind}={id}");
    let output = dir.join("live.ics");
    let exported = checked(
        dir,
        &[
            "--offline",
            "--output",
            output.to_str().unwrap(),
            "optimize",
            "6.1200",
            "--member",
            &mapping,
            "--export",
        ],
    );
    assert_eq!(exported["solution"]["score"], before["solution"]["score"]);
    assert!(
        exported["sections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["section"]["room"] == "LOCAL TEST ROOM")
    );
    let text = fs::read_to_string(&output).unwrap();
    let parsed: icalendar::Calendar = text.parse().unwrap();
    let count = parsed.events().count();
    assert_eq!(
        count,
        exported["export"]["events"].as_u64().unwrap() as usize
    );
    assert!(count > 0);
    assert!(text.contains("LOCAL TEST ROOM"));
    assert!(
        !invoke(
            dir,
            &[
                "--offline",
                "--output",
                output.to_str().unwrap(),
                "optimize",
                "6.1200",
                "--export"
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read_to_string(&output).unwrap(), text);
    println!(
        "LIVE_ACCEPTANCE score={} events={count} manual_retained=true member_switched=true offline=true fallback_with_age=true failed_refresh_preserved=true no_clobber=true",
        exported["solution"]["score"]
    );
}
