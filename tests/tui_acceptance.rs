//! Native-terminal acceptance of the actual executable. Offline cases run by default.
//! Live case: cargo test --test tui_acceptance -- --ignored --nocapture
#![cfg(unix)]

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Driver {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    screen: vt100::Parser,
    raw: Vec<u8>,
}
impl Driver {
    fn new(dir: &Path, output: &Path) -> Self {
        Self::with_size(dir, output, 55, 170, &[])
    }

    fn with_size(dir: &Path, output: &Path, rows: u16, cols: u16, args: &[&str]) -> Self {
        Self::with_binary(
            env!("CARGO_BIN_EXE_hydrant-optimizer"),
            dir,
            output,
            rows,
            cols,
            args,
        )
    }

    fn with_binary(
        binary: &str,
        dir: &Path,
        output: &Path,
        rows: u16,
        cols: u16,
        args: &[&str],
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        // Inspect the real slave's termios before and after the app exits.
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "before=$(stty -g); \"$@\"; code=$?; after=$(stty -g); if [ \"$before\" = \"$after\" ]; then printf '\nTERMINAL_RESTORED\n'; else printf '\nTERMINAL_NOT_RESTORED\n'; exit 99; fi; exit \"$code\"", "--",binary,"--offline","--data-dir",dir.to_str().unwrap(),"--output",output.to_str().unwrap()]);
        cmd.args(args);
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 || send.send(bytes[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            _master: pair.master,
            writer,
            output: receive,
            screen: vt100::Parser::new(rows, cols, 1000),
            raw: Vec::new(),
        }
    }
    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).unwrap();
        self.writer.flush().unwrap();
    }
    fn until(&mut self, label: &str, condition: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let screen = self.screen.screen().contents();
            if condition(&screen) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {label}\n{screen}"
            );
            match self.output.recv_timeout(Duration::from_millis(100)) {
                Ok(bytes) => {
                    self.screen.process(&bytes);
                    self.raw.extend(bytes);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("terminal ended before {label}: {e}\n{screen}"),
            }
        }
    }
    fn marker(&mut self, marker: &str) {
        self.until(marker, |screen| screen.contains(marker));
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
    }
}

fn cli(dir: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_hydrant-optimizer"))
        .args(["--data-dir", dir.to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn week_row(screen: &str, time: &str) -> Option<Vec<String>> {
    screen
        .lines()
        .find(|line| line.split('│').any(|cell| cell.trim() == time))
        .map(|line| {
            line.split('│')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect()
        })
}

#[test]
fn actual_tui_week_replay_preserves_exact_times_and_records_observations() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let catalog_path = dir.join("week.json");
    let mut catalog: Value = serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    catalog["classes"] = serde_json::json!({
        "W": {"number":"W", "name":"Week replay", "sectionKinds":["lecture"], "lectureSections":[]},
        "X": {"number":"X", "name":"Shared bucket", "sectionKinds":["lecture"], "lectureSections":[]}
    });
    fs::write(&catalog_path, catalog.to_string()).unwrap();
    let term_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json");
    let source = [
        "--catalog",
        catalog_path.to_str().unwrap(),
        "--term",
        term_path,
    ];
    let meetings = "Mon 09:05-09:20;Tue 09:35-10:05;Wed 10:00-10:30;Thu 10:30-11:00;Fri 11:00-11:30;Sat 11:30-12:00;Sun 23:35-24:00";
    // Enter the exact-minute fixture through the actual CLI, not AppState helpers.
    for (id, label, room, times) in [
        ("W", "Week one", "Week room one", meetings),
        ("W", "Week two", "Week room two", meetings),
        ("X", "Short meeting", "Bucket room", "Mon 09:20-09:25"),
    ] {
        let mut args = source.to_vec();
        args.extend([
            "manual",
            "add",
            "--course",
            id,
            "--kind",
            "lecture",
            "--label",
            label,
            "--room",
            room,
            "--meetings",
            times,
        ]);
        cli(dir, &args);
    }
    let mut reference_args = source.to_vec();
    reference_args.extend(["optimize", "W", "X"]);
    let reference = cli(dir, &reference_args);
    assert_eq!(
        reference["solution"]["score"],
        serde_json::json!({"occupied_days":7,"gap_minutes":0})
    );
    let next_room = reference["solution"]["choices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|choice| choice["requirement_id"] == "W/lecture")
        .unwrap()["members"][1]["room"]
        .as_str()
        .unwrap();
    let mut args = source.to_vec();
    args.extend(["tui", "--select", "W", "--select", "X"]);
    let output = dir.join("week.ics");
    let mut ui = Driver::with_size(dir, &output, 72, 120, &args);
    ui.marker("Preselected 2 subject(s)");
    ui.send(b"/week\r");
    ui.until("query visible in top search bar", |s| {
        let header = s.lines().take(3).collect::<String>();
        header.contains("Search subjects") && header.contains("week") && s.contains("selected 2")
    });
    let header = ui
        .screen
        .screen()
        .contents()
        .lines()
        .take(3)
        .collect::<String>();
    assert!(!header.contains("Status"));
    assert!(header.contains("f26"));
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({"requirement":"top_search", "query":"week", "in_top_three_rows":true, "old_status_header":false, "selection_retained":2})
    );
    ui.send(b"o");
    ui.marker("Optimal: 7 occupied day(s), 0 gap minute(s).");
    ui.send(b"r");
    ui.until("full seven-day grid through midnight", |s| {
        s.contains("> Results") && s.contains("PgUp/Dn |") && week_row(s, "23:30").is_some()
    });
    let screen = ui.screen.screen().contents();
    let headers = week_row(&screen, "Time").unwrap();
    assert_eq!(
        headers,
        ["Time", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    );
    let row_times: Vec<_> = (540..1440)
        .step_by(30)
        .map(|minute| format!("{:02}:{:02}", minute / 60, minute % 60))
        .collect();
    let observed_times: Vec<_> = screen
        .lines()
        .filter_map(|line| line.split('│').nth(2).map(str::trim))
        .filter(|cell| cell.len() == 5 && chrono::NaiveTime::parse_from_str(cell, "%H:%M").is_ok())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        observed_times, row_times,
        "the actual terminal must contain exactly the expected 30-minute rows"
    );
    let first = week_row(&screen, "09:00").unwrap();
    assert!(first[1].contains("2×") && first[1].contains('W') && first[1].contains('X'));
    assert!(first[2..].iter().all(|cell| cell == "·"));
    for (time, day) in [
        ("09:30", 2),
        ("10:00", 2),
        ("10:00", 3),
        ("10:30", 4),
        ("11:00", 5),
        ("11:30", 6),
        ("23:30", 7),
    ] {
        assert_eq!(
            week_row(&screen, time).unwrap()[day],
            "W",
            "wrong day/slot {time}/{day}"
        );
    }
    assert_eq!(
        week_row(&screen, "10:30").unwrap()[2],
        "·",
        "end boundary must not occupy another bucket"
    );
    assert!(screen.contains("2× means multiple meetings"));
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({"requirement":"weekly_grid", "headers":headers, "visible_half_hour_rows":observed_times.len(), "first_row":observed_times.first(), "last_row":observed_times.last(), "monday_shared_bucket":first[1], "all_seven_days_placed":true, "adjacent_meetings_not_conflicts":true})
    );

    // Page into details and compare the user-visible exact times with the fixture.
    ui.send(b"\x1b[F");
    ui.until("exact labels and rooms", |s| {
        s.contains("Sun 23:35-24:00")
            && s.contains("Mon 09:05-09:20")
            && s.contains("Tue 09:35-10:05")
            && s.contains("Bucket room")
    });
    ui.send(b"n");
    ui.until("actual member switched", |s| {
        s.contains("(2/2)") && s.contains(next_room)
    });
    ui.send(b"e");
    ui.marker("Exported");
    let calendar = fs::read_to_string(&output).unwrap();
    let parsed: icalendar::Calendar = calendar.parse().unwrap();
    assert_eq!(parsed.events().count(), 17);
    for exact in [
        "DTSTART:20261026T130500Z",
        "DTEND:20261026T132000Z",
        "DTSTART:20261026T132000Z",
        "DTEND:20261026T132500Z",
        "DTSTART:20261109T043500Z",
        "DTEND:20261109T050000Z",
    ] {
        assert!(
            calendar.contains(exact),
            "display bucketing must not change {exact}"
        );
    }
    assert!(calendar.contains(&format!("LOCATION:{next_room}")));
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({"requirement":"exact_details_and_export", "displayed_exact_times":["Mon 09:05-09:20","Tue 09:35-10:05","Sun 23:35-24:00"], "switched_room":next_room, "events":parsed.events().count(), "exact_utc_boundaries_preserved":true})
    );
    // The same l+Enter sequence used against the baseline must edit Manual, not deselect W.
    ui.send(b"\tl\r");
    ui.marker("Edit manual entry");
    assert!(ui.screen.screen().contents().contains("selected 2"));
    ui.send(b"\x1b");
    ui.until("editor closed", |s| !s.contains("Edit manual entry"));
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({"requirement":"horizontal_navigation", "l_then_enter":"edits Manual entry", "selected_subjects_after":2, "terminal_restored":true})
    );

    // Optional comparative replay uses an independently built pre-refresh executable.
    // The main acceptance above always runs, even without that historical artifact.
    if let Ok(binary) = std::env::var("HYDRANT_UX_BASELINE_BIN") {
        let mut old = Driver::with_binary(
            &binary,
            dir,
            &dir.join("baseline-unused.ics"),
            72,
            120,
            &args,
        );
        old.marker("Preselected 2 subject(s)");
        old.send(b"/week\r");
        old.until("baseline query in subject pane", |s| {
            s.contains("Subjects (/ search: week)")
        });
        let header = old
            .screen
            .screen()
            .contents()
            .lines()
            .take(3)
            .collect::<String>();
        assert!(header.contains("Status"));
        assert!(!header.contains("week"));
        old.send(b"o");
        old.marker("Optimal: 7 occupied day(s), 0 gap minute(s).");
        old.send(b"r\x1b[H");
        old.until("baseline timetable at top", |s| {
            s.contains("Timetable")
                && s.contains("Status: OptimalKnown")
                && s.contains("Sun 23:35-24:00")
        });
        let old_screen = old.screen.screen().contents();
        assert!(week_row(&old_screen, "Time").is_none());
        assert!(week_row(&old_screen, "09:00").is_none());
        old.send(b"\tl\r");
        old.marker("selected 1");
        assert!(!old.screen.screen().contents().contains("Edit manual entry"));
        old.send(b"q");
        old.marker("TERMINAL_RESTORED");
        assert!(old.child.wait().unwrap().success());
        println!(
            "UX_COMPARISON {}",
            serde_json::json!({"baseline_revision":"149c684", "before":{"status_header":true,"query_in_top_bar":false,"weekly_day_columns":0,"half_hour_table_rows":0,"l_then_enter":"deselects subject because focus did not move"},"after":{"status_header":false,"query_in_top_bar":true,"weekly_day_columns":7,"half_hour_table_rows":30,"l_then_enter":"edits Manual entry without changing selection"},"same_fixture_and_terminal_size":true})
        );
    }
}

#[test]
fn actual_tui_80x24_reaches_members_all_notices_and_exports() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut catalog: Value = serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    // More notices than the old six-notice cap, through the real adapter and solver.
    for i in 0..10 {
        let id = format!("U{i}");
        catalog["classes"][&id] = serde_json::json!({
            "number": id, "name": "Unannounced meetings", "sectionKinds": []
        });
    }
    let catalog_path = dir.join("catalog.json");
    fs::write(&catalog_path, catalog.to_string()).unwrap();
    let term_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json");
    let mut args = vec![
        "--catalog",
        catalog_path.to_str().unwrap(),
        "--term",
        term_path,
        "tui",
        "--select",
        "A",
        "--select",
        "B",
    ];
    let unknowns = (0..10).map(|i| format!("U{i}")).collect::<Vec<_>>();
    for id in &unknowns {
        args.extend(["--select", id.as_str()]);
    }
    let output = dir.join("small.ics");
    let mut ui = Driver::with_size(dir, &output, 24, 80, &args);
    ui.marker("Preselected 12 subject(s)");
    let header = ui
        .screen
        .screen()
        .contents()
        .lines()
        .take(3)
        .collect::<String>();
    assert!(header.contains("Search subjects"));
    assert!(!header.contains("Status"));
    ui.send(b"l");
    ui.marker("> Manual entries");
    ui.send(b"\x1b[C");
    ui.marker("> Results");
    ui.send(b"h");
    ui.marker("> Manual entries");
    ui.send(b"\x1b[D");
    ui.marker("> Subjects");
    ui.send(b"/hjkl");
    ui.until("search input, not navigation shortcuts", |s| {
        s.lines().take(3).collect::<String>().contains("hjkl") && s.contains("selected 12")
    });
    ui.send(b"\x15\rj");
    ui.marker("> [x] B");
    ui.send(b"\x1b[A");
    ui.marker("> [x] A");
    ui.send(b"\x1b[B");
    ui.marker("> [x] B");
    ui.send(b"k");
    ui.marker("> [x] A");
    ui.send(b"?");
    ui.marker("Home/End");
    ui.marker("PgUp/PgDn");
    ui.marker("scroll results");
    ui.send(b"?");
    ui.send(b"o");
    ui.marker("Optimal: 1 occupied day(s), 0 gap minute(s).");
    ui.send(b"r");
    ui.marker("Timetable");
    ui.until("seven-day 30-minute grid", |s| {
        s.contains("> Results")
            && s.contains("PgUp/Dn |")
            && [
                "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun", "09:00", "09:30",
            ]
            .iter()
            .all(|label| s.contains(label))
    });
    println!("WEEK_VIEW_80x24\n{}", ui.screen.screen().contents());
    ui.send(b"n");
    ui.marker("(2/2)");
    ui.send(b"j");
    ui.marker("> B/lecture:");
    ui.send(b"k");
    ui.marker("> A/lecture:");
    ui.send(b"\x1b[B");
    ui.marker("> B/lecture:");
    ui.send(b"\x1b[A");
    ui.marker("> A/lecture:");
    ui.send(b"\x1b[F"); // End must expose the final notice, not a truncated subset.
    ui.marker("U9: no known meeting components");
    ui.send(b"\x1b[H");
    ui.marker("Timetable");
    ui.send(b"\x1b[6~"); // PageDown scrolls content, not the selected component.
    ui.until("paged results", |s| !s.contains("Timetable"));
    ui.send(b"\x1b[5~");
    ui.marker("Timetable");
    ui.send(b"e");
    ui.marker("Exported");
    ui.send(b"\x1b[F");
    ui.until("last wrapped export notice", |s| {
        s.contains("Export notice: U9: no known meeting") && s.contains("components")
    });
    let text = fs::read_to_string(&output).unwrap();
    let parsed: icalendar::Calendar = text.parse().unwrap();
    assert_eq!(parsed.events().count(), 6);
    assert!(
        text.contains("LOCATION:Room A"),
        "switched member must reach export"
    );
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    assert!(
        ui.raw
            .windows(b"\x1b[?1049l".len())
            .any(|w| w == b"\x1b[?1049l")
    );
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({
            "requirement":"small_terminal_controls_and_disclosure", "terminal":"80x24",
            "search_header_replaces_status":true, "hjkl_typed_in_search":true,
            "horizontal_focus_sequence":["Subjects","Manual","Results","Manual","Subjects"],
            "vertical_keys_checked":["j","k","Up","Down"],
            "result_navigation_checked":["r","PgUp","PgDn","Home","End","n"],
            "last_unresolved_notice_reached":"U9", "last_export_notice_reached":"U9",
            "same_time_member_position":"2/2", "exported_events":parsed.events().count(),
            "exported_selected_room":"Room A", "termios_and_alternate_screen_restored":true
        })
    );
}

#[test]
#[ignore = "requires live public Hydrant HTTPS and a native Unix PTY"]
fn actual_tui_live_selection_editor_solver_member_switch_export_and_restore() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let loaded = cli(dir, &["refresh"]);
    println!(
        "TUI_LIVE_DATA term={} courses={}",
        loaded["term"], loaded["courses"]
    );
    let reference = cli(dir, &["--offline", "optimize", "6.1200", "18.01"]);
    assert_eq!(reference["solution"]["status"], "optimal_known");
    let target = reference["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["course_id"] == "6.1200" && s["kind"] == "lecture")
        .unwrap();
    let meetings = target["section"]["meetings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let day = m["weekday"].as_u64().unwrap() as usize;
            let a = m["start_minute"].as_u64().unwrap();
            let b = m["end_minute"].as_u64().unwrap();
            format!(
                "{} {:02}:{:02}-{:02}:{:02}",
                ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][day],
                a / 60,
                a % 60,
                b / 60,
                b % 60
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let output = dir.join("tui.ics");
    let mut ui = Driver::new(dir, &output);
    ui.marker("Ready. Press ? for help.");
    ui.send(b"/6.1200\r ");
    ui.until("first subject selection", |s| {
        s.contains("6.1200") && !s.contains("Ready. Press ? for help.")
    });
    ui.send(b"/\x1518.01\r ");
    ui.until("second subject selection", |s| s.contains("18.01"));
    ui.send(b"a");
    ui.marker("Manual editor opened for");
    let fields = [
        "6.1200",
        "lecture",
        "PTY acceptance alternative",
        &meetings,
        "PTY ROOM",
        "",
        "",
    ];
    let mut edit = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            edit.push(b'\t');
        }
        edit.push(21);
        edit.extend(field.as_bytes());
    }
    edit.push(b'\r');
    ui.send(&edit);
    ui.marker("Manual entries saved. Press o to re-optimize.");
    let stored: Value =
        serde_json::from_slice(&fs::read(dir.join("manual.json")).unwrap()).unwrap();
    assert_eq!(
        stored["entries"][0]["option"]["label"],
        "PTY acceptance alternative"
    );
    ui.send(b"o");
    ui.marker("Optimal:");
    let expected = format!(
        "Optimal: {} occupied day(s), {} gap minute(s).",
        reference["solution"]["score"]["occupied_days"],
        reference["solution"]["score"]["gap_minutes"]
    );
    ui.marker(&expected);
    let index = reference["solution"]["choices"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["requirement_id"] == "6.1200/lecture")
        .unwrap();
    ui.send(b"r");
    for _ in 0..index {
        ui.send(b"\x1b[B");
    }
    ui.send(b"n");
    ui.marker("PTY ROOM");
    ui.send(b"e");
    ui.marker("Exported");
    let calendar = fs::read_to_string(&output).unwrap();
    assert!(calendar.contains("PTY ROOM"));
    assert!(calendar.contains("6.1200"));
    assert!(calendar.contains("18.01"));
    let parsed: icalendar::Calendar = calendar.parse().unwrap();
    let events = parsed.events().count();
    assert!(events > 0);
    // A second export must not overwrite the first file.
    ui.send(b"e");
    ui.until("overwrite rejection", |s| {
        // Long checkout/output paths can wrap "never overwritten" across rows.
        // Require the failure status and its reason, then verify bytes below.
        s.contains("Export failed:") && (s.contains("overwritten") || s.contains("already exists"))
    });
    assert_eq!(fs::read_to_string(&output).unwrap(), calendar);

    // Exercise review fixes through real keyboard input, not just AppState helpers.
    ui.send(b"mx");
    ui.marker("Disabled");
    let disabled: Value =
        serde_json::from_slice(&fs::read(dir.join("manual.json")).unwrap()).unwrap();
    assert_eq!(disabled["entries"][0]["enabled"], false);
    ui.send(b"e");
    ui.marker("optimize before exporting");
    assert_eq!(fs::read_to_string(&output).unwrap(), calendar);

    ui.send(b"\r");
    ui.marker("Edit manual entry");
    ui.send(b"\t\t\x15Edited while disabled\r");
    ui.marker("Manual entries saved. Press o to re-optimize.");
    let saved_bytes = fs::read(dir.join("manual.json")).unwrap();
    let edited: Value = serde_json::from_slice(&saved_bytes).unwrap();
    assert_eq!(edited["entries"][0]["enabled"], false);
    assert_eq!(
        edited["entries"][0]["option"]["label"],
        "Edited while disabled"
    );
    assert_eq!(
        edited["entries"][0]["option"]["id"],
        stored["entries"][0]["option"]["id"]
    );

    ui.send(b"\r");
    ui.marker("Edit manual entry");
    ui.send(b"\x1518.01\r");
    ui.marker("manual edits cannot move");
    assert_eq!(fs::read(dir.join("manual.json")).unwrap(), saved_bytes);
    ui.send(b"\x1b");
    ui.until("editor cancelled", |screen| {
        !screen.contains("Edit manual entry")
    });
    ui.send(b"x");
    ui.marker("Enabled");
    ui.send(b"o");
    ui.marker(&expected);

    ui.send(b"/\x15mathematics for\r");
    ui.marker("mathematics for");
    assert!(ui.screen.screen().contents().contains("selected 2"));
    assert_eq!(fs::read_to_string(&output).unwrap(), calendar);
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    assert!(
        ui.raw
            .windows(b"\x1b[?1049l".len())
            .any(|w| w == b"\x1b[?1049l")
    );
    println!(
        "TUI_ACCEPTANCE subjects=2 selection_across_search=true multiword_search=true manual_editor=true disabled_edit_preserved=true scope_move_rejected=true stale_export_rejected=true reoptimization=true exact_score={} member_switch=true events={events} no_clobber=true termios_restored=true alternate_screen_restored=true",
        reference["solution"]["score"]
    );
}
