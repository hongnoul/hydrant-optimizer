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
        cmd.args(["-c", "before=$(stty -g); \"$@\"; code=$?; after=$(stty -g); if [ \"$before\" = \"$after\" ]; then printf '\nTERMINAL_RESTORED\n'; else printf '\nTERMINAL_NOT_RESTORED\n'; exit 99; fi; exit \"$code\"", "--",env!("CARGO_BIN_EXE_hydrant-optimizer"),"--offline","--data-dir",dir.to_str().unwrap(),"--output",output.to_str().unwrap()]);
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
    ui.send(b"?");
    ui.marker("Home/End first/last result line");
    ui.marker("PgUp/PgDn");
    ui.marker("scroll results");
    ui.send(b"?");
    ui.send(b"o");
    ui.marker("Optimal: 1 occupied day(s), 0 gap minute(s).");
    ui.send(b"r");
    ui.marker("> A/lecture:");
    ui.send(b"n");
    ui.marker("(2/2)");
    ui.send(b"\x1b[B");
    ui.marker("> B/lecture:");
    ui.send(b"\x1b[A");
    ui.marker("> A/lecture:");
    ui.send(b"\x1b[F"); // End must expose the final notice, not a truncated subset.
    ui.marker("U9: no known meeting components");
    ui.send(b"\x1b[H");
    ui.marker("Timetable");
    ui.send(b"\x1b[6~"); // PageDown scrolls content, not the selected component.
    ui.until("paged results", |s| !s.contains("Status: OptimalKnown"));
    ui.send(b"\x1b[5~");
    ui.marker("Status: OptimalKnown");
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
        s.contains("never overwritten") || s.contains("already exists")
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
