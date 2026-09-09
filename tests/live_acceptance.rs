//! Opt-in real public-service / actual-executable acceptance, not a mock server.
//! Run: cargo test --test live_acceptance -- --ignored --nocapture
mod support;
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
    let hint = dir.join("live.ics");
    let exported = checked(
        dir,
        &[
            "--offline",
            "--output",
            hint.to_str().unwrap(),
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
    let output = std::path::Path::new(exported["export"]["path"].as_str().unwrap()).to_path_buf();
    assert!(!hint.exists(), "the bare hint must never be written");
    let text = fs::read_to_string(&output).unwrap();
    let parsed: icalendar::Calendar = text.parse().unwrap();
    assert!(parsed.events().count() > 0);
    let count = support::expand_calendar(&text)
        .matches("BEGIN:VEVENT")
        .count();
    assert_eq!(
        count,
        exported["export"]["events"].as_u64().unwrap() as usize
    );
    assert!(count > 0);
    assert!(text.contains("LOCAL TEST ROOM"));
    // A repeat export mints a fresh Unix-time file, never overwriting.
    let again = checked(
        dir,
        &[
            "--offline",
            "--output",
            hint.to_str().unwrap(),
            "optimize",
            "6.1200",
            "--export",
        ],
    );
    let second = std::path::Path::new(again["export"]["path"].as_str().unwrap()).to_path_buf();
    assert_ne!(second, output);
    assert_eq!(fs::read_to_string(&output).unwrap(), text);

    let pe_courses = checked(dir, &["--offline", "search", "PE."]);
    let pe_courses = pe_courses.as_array().unwrap();
    assert!(
        !pe_courses.is_empty(),
        "live catalog has no PE offerings to validate"
    );
    let (pe_id, pe_solved) = pe_courses
        .iter()
        .find_map(|course| {
            let id = course["id"].as_str()?;
            if !id.starts_with("PE.") {
                return None;
            }
            let solved = checked(dir, &["--offline", "optimize", id]);
            (solved["solution"]["choices"].as_array()?.len() == 1).then_some((id, solved))
        })
        .expect("live PE feed has no supported dated offering");
    let pe_meetings = pe_solved["sections"][0]["section"]["meetings"]
        .as_array()
        .unwrap();
    assert!(!pe_meetings.is_empty());
    let start = chrono::NaiveDate::parse_from_str(
        pe_meetings[0]["start_date"].as_str().unwrap(),
        "%Y-%m-%d",
    )
    .unwrap();
    let end =
        chrono::NaiveDate::parse_from_str(pe_meetings[0]["end_date"].as_str().unwrap(), "%Y-%m-%d")
            .unwrap();
    let pe_hint = dir.join("live-pe.ics");
    let pe_export = checked(
        dir,
        &[
            "--offline",
            "--output",
            pe_hint.to_str().unwrap(),
            "optimize",
            pe_id,
            "--export",
        ],
    );
    let pe_output =
        std::path::Path::new(pe_export["export"]["path"].as_str().unwrap()).to_path_buf();
    assert!(!pe_hint.exists(), "the bare hint must never be written");
    let pe_text = fs::read_to_string(&pe_output).unwrap();
    let pe_calendar: icalendar::Calendar = pe_text.parse().unwrap();
    assert!(pe_calendar.events().count() > 0);
    let pe_text = support::expand_calendar(&pe_text);
    let pe_count = pe_text.matches("BEGIN:VEVENT").count();
    assert!(pe_count > 0);
    assert_eq!(
        pe_count,
        pe_export["export"]["events"].as_u64().unwrap() as usize
    );
    for stamp in pe_text
        .lines()
        .filter_map(|line| line.strip_prefix("DTSTART:"))
    {
        let local = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ")
            .unwrap()
            .and_utc()
            .with_timezone(&chrono_tz::America::New_York);
        assert!(
            (start..=end).contains(&local.date_naive()),
            "PE escaped its published offering range: {stamp}"
        );
    }
    assert!(pe_text.contains(&format!("SUMMARY:{pe_id} PE")));
    println!(
        "LIVE_PE_ACCEPTANCE offerings={} selected={pe_id} events={pe_count} date_bounds={start}..{end} bounded_export=true",
        pe_courses.len()
    );
    println!(
        "LIVE_ACCEPTANCE score={} events={count} manual_retained=true member_switched=true offline=true fallback_with_age=true failed_refresh_preserved=true timestamped_exports=true",
        exported["solution"]["score"]
    );
}
