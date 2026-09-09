//! Native-terminal acceptance of the actual executable. Offline cases run by default.
//! Live case: cargo test --test tui_acceptance -- --ignored --nocapture
#![cfg(unix)]

mod support;
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
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    screen: vt100::Parser,
    raw: Vec<u8>,
}

fn session_ui(dir: &Path, extra: &[&str]) -> Driver {
    let mut args = vec![
        "--catalog",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json"),
        "--term",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
    ];
    args.extend_from_slice(extra);
    Driver::with_size(dir, &dir.join("unused-session-export.ics"), 40, 120, &args)
}

fn saved_selection(dir: &Path) -> Value {
    let value: Value =
        serde_json::from_slice(&fs::read(dir.join("sessions.json")).unwrap()).unwrap();
    value["terms"].as_object().unwrap().values().next().unwrap()["selected"].clone()
}

fn quit_session(mut ui: Driver) {
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
}

#[test]
fn actual_tui_autosaves_restores_and_keeps_cleared_selection_empty() {
    let dir = tempfile::tempdir().unwrap();
    let mut ui = session_ui(dir.path(), &[]);
    ui.marker("No classes selected.");
    ui.send(b"/A\r ");
    ui.marker("Optimal:");
    assert_eq!(saved_selection(dir.path()), serde_json::json!(["A"]));
    quit_session(ui);

    let mut ui = session_ui(dir.path(), &[]);
    ui.marker("Optimal:");
    assert!(pane_contents(&ui.screen.screen().contents(), "Selected").contains("Algorithms"));
    ui.send(b"s ");
    ui.marker("No classes selected.");
    assert_eq!(saved_selection(dir.path()), serde_json::json!([]));
    quit_session(ui);

    let mut ui = session_ui(dir.path(), &[]);
    ui.marker("No classes selected.");
    assert_eq!(saved_selection(dir.path()), serde_json::json!([]));
    quit_session(ui);
}

#[test]
fn actual_tui_explicit_seed_replaces_saved_but_ephemeral_never_writes() {
    let dir = tempfile::tempdir().unwrap();
    for subject in ["A", "B"] {
        let mut ui = session_ui(dir.path(), &["tui", "--select", subject]);
        ui.marker("Optimal:");
        assert_eq!(saved_selection(dir.path()), serde_json::json!([subject]));
        quit_session(ui);
    }
    let before = fs::read(dir.path().join("sessions.json")).unwrap();
    let mut ui = session_ui(dir.path(), &["--no-restore"]);
    ui.marker("No classes selected.");
    ui.marker("Ephemeral selections");
    ui.send(b"/A\r ");
    ui.marker("Optimal:");
    quit_session(ui);
    assert_eq!(fs::read(dir.path().join("sessions.json")).unwrap(), before);
    let mut ui = session_ui(dir.path(), &[]);
    ui.marker("Optimal:");
    assert_eq!(saved_selection(dir.path()), serde_json::json!(["B"]));
    quit_session(ui);
}

#[test]
fn actual_tui_preserves_bad_session_files_and_keeps_warning_visible() {
    for bytes in ["broken json", r#"{"version":99,"future_data":"precious"}"#] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        fs::write(&path, bytes).unwrap();
        let mut ui = session_ui(dir.path(), &[]);
        ui.marker("Selections NOT saved:");
        ui.send(b" ");
        ui.marker("Optimal:");
        assert!(
            ui.screen
                .screen()
                .contents()
                .contains("Selections NOT saved:")
        );
        quit_session(ui);
        assert_eq!(fs::read_to_string(path).unwrap(), bytes);
    }
}

#[test]
fn actual_tui_second_writer_cannot_overwrite_an_active_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = session_ui(dir.path(), &["tui", "--select", "A"]);
    first.marker("Optimal:");
    let before = fs::read(dir.path().join("sessions.json")).unwrap();
    let mut second = session_ui(dir.path(), &["tui", "--select", "B"]);
    second.marker("Optimal:");
    second.marker("Selections NOT saved:");
    quit_session(second);
    assert_eq!(fs::read(dir.path().join("sessions.json")).unwrap(), before);
    quit_session(first);
    let mut resumed = session_ui(dir.path(), &[]);
    resumed.marker("Optimal:");
    assert_eq!(saved_selection(dir.path()), serde_json::json!(["A"]));
    assert!(
        !resumed
            .screen
            .screen()
            .contents()
            .contains("Selections NOT saved:")
    );
    quit_session(resumed);
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
            master: pair.master,
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
    fn resize(&mut self, rows: u16, cols: u16) {
        self.screen.screen_mut().set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }
    fn until(&mut self, label: &str, condition: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let screen = self.screen.screen().contents();
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {label}\n{screen}"
            );
            if condition(&screen) {
                // A PTY read can split a redraw immediately after its title.
                // Drain the rest before callers compare whole rendered panes.
                match self.output.recv_timeout(Duration::from_millis(20)) {
                    Ok(bytes) => {
                        self.screen.process(&bytes);
                        self.raw.extend(bytes);
                        continue;
                    }
                    Err(_) => return,
                }
            }
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

// Exports always gain a Unix-time suffix, so locate the fresh file from its hint.
// Order by (stamp, counter), not filename: lexicographic sort would rank
// `stem-9.ics` above `stem-10.ics`.
fn export_sort_key(path: &Path, stem: &str) -> (u64, u64) {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let suffix = name.strip_prefix(&format!("{stem}-")).unwrap_or("");
    let (stamp, counter) = match suffix.split_once('-') {
        Some((stamp, counter)) => (stamp, counter.parse().unwrap_or(0)),
        None => (suffix, 0),
    };
    (stamp.parse().unwrap_or(0), counter)
}

fn latest_export(dir: &Path, hint: &Path) -> std::path::PathBuf {
    let stem = hint
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("schedule");
    let mut matches: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().and_then(|e| e.to_str()) == Some("ics")
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&format!("{stem}-")))
        })
        .collect();
    matches.sort_by_key(|path| export_sort_key(path, stem));
    matches.pop().expect("expected a timestamped export")
}

fn read_latest_export(dir: &Path, hint: &Path) -> String {
    let path = latest_export(dir, hint);
    assert!(
        !hint.exists(),
        "the bare hint {} must never be written",
        hint.display()
    );
    fs::read_to_string(&path).unwrap()
}

fn week_row(screen: &str, time: &str) -> Option<Vec<String>> {
    let half_hour = time.ends_with(":30");
    let label = if half_hour {
        time.replace(":30", ":00")
    } else {
        time.to_owned()
    };
    let mut rows = screen.lines();
    let found = rows.find(|line| line.split('│').any(|cell| cell.trim() == label));
    (if half_hour {
        found.and_then(|_| rows.next())
    } else {
        found
    })
    .map(|line| {
        line.trim()
            .strip_prefix('│')
            .and_then(|row| row.strip_suffix('│'))
            .expect("weekday rows have left and right borders")
            .split('│')
            .map(str::trim)
            .map(str::to_owned)
            .collect()
    })
}

// Read a rendered pane without its focus-dependent title or border. Keeping the
// real terminal's pane boundaries lets scroll checks distinguish Subjects and
// Selected from the Timetable instead of matching text anywhere.
fn pane_contents(screen: &str, title: &str) -> String {
    if title == "Timetable" {
        // The timetable has no parent box. Read below its fixed heading until
        // the footer's border, including the table's own top/bottom borders.
        let top = screen
            .lines()
            .position(|line| line.starts_with("Timetable") || line.starts_with("> Timetable"))
            .unwrap_or_else(|| panic!("missing timetable heading\n{screen}"));
        return screen
            .lines()
            .skip(top + 1)
            .take_while(|line| !(line.starts_with('┌') && !line.contains('┬')))
            .collect::<Vec<_>>()
            .join("\n");
    }
    let lines: Vec<Vec<char>> = screen.lines().map(|line| line.chars().collect()).collect();
    let (top, left, right) = lines
        .iter()
        .enumerate()
        .find_map(|(row, line)| {
            let text: String = line.iter().collect();
            let title_byte = text.find(title)?;
            let title_col = text[..title_byte].chars().count();
            let left = line[..title_col].iter().rposition(|c| *c == '┌')?;
            let right = title_col + line[title_col..].iter().position(|c| *c == '┐')?;
            Some((row, left, right))
        })
        .unwrap_or_else(|| panic!("missing pane {title}\n{screen}"));
    lines[top + 1..]
        .iter()
        .take_while(|line| line.get(left) != Some(&'└'))
        .map(|line| {
            line.iter()
                .skip(left + 1)
                .take(right - left - 1)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_three_pane_ui(screen: &str) {
    for title in ["Subjects", "Selected", "Timetable"] {
        assert!(screen.contains(title), "missing {title}\n{screen}");
    }
    assert!(
        !screen.to_lowercase().contains("results"),
        "removed Results pane or shortcut is still visible\n{screen}"
    );
}

fn assert_removed_shortcuts_ignored(ui: &mut Driver) {
    // Clearing the help overlay can materialize trailing blank cells in vt100.
    // Those invisible cells do not represent a shortcut changing the UI.
    let visible = |screen: &str| {
        screen
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    };
    for key in *b"rnp" {
        let before = ui.screen.screen().contents();
        // Opening then closing help is a round-trip barrier. Merely waiting for
        // an unchanged screen could pass before the removed key is processed.
        ui.send(&[key, b'?']);
        ui.marker("Help");
        let help = ui.screen.screen().contents().to_lowercase();
        assert!(!help.contains("results"), "obsolete help entry\n{help}");
        assert!(!help.contains("n/p"), "obsolete member shortcut\n{help}");
        ui.send(b"?");
        ui.until("help closed after removed shortcut", |screen| {
            !screen.contains("Help")
        });
        let after = ui.screen.screen().contents();
        assert_eq!(
            visible(&after),
            visible(&before),
            "removed shortcut {} changed the UI",
            key as char
        );
        assert_three_pane_ui(&after);
    }
}

#[test]
fn actual_tui_keeps_container_borders_visible_through_pane_resizes() {
    let temp = tempfile::tempdir().unwrap();
    let mut ui = Driver::with_size(
        temp.path(),
        &temp.path().join("resize-unused.ics"),
        61,
        107,
        &[
            "--catalog",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json"),
            "--term",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
            "tui",
            "--select",
            "A",
            "--select",
            "B",
        ],
    );
    ui.marker("Optimal:");
    for (cols, rows) in [(107, 61), (80, 24), (53, 24), (170, 55), (107, 61)] {
        ui.resize(rows, cols);
        ui.until("containers fit resized pane", |screen| {
            let lines = screen.lines().collect::<Vec<_>>();
            [0, 3, rows as usize - 5].into_iter().all(|row| {
                lines
                    .get(row)
                    .and_then(|line| line.chars().nth(cols as usize - 2))
                    == Some('┐')
            }) && lines
                .get(rows as usize - 1)
                .and_then(|line| line.chars().nth(cols as usize - 2))
                == Some('┘')
        });
        let screen = ui.screen.screen();
        for row in 0..rows {
            assert!(
                screen
                    .cell(row, cols - 1)
                    .unwrap()
                    .contents()
                    .trim()
                    .is_empty(),
                "right gutter at {cols}x{rows}, row {row}"
            );
        }
        let contents = screen.contents();
        assert_three_pane_ui(&contents);
        let header = contents
            .lines()
            .find(|line| line.starts_with("│Time"))
            .unwrap();
        assert_eq!(header.chars().nth(cols as usize - 2), Some('│'));
        for day in ["Mon", "Tue", "Wed", "Thu", "Fri"] {
            assert!(header.contains(day), "missing {day} at {cols}x{rows}");
        }
        assert!(pane_contents(&contents, "Selected").contains("Algorithms"));
    }
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    if std::env::var_os("HYDRANT_PANE_BASELINE_BIN").is_some() {
        compare_visible_borders_with_pre_fix_binary();
    }
}

fn compare_visible_borders_with_pre_fix_binary() {
    let baseline = std::env::var("HYDRANT_PANE_BASELINE_BIN").unwrap();
    let evidence = std::env::var("HYDRANT_PANE_EVIDENCE_DIR").unwrap();
    let evidence = Path::new(&evidence);
    fs::create_dir_all(evidence).unwrap();
    let mut observations = Vec::new();
    for (cols, rows) in [(107, 61), (80, 24), (53, 24)] {
        for (label, binary) in [
            ("before", baseline.as_str()),
            ("after", env!("CARGO_BIN_EXE_hydrant-optimizer")),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut ui = Driver::with_binary(
                binary,
                temp.path(),
                &temp.path().join("unused.ics"),
                rows,
                cols,
                &[
                    "--catalog",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json"),
                    "--term",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
                    "tui",
                    "--select",
                    "A",
                    "--select",
                    "B",
                ],
            );
            ui.until("selection ready for comparison", |screen| {
                screen.contains("Preselected 2 subject(s)")
                    || screen.contains("Optimizing")
                    || screen.contains("Optimal:")
            });
            if ui
                .screen
                .screen()
                .contents()
                .contains("Preselected 2 subject(s)")
            {
                ui.send(b"o");
            }
            ui.marker("Optimal:");
            let screen = ui.screen.screen();
            let contents = screen.contents();
            assert!(contents.contains("Selected classes | 2"));
            let header = contents
                .lines()
                .position(|line| line.starts_with("│Time"))
                .unwrap() as u16;
            let checks = [
                (0, "┐"),
                (1, "│"),
                (2, "┘"),
                (3, "┐"),
                (4, "│"),
                (rows - 5, "┐"),
                (rows - 4, "│"),
                (rows - 1, "┘"),
                (header - 1, "┐"),
                (header, "│"),
            ];
            // Model the host clipping its last reported column, as in the
            // screenshot. The executable and terminal output are otherwise real.
            let visible_borders = checks
                .iter()
                .filter(|(row, expected)| {
                    screen.cell(*row, cols - 2).unwrap().contents() == *expected
                })
                .count();
            let visible = contents
                .lines()
                .map(|line| line.chars().take(cols as usize - 1).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(evidence.join(format!("{label}-{cols}x{rows}.txt")), visible).unwrap();
            let observation = serde_json::json!({
                "version": label,
                "terminal_columns": cols,
                "visible_columns": cols - 1,
                "rows": rows,
                "border_checks": checks.len(),
                "visible_right_borders": visible_borders,
                "selected_classes": 2,
                "source": "actual executable in native PTY, final column cropped",
            });
            println!("UX_OBSERVATION {observation}");
            observations.push(observation);
            assert_eq!(
                visible_borders,
                if label == "after" { checks.len() } else { 0 }
            );
            ui.send(b"q");
            ui.marker("TERMINAL_RESTORED");
            assert!(ui.child.wait().unwrap().success());
        }
    }
    fs::write(
        evidence.join("observations.json"),
        serde_json::to_string_pretty(&observations).unwrap(),
    )
    .unwrap();
}

#[test]
fn actual_tui_pe_search_selection_labels_and_bounded_export() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("pe.ics");
    let mut ui = Driver::with_size(
        temp.path(),
        &output,
        55,
        170,
        &[
            "--catalog",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/catalog-pe.json"
            ),
            "--term",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
            "tui",
            "--select",
            "A",
        ],
    );
    ui.marker("Optimal:");
    ui.send(b"/swimming\r");
    ui.marker("> [ ] PE.1000.Q1");
    ui.send(b"\r");
    ui.marker("> [x] PE.1000.Q1");
    assert!(
        ui.screen
            .screen()
            .contents()
            .contains("PE.1000.Q1 Swimming")
    );
    ui.send(b"a");
    ui.marker("Add manual entry");
    ui.until("published PE dates in editor fields", |screen| {
        [("Start date", "2026-10-26"), ("End date", "2026-10-30")]
            .iter()
            .all(|(label, date)| {
                screen
                    .lines()
                    .any(|line| line.contains(label) && line.contains(date))
            })
    });
    ui.send(b"\x1b");
    ui.until("manual editor cancelled", |screen| {
        !screen.contains("Add manual entry")
    });
    ui.send(b"t");
    ui.until("all academic and PE component labels", |screen| {
        screen.contains("> Timetable") && week_row(screen, "11:00").is_some()
    });
    let screen = ui.screen.screen().contents();
    let header: Vec<_> = screen
        .lines()
        .find(|line| line.contains("Time") && line.contains("Mon"))
        .unwrap()
        .split('│')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .collect();
    assert_eq!(header, ["Time", "Mon", "Tue", "Wed", "Thu", "Fri"]);
    let academics = week_row(&screen, "09:00").unwrap();
    assert_eq!(&academics[1..4], ["A Lec", "A Rec", "A Lab"]);
    assert_eq!(week_row(&screen, "11:00").unwrap()[1], "PE.1000.Q1 PE");
    ui.send(b"e");
    ui.marker("Exported");
    let text = read_latest_export(temp.path(), &output);
    let calendar: icalendar::Calendar = text.parse().unwrap();
    assert_eq!(calendar.events().count(), 4);
    assert_eq!(
        support::expand_calendar(&text)
            .matches("BEGIN:VEVENT")
            .count(),
        7
    );
    let pe_events: Vec<_> = text
        .split("BEGIN:VEVENT")
        .skip(1)
        .filter(|event| event.contains("SUMMARY:PE."))
        .collect();
    assert_eq!(pe_events.len(), 1);
    assert!(pe_events[0].contains("DTSTART:20261026T150000Z"));
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    println!(
        "TUI_PE_ACCEPTANCE search=true selection=true date_prefill=true weekday_columns=5 component_labels=Lec,Rec,Lab,PE events=7 bounded_pe_events=1 terminal_restored=true"
    );
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
    let choice = reference["solution"]["choices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|choice| choice["requirement_id"] == "W/lecture")
        .unwrap();
    let initial_room = choice["members"][0]["room"].as_str().unwrap();
    let next_member = &choice["members"][1];
    let next_room = next_member["room"].as_str().unwrap();
    let next_label = next_member["label"].as_str().unwrap();
    let mut args = source.to_vec();
    args.extend(["tui", "--select", "W", "--select", "X"]);
    let output = dir.join("week.ics");
    let mut ui = Driver::with_size(dir, &output, 110, 120, &args);
    ui.marker("Optimal:");
    ui.send(b"/week\r");
    ui.until("query visible in top search bar", |s| {
        let header = s.lines().take(3).collect::<String>();
        header.contains("Search subjects")
            && header.contains("week")
            && s.contains("Selected classes | 2")
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
    ui.marker("Optimal: 7 occupied day(s), 0 gap minute(s).");
    ui.send(b"t");
    ui.until("weekday grid with weekend disclosure", |s| {
        s.contains("> Timetable")
            && s.contains("PgUp/Dn |")
            && week_row(s, "11:00").is_some()
            && s.contains("weekend")
    });
    let screen = ui.screen.screen().contents();
    let headers = week_row(&screen, "Time").unwrap();
    assert_eq!(headers, ["Time", "Mon", "Tue", "Wed", "Thu", "Fri"]);
    let row_times: Vec<_> = (0..1440)
        .step_by(60)
        .map(|minute| format!("{:02}:{:02}", minute / 60, minute % 60))
        .collect();
    let observed_times: Vec<_> = screen
        .lines()
        .flat_map(|line| line.split('│').map(str::trim))
        .filter(|cell| cell.len() == 5 && chrono::NaiveTime::parse_from_str(cell, "%H:%M").is_ok())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        observed_times, row_times,
        "the actual terminal must contain exactly the expected hourly labels"
    );
    let first = week_row(&screen, "09:00").unwrap();
    assert!(first[1].contains("2×") && first[1].contains('W') && first[1].contains('X'));
    assert!(first[2..].iter().all(|cell| cell.is_empty()));
    for (time, day) in [("09:30", 2), ("10:00", 3), ("10:30", 4), ("11:00", 5)] {
        assert_eq!(
            week_row(&screen, time).unwrap()[day],
            "W Lec",
            "wrong day/slot {time}/{day}"
        );
    }
    assert_eq!(
        week_row(&screen, "10:00").unwrap()[2],
        initial_room,
        "the second session row must show its room, not repeat the legend"
    );
    assert_eq!(
        week_row(&screen, "10:30").unwrap()[2],
        "",
        "end boundary must not occupy another bucket"
    );
    assert!(!screen.contains("Legend:"));
    assert!(!screen.contains("Lec=lecture"));
    assert_three_pane_ui(&screen);
    let timetable = pane_contents(&screen, "Timetable");
    assert_eq!(
        timetable.matches(initial_room).count(),
        1,
        "only Tuesday spans two rows, so all one-row meetings must omit rooms"
    );
    assert!(!timetable.contains("Bucket room"));
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({
            "requirement": "session_room_row",
            "meeting": "Tue 09:35-10:05",
            "first_row": week_row(&screen, "09:30").unwrap()[2],
            "second_row": week_row(&screen, "10:00").unwrap()[2],
            "after_end": week_row(&screen, "10:30").unwrap()[2],
            "room_occurrences": timetable.matches(initial_room).count(),
            "one_row_room_occurrences": timetable.matches("Bucket room").count(),
        })
    );
    // Optional actual-executable comparison, independent of renderer test helpers.
    if let Ok(binary) = std::env::var("HYDRANT_ROOM_BASELINE_BIN") {
        let mut old = Driver::with_binary(
            &binary,
            dir,
            &dir.join("room-baseline-unused.ics"),
            110,
            120,
            &args,
        );
        old.marker("Optimal: 7 occupied day(s), 0 gap minute(s).");
        old.send(b"t");
        old.until("baseline room row visible", |s| {
            s.contains("> Timetable") && week_row(s, "11:00").is_some()
        });
        let before = old.screen.screen().contents();
        let before_legend = week_row(&before, "09:30").unwrap()[2].clone();
        let before_room = week_row(&before, "10:00").unwrap()[2].clone();
        assert_eq!(before_legend, "W Lec");
        assert_eq!(before_room, "");
        assert_eq!(week_row(&screen, "09:30").unwrap()[2], before_legend);
        assert!(!pane_contents(&before, "Timetable").contains("Bucket room"));
        old.send(b"q");
        old.marker("TERMINAL_RESTORED");
        assert!(old.child.wait().unwrap().success());
        println!(
            "UX_COMPARISON {}",
            serde_json::json!({
                "requirement": "session_room_row",
                "before": {"first_row": before_legend, "second_row": before_room},
                "after": {
                    "first_row": week_row(&screen, "09:30").unwrap()[2],
                    "second_row": week_row(&screen, "10:00").unwrap()[2],
                },
                "same_fixture_and_terminal_size": true,
                "one_row_rooms_hidden_in_both": true,
                "baseline_terminal_restored": true,
            })
        );
    }
    for title in ["Subjects", "Selected"] {
        assert!(week_row(&pane_contents(&screen, title), "Time").is_none());
    }
    assert_eq!(week_row(&timetable, "Time").unwrap(), headers);
    let timetable_title = screen
        .lines()
        .position(|line| line.starts_with("Timetable") || line.starts_with("> Timetable"))
        .unwrap();
    let subjects_title = screen
        .lines()
        .position(|line| line.contains("Subjects") && line.contains('┌'))
        .unwrap();
    assert!(
        timetable_title > subjects_title,
        "Timetable belongs below the Subjects and Selected panes"
    );
    assert!(!screen.lines().nth(timetable_title).unwrap().contains('┌'));
    let border = screen.lines().nth(timetable_title + 1).unwrap();
    assert!(border.starts_with('┌') && border.ends_with('┐'));
    assert!(
        border.contains('┬'),
        "only the table border, not a parent box"
    );
    assert_eq!(
        border.chars().count(),
        119,
        "The table must fill the safe pane width, leaving the terminal's final column unused"
    );
    let grid_header = screen.lines().nth(timetable_title + 2).unwrap();
    assert_eq!(grid_header.matches('│').count(), 7, "no parent box sides");
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({"requirement":"weekly_grid", "headers":headers, "visible_hour_labels":observed_times.len(), "first_row":observed_times.first(), "last_row":observed_times.last(), "monday_shared_bucket":first[1], "all_weekdays_placed":true,"weekend_disclosed":true, "adjacent_meetings_not_conflicts":true})
    );

    // Drill into the first chronological session (W), then its same-time
    // options. Exact-minute and room fidelity is checked in the actual export.
    ui.send(b"\r");
    ui.marker("Sessions: Enter options");
    ui.send(b"\r");
    ui.marker("Options: h/l switch");
    ui.send(b"l");
    ui.until("actual member switched", |s| {
        s.contains(&format!("W/lecture now uses {next_label}"))
    });
    let switched = ui.screen.screen().contents();
    assert_eq!(week_row(&switched, "10:00").unwrap()[2], next_room);
    assert!(!pane_contents(&switched, "Timetable").contains(initial_room));
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({
            "requirement": "session_room_switch",
            "before": week_row(&screen, "10:00").unwrap()[2],
            "after": week_row(&switched, "10:00").unwrap()[2],
            "stale_room_occurrences": pane_contents(&switched, "Timetable").matches(initial_room).count(),
        })
    );
    ui.send(b"e");
    ui.marker("Exported");
    let calendar = read_latest_export(dir, &output);
    let parsed: icalendar::Calendar = calendar.parse().unwrap();
    assert!(parsed.events().count() < 17);
    let calendar = support::expand_calendar(&calendar);
    assert_eq!(calendar.matches("BEGIN:VEVENT").count(), 17);
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
        serde_json::json!({"requirement":"exact_times_and_export", "source_exact_times":["Mon 09:05-09:20","Tue 09:35-10:05","Sun 23:35-24:00"], "member_navigation":"Enter, Enter, l", "switched_room":next_room, "events":17, "series":parsed.events().count(), "exact_utc_boundaries_preserved":true})
    );
    // Search hides X, but Selected must retain both classes. Horizontal focus
    // now reaches Selected rather than the old always-visible Manual pane.
    ui.send(b"/\rl");
    ui.marker("> Selected");
    let selected = pane_contents(&ui.screen.screen().contents(), "Selected");
    assert!(selected.contains('W') && selected.contains('X'));
    ui.send(b"m");
    ui.marker("> Manual entries");
    ui.send(b"\r");
    ui.marker("Edit manual entry");
    assert!(
        ui.screen
            .screen()
            .contents()
            .contains("Selected classes | 2")
    );
    ui.send(b"\x1b");
    ui.until("editor closed", |s| !s.contains("Edit manual entry"));
    ui.send(b"\x1b");
    ui.until("manual overlay closed to Selected", |s| {
        s.contains("> Selected") && !s.contains("Manual entries")
    });
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    println!(
        "UX_OBSERVATION {}",
        serde_json::json!({"requirement":"horizontal_navigation", "l_focus":"Selected", "manual_overlay":"m, Enter edits, Esc closes to Selected", "selected_subjects_after":2, "terminal_restored":true})
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
        let old_screen = old.screen.screen().contents();
        assert!(week_row(&old_screen, "Time").is_none());
        assert!(week_row(&old_screen, "09:00").is_none());
        old.send(b"/\rl\r");
        old.marker("selected 1");
        assert!(!old.screen.screen().contents().contains("Edit manual entry"));
        old.send(b"q");
        old.marker("TERMINAL_RESTORED");
        assert!(old.child.wait().unwrap().success());
        println!(
            "UX_COMPARISON {}",
            serde_json::json!({"baseline_revision":"149c684", "before":{"status_header":true,"query_in_top_bar":false,"weekly_day_columns":0,"half_hour_table_rows":0,"l_focus":"Subjects (focus did not move)","l_then_enter":"deselects filtered subject"},"after":{"status_header":false,"query_in_top_bar":true,"weekly_day_columns":5,"half_hour_table_rows":5,"l_focus":"Selected","three_pane_ui":true,"manual_overlay_preserves_selection":true},"same_fixture_and_terminal_size":true})
        );
    }
}

#[test]
fn actual_tui_selected_classes_survive_search_and_support_remove_reselect() {
    let temp = tempfile::tempdir().unwrap();
    let mut ui = Driver::with_size(
        temp.path(),
        &temp.path().join("selected-unused.ics"),
        36,
        120,
        &[
            "--catalog",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json"),
            "--term",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
            "tui",
            "--select",
            "A",
            "--select",
            "B",
        ],
    );
    ui.marker("Optimal:");
    assert!(!ui.screen.screen().contents().contains("Manual entries"));
    assert_three_pane_ui(&ui.screen.screen().contents());
    assert_removed_shortcuts_ignored(&mut ui);
    // Tab and Shift-Tab cycle through exactly three panes. Horizontal keys
    // enter Timetable from the lists but navigate schedules inside Timetable.
    for title in ["Selected", "Timetable", "Subjects"] {
        ui.send(b"\t");
        ui.marker(&format!("> {title}"));
        assert_three_pane_ui(&ui.screen.screen().contents());
    }
    for title in ["Timetable", "Selected", "Subjects"] {
        ui.send(b"\x1b[Z");
        ui.marker(&format!("> {title}"));
        assert_three_pane_ui(&ui.screen.screen().contents());
    }
    for (forward, backward) in [(b"l".as_slice(), b"h".as_slice()), (b"\x1b[C", b"\x1b[D")] {
        ui.send(forward);
        ui.marker("> Selected");
        ui.send(backward);
        ui.marker("> Subjects");
        ui.send(backward);
        ui.marker("> Timetable");
        ui.send(b"\t");
        ui.marker("> Subjects");
        ui.send(forward);
        ui.marker("> Selected");
        ui.send(forward);
        ui.marker("> Timetable");
        ui.send(b"\t");
        ui.marker("> Subjects");
    }
    ui.send(b"/biology\r");
    ui.marker("> [x] B");
    let screen = ui.screen.screen().contents();
    assert!(!pane_contents(&screen, "Subjects").contains("Algorithms"));
    let selected = pane_contents(&screen, "Selected");
    assert!(selected.contains("Algorithms") && selected.contains("Biology"));
    ui.send(b"sj ");
    ui.until("Space removes highlighted B from Selected", |s| {
        s.contains("> Selected")
            && s.contains("Selected classes | 1")
            && !pane_contents(s, "Selected").contains("Biology")
    });
    let screen = ui.screen.screen().contents();
    assert!(pane_contents(&screen, "Selected").contains("Algorithms"));
    assert!(pane_contents(&screen, "Subjects").contains("[ ] B"));
    ui.marker("Optimal:");
    // A is hidden by the Biology search. Enter must still remove A, not B.
    ui.send(b"\r");
    ui.until("Enter removes remaining hidden-by-search A", |s| {
        let selected = pane_contents(s, "Selected");
        s.contains("Selected classes | 0")
            && !selected.contains("Algorithms")
            && !selected.contains("Biology")
    });
    ui.send(b"/\r ");
    ui.until("reselect B through still-active search", |s| {
        s.contains("Selected classes | 1") && s.contains("> [x] B")
    });
    ui.send(b"/\x15algorithm\r ");
    ui.until("reselect A after changing search", |s| {
        s.contains("Selected classes | 2") && s.contains("> [x] A")
    });
    ui.marker("Optimal:");
    ui.send(b"s");
    ui.marker("> Selected");
    let selected = pane_contents(&ui.screen.screen().contents(), "Selected");
    assert!(selected.contains("Algorithms") && selected.contains("Biology"));
    ui.send(b"m");
    ui.marker("> Manual entries");
    ui.send(b"a");
    ui.marker("Add manual entry");
    ui.send(b"\x1b");
    ui.until("cancel editor returns to manual overlay", |s| {
        s.contains("> Manual entries") && !s.contains("Add manual entry")
    });
    ui.send(b"\x1b");
    ui.until(
        "Esc closes overlay without quitting or changing selection",
        |s| {
            s.contains("> Selected")
                && s.contains("Selected classes | 2")
                && !s.contains("Manual entries")
        },
    );
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    assert!(
        ui.raw
            .windows(b"\x1b[?1049l".len())
            .any(|bytes| bytes == b"\x1b[?1049l")
    );
    println!(
        "TUI_SELECTED_ACCEPTANCE independent_search=true space_and_enter_remove=true reselect=true three_pane_cycle=true removed_shortcuts_ignored=true manual_overlay=true terminal_restored=true"
    );
}

#[test]
fn actual_tui_timetable_scrolls_and_resets_independently_of_subject_lists() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut catalog: Value = serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    catalog["classes"] = serde_json::json!({
        "W": {"number":"W", "name":"Long weekday", "sectionKinds":["lecture"], "lectureSections":[]}
    });
    let unknowns = (0..10).map(|i| format!("U{i}")).collect::<Vec<_>>();
    for id in &unknowns {
        catalog["classes"][id] = serde_json::json!({
            "number":id, "name":"Unannounced meetings", "sectionKinds":[]
        });
    }
    let catalog_path = dir.join("long-week.json");
    fs::write(&catalog_path, catalog.to_string()).unwrap();
    let source = [
        "--catalog",
        catalog_path.to_str().unwrap(),
        "--term",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
    ];
    let mut manual = source.to_vec();
    manual.extend([
        "manual",
        "add",
        "--course",
        "W",
        "--kind",
        "lecture",
        "--label",
        "Long lecture",
        "--room",
        "Long room",
        "--meetings",
        "Mon 09:00-23:00",
    ]);
    cli(dir, &manual);
    let mut args = source.to_vec();
    args.extend(["tui", "--select", "W"]);
    for id in &unknowns {
        args.extend(["--select", id]);
    }
    let mut ui = Driver::with_size(dir, &dir.join("scroll-unused.ics"), 24, 100, &args);
    ui.marker("Optimal:");
    ui.marker("Optimal: 1 occupied day(s), 0 gap minute(s).");
    ui.send(b"t");
    ui.marker("> Timetable");
    let screen = ui.screen.screen().contents();
    assert_three_pane_ui(&screen);
    let subjects_top = pane_contents(&screen, "Subjects");
    let selected_top = pane_contents(&screen, "Selected");
    let timetable_top = pane_contents(&screen, "Timetable");
    assert!(week_row(&timetable_top, "08:00").is_some());

    for (down, up) in [
        (b"j".as_slice(), b"k".as_slice()),
        (b"\x1b[B", b"\x1b[A"),
        (b"\x1b[6~", b"\x1b[5~"),
    ] {
        ui.send(down);
        ui.until("Timetable scrolled without moving subject lists", |s| {
            pane_contents(s, "Timetable") != timetable_top
        });
        let screen = ui.screen.screen().contents();
        assert_eq!(pane_contents(&screen, "Subjects"), subjects_top);
        assert_eq!(pane_contents(&screen, "Selected"), selected_top);
        ui.send(up);
        ui.until("Timetable returns to first page", |s| {
            pane_contents(s, "Timetable") == timetable_top
        });
        let screen = ui.screen.screen().contents();
        assert_eq!(pane_contents(&screen, "Subjects"), subjects_top);
        assert_eq!(pane_contents(&screen, "Selected"), selected_top);
    }
    ui.send(b"\x1b[F");
    ui.until("Timetable End reaches late weekday rows", |s| {
        week_row(&pane_contents(s, "Timetable"), "22:30").is_some()
    });
    let timetable_bottom = pane_contents(&ui.screen.screen().contents(), "Timetable");
    assert!(week_row(&timetable_bottom, "09:00").is_none());
    let screen = ui.screen.screen().contents();
    assert_eq!(pane_contents(&screen, "Subjects"), subjects_top);
    assert_eq!(pane_contents(&screen, "Selected"), selected_top);
    ui.send(b"\x1b[H");
    ui.until("Timetable Home reveals overnight hours", |s| {
        week_row(&pane_contents(s, "Timetable"), "00:00").is_some()
    });
    ui.send(b"\x1b[F");
    ui.until("Timetable End restores last page", |s| {
        pane_contents(s, "Timetable") == timetable_bottom
    });

    ui.send(b"s");
    ui.marker("> Selected");
    ui.send(b"\x1b[6~");
    ui.until("Selected scrolls to the last subject", |s| {
        pane_contents(s, "Selected").contains("> W Long weekday")
    });
    assert_eq!(
        pane_contents(&ui.screen.screen().contents(), "Timetable"),
        timetable_bottom
    );
    ui.send(b"\t");
    ui.marker("> Timetable");
    assert_eq!(
        pane_contents(&ui.screen.screen().contents(), "Timetable"),
        timetable_bottom
    );
    let selected_bottom = pane_contents(&ui.screen.screen().contents(), "Selected");
    assert_ne!(selected_bottom, selected_top);
    ui.send(b"t");
    ui.until("t resets Timetable without moving subject lists", |s| {
        pane_contents(s, "Timetable") == timetable_top
    });
    assert_eq!(
        pane_contents(&ui.screen.screen().contents(), "Selected"),
        selected_bottom
    );
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    println!(
        "TUI_SCROLL_ACCEPTANCE independent_subject_lists=true timetable_j_k_arrows_pages_home_end=true focus_preserves_scroll=true timetable_t_reset_only=true terminal_restored=true"
    );
}

#[test]
fn actual_tui_80x24_navigates_sessions_and_exports_with_unknown_subjects() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut catalog: Value = serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    // Unknown meeting records must remain selectable without blocking export.
    for i in 0..10 {
        let id = format!("U{i}");
        catalog["classes"][&id] = serde_json::json!({
            "number": id, "name": "Unannounced meetings", "sectionKinds": []
        });
    }
    let catalog_path = dir.join("catalog.json");
    fs::write(&catalog_path, catalog.to_string()).unwrap();
    let term_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json");
    let reference = cli(
        dir,
        &[
            "--catalog",
            catalog_path.to_str().unwrap(),
            "--term",
            term_path,
            "optimize",
            "A",
            "B",
        ],
    );
    let members = reference["solution"]["choices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|choice| choice["requirement_id"] == "A/lecture")
        .unwrap()["members"]
        .as_array()
        .unwrap();
    assert_eq!(members.len(), 2);
    let first_label = members[0]["label"].as_str().unwrap();
    let next_label = members[1]["label"].as_str().unwrap();
    let next_room = members[1]["room"].as_str().unwrap();
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
    ui.marker("Optimal:");
    let header = ui
        .screen
        .screen()
        .contents()
        .lines()
        .take(3)
        .collect::<String>();
    assert!(header.contains("Search subjects"));
    assert!(!header.contains("Status"));
    assert_three_pane_ui(&ui.screen.screen().contents());
    ui.send(b"l");
    ui.marker("> Selected");
    ui.send(b"\x1b[C");
    ui.marker("> Timetable");
    ui.send(b"\t");
    ui.marker("> Subjects");
    ui.send(b"h");
    ui.marker("> Timetable");
    ui.send(b"\x1b[Z");
    ui.marker("> Selected");
    ui.send(b"\x1b[D");
    ui.marker("> Subjects");
    ui.send(b"/hjkl");
    ui.until("search input, not navigation shortcuts", |s| {
        s.lines().take(3).collect::<String>().contains("hjkl")
            && s.contains("Selected classes | 12")
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
    ui.marker("Scroll the timetable.");
    assert!(
        !ui.screen
            .screen()
            .contents()
            .to_lowercase()
            .contains("results")
    );
    ui.send(b"?");
    ui.marker("Optimal: 1 occupied day(s), 0 gap minute(s).");
    ui.send(b"t");
    ui.until("weekday 30-minute grid", |s| {
        s.contains("> Timetable")
            && ["Mon", "Tue", "Wed", "Thu", "Fri", "08:00", "09:00"]
                .iter()
                .all(|label| s.contains(label))
    });
    println!("WEEK_VIEW_80x24\n{}", ui.screen.screen().contents());
    ui.send(b"\r");
    ui.marker("Sessions: Enter options");
    for (key, requirement) in [
        (b"j".as_slice(), "B/lecture"),
        (b"k".as_slice(), "A/lecture"),
        (b"\x1b[B".as_slice(), "B/lecture"),
        (b"\x1b[A".as_slice(), "A/lecture"),
    ] {
        ui.send(key);
        ui.until(&format!("selected timetable session {requirement}"), |s| {
            s.lines()
                .any(|line| line.starts_with("> Timetable") && line.contains(requirement))
        });
    }
    ui.send(b"\r");
    ui.marker("Options: h/l switch");
    assert_removed_shortcuts_ignored(&mut ui);
    for (key, label) in [
        (b"l".as_slice(), next_label),
        (b"\x1b[D".as_slice(), first_label),
        (b"\x1b[C".as_slice(), next_label),
    ] {
        ui.send(key);
        ui.marker(&format!("A/lecture now uses {label}"));
    }
    ui.send(b"\x1b");
    ui.marker("Sessions: Enter options");
    ui.send(b"\x1b");
    ui.marker("Enter sessions");
    assert_three_pane_ui(&ui.screen.screen().contents());
    ui.send(b"e");
    ui.marker("Exported");
    let text = read_latest_export(dir, &output);
    let parsed: icalendar::Calendar = text.parse().unwrap();
    assert_eq!(parsed.events().count(), 2);
    assert_eq!(
        support::expand_calendar(&text)
            .matches("BEGIN:VEVENT")
            .count(),
        6
    );
    assert!(
        text.contains(&format!("LOCATION:{next_room}")),
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
            "requirement":"small_terminal_controls_and_export", "terminal":"80x24",
            "search_header_replaces_status":true, "hjkl_typed_in_search":true,
            "focus_sequence":["Subjects","Selected","Timetable","Subjects","Timetable","Selected","Subjects"],
            "vertical_keys_checked":["j","k","Up","Down"],
            "timetable_navigation_checked":["t","Enter","j","k","Up","Down","l","Left","Right","Esc"],
            "removed_shortcuts_ignored":["r","n","p"], "unknown_subjects_selected":10,
            "same_time_member_position":"2/2", "exported_events":6, "exported_series":parsed.events().count(),
            "exported_selected_room":next_room, "termios_and_alternate_screen_restored":true
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
    ui.marker("Add manual entry");
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
    ui.marker("Optimal:");
    let stored: Value =
        serde_json::from_slice(&fs::read(dir.join("manual.json")).unwrap()).unwrap();
    assert_eq!(
        stored["entries"][0]["option"]["label"],
        "PTY acceptance alternative"
    );
    ui.marker("Optimal:");
    let expected = format!(
        "Optimal: {} occupied day(s), {} gap minute(s).",
        reference["solution"]["score"]["occupied_days"],
        reference["solution"]["score"]["gap_minutes"]
    );
    ui.marker(&expected);
    let updated = cli(dir, &["--offline", "optimize", "6.1200", "18.01"]);
    let choices = updated["solution"]["choices"].as_array().unwrap();
    let mut blocks: Vec<_> = choices
        .iter()
        .flat_map(|choice| {
            choice["meetings"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|meeting| {
                    let day = meeting["weekday"].as_u64().unwrap();
                    (day < 5).then_some((
                        day,
                        meeting["start_minute"].as_u64().unwrap(),
                        choice["requirement_id"].as_str().unwrap(),
                    ))
                })
        })
        .collect();
    blocks.sort_unstable();
    let target = blocks
        .iter()
        .find(|block| block.2 == "6.1200/lecture")
        .unwrap();
    let previous_days = blocks
        .iter()
        .filter(|block| block.0 < target.0)
        .map(|block| block.0)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let same_day: Vec<_> = blocks.iter().filter(|block| block.0 == target.0).collect();
    let earlier_sessions = same_day.iter().position(|block| *block == target).unwrap();
    ui.send(b"t\r");
    ui.marker("Sessions: Enter options");
    // Horizontal movement reaches the next occupied day. Move to its first
    // session before walking down to the target, independent of choice order.
    ui.send(&vec![b'l'; previous_days]);
    ui.send(&vec![b'k'; same_day.len()]);
    ui.send(&vec![b'j'; earlier_sessions]);
    ui.until("live lecture selected in timetable", |s| {
        s.lines()
            .any(|line| line.starts_with("> Timetable") && line.contains("6.1200/lecture"))
    });
    ui.send(b"\r");
    ui.marker("Options: h/l switch");
    let member_index = choices
        .iter()
        .find(|choice| choice["requirement_id"] == "6.1200/lecture")
        .unwrap()["members"]
        .as_array()
        .unwrap()
        .iter()
        .position(|member| member["room"] == "PTY ROOM")
        .unwrap();
    ui.send(&vec![b'l'; member_index]);
    ui.marker("PTY acceptance alternative");
    ui.send(b"e");
    ui.marker("Exported");
    let first_path = latest_export(dir, &output);
    let calendar = fs::read_to_string(&first_path).unwrap();
    assert!(calendar.contains("PTY ROOM"));
    assert!(calendar.contains("6.1200"));
    assert!(calendar.contains("18.01"));
    let parsed: icalendar::Calendar = calendar.parse().unwrap();
    let events = parsed.events().count();
    assert!(events > 0);
    // A second export mints a fresh Unix-time file, never overwriting the first.
    ui.send(b"e");
    ui.marker("Exported");
    let second_path = latest_export(dir, &output);
    assert_ne!(
        second_path, first_path,
        "repeat exports must not reuse a filename"
    );
    assert_eq!(fs::read_to_string(&first_path).unwrap(), calendar);
    assert_eq!(fs::read_to_string(&second_path).unwrap(), calendar);

    // Exercise review fixes through real keyboard input, not just AppState helpers.
    ui.send(b"mx");
    ui.marker("Optimal:");
    let disabled: Value =
        serde_json::from_slice(&fs::read(dir.join("manual.json")).unwrap()).unwrap();
    assert_eq!(disabled["entries"][0]["enabled"], false);
    ui.send(b"e");
    ui.marker("Export failed:");
    assert_eq!(fs::read_to_string(&first_path).unwrap(), calendar);

    ui.send(b"\r");
    ui.marker("Edit manual entry");
    ui.send(b"\t\t\x15Edited while disabled\r");
    ui.marker("Optimal:");
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
    ui.marker("Optimal:");
    ui.marker(&expected);

    ui.send(b"/\x15mathematics for\r");
    ui.marker("mathematics for");
    assert!(
        ui.screen
            .screen()
            .contents()
            .contains("Selected classes | 2")
    );
    assert_eq!(fs::read_to_string(&first_path).unwrap(), calendar);
    assert_eq!(fs::read_to_string(&second_path).unwrap(), calendar);
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    assert!(
        ui.raw
            .windows(b"\x1b[?1049l".len())
            .any(|w| w == b"\x1b[?1049l")
    );
    println!(
        "TUI_ACCEPTANCE subjects=2 selection_across_search=true multiword_search=true manual_editor=true disabled_edit_preserved=true scope_move_rejected=true stale_export_rejected=true reoptimization=true exact_score={} member_switch=true events={events} timestamped_exports=true termios_restored=true alternate_screen_restored=true",
        reference["solution"]["score"]
    );
}

#[test]
fn actual_tui_nested_navigation_cycles_optima_blocks_and_fixed_time_members() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let catalog_path = dir.join("navigation.json");
    let mut catalog: Value = serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    catalog["classes"] = serde_json::json!({
        "N": {"number":"N", "name":"Navigation", "sectionKinds":["lecture"], "lectureSections":[]}
    });
    fs::write(&catalog_path, catalog.to_string()).unwrap();
    let source = [
        "--catalog",
        catalog_path.to_str().unwrap(),
        "--term",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
    ];
    for (label, room, meetings) in [
        (
            "Early one",
            "Early room one",
            "Mon 09:00-09:30;Mon 10:00-10:30;Wed 10:00-10:30",
        ),
        (
            "Early two",
            "Early room two",
            "Mon 09:00-09:30;Mon 10:00-10:30;Wed 10:00-10:30",
        ),
        (
            "Late one",
            "Late room one",
            "Mon 11:00-11:30;Mon 12:00-12:30;Wed 12:00-12:30",
        ),
        (
            "Late two",
            "Late room two",
            "Mon 11:00-11:30;Mon 12:00-12:30;Wed 12:00-12:30",
        ),
    ] {
        let mut args = source.to_vec();
        args.extend([
            "manual",
            "add",
            "--course",
            "N",
            "--kind",
            "lecture",
            "--label",
            label,
            "--room",
            room,
            "--meetings",
            meetings,
        ]);
        cli(dir, &args);
    }
    let mut args = source.to_vec();
    args.extend(["optimize", "N"]);
    let reference = cli(dir, &args);
    let alternatives = reference["solution"]["alternatives"].as_array().unwrap();
    assert_eq!(alternatives.len(), 2);
    let second = &alternatives[1][0];
    let first_time = second["meetings"][0]["start_minute"].as_u64().unwrap();
    let next_time = second["meetings"][1]["start_minute"].as_u64().unwrap();
    let time = |minutes: u64| format!("{:02}:{:02}", minutes / 60, minutes % 60);
    let output = dir.join("navigation.ics");
    let mut args = source.to_vec();
    args.extend(["tui", "--select", "N"]);
    let mut ui = Driver::with_size(dir, &output, 40, 140, &args);
    ui.marker("Optimal:");
    ui.send(b"t");
    ui.marker("> Timetable 1/2");
    for (key, expected) in [
        (b"l".as_slice(), "2/2"),
        (b"\x1b[D".as_slice(), "1/2"),
        (b"\x1b[C".as_slice(), "2/2"),
        (b"h".as_slice(), "1/2"),
        (b"l".as_slice(), "2/2"),
    ] {
        ui.send(key);
        ui.marker(&format!("> Timetable {expected}"));
    }
    ui.send(b"\r");
    ui.marker("Sessions: Enter options");

    // Observe the actual terminal's yellow focus cell, not internal AppState.
    fn selected_cell(ui: &mut Driver, time: &str, day: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let screen = ui.screen.screen().contents();
            if let Some((row, line)) = screen
                .lines()
                .enumerate()
                .find(|(_, line)| line.starts_with(&format!("│{time}│")))
            {
                let column = line
                    .chars()
                    .enumerate()
                    .filter(|(_, c)| *c == '│')
                    .nth(day + 1)
                    .unwrap()
                    .0
                    + 1;
                if ui
                    .screen
                    .screen()
                    .cell(row as u16, column as u16)
                    .unwrap()
                    .bgcolor()
                    == vt100::Color::Idx(15)
                {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "selected block missing at {time}/{day}\n{screen}"
            );
            if let Ok(bytes) = ui.output.recv_timeout(Duration::from_millis(50)) {
                ui.screen.process(&bytes);
                ui.raw.extend(bytes);
            }
        }
    }
    selected_cell(&mut ui, &time(first_time), 0);
    for (keys, minutes, day) in [
        (b"j".as_slice(), next_time, 0),
        (b"\x1b[A".as_slice(), first_time, 0),
        (b"\x1b[B".as_slice(), next_time, 0),
        (b"l".as_slice(), next_time, 2),
        (b"\x1b[D".as_slice(), next_time, 0),
        (b"\x1b[C".as_slice(), next_time, 2),
        (b"h".as_slice(), next_time, 0),
        (b"k".as_slice(), first_time, 0),
    ] {
        ui.send(keys);
        selected_cell(&mut ui, &time(minutes), day);
    }
    ui.send(b"\r");
    ui.marker("Options: h/l switch");
    let member = &second["members"][1];
    ui.send(b"l");
    ui.marker(&format!(
        "N/lecture now uses {}.",
        member["label"].as_str().unwrap()
    ));
    ui.send(b"\x1b[D");
    ui.marker(&format!(
        "N/lecture now uses {}.",
        second["members"][0]["label"].as_str().unwrap()
    ));
    ui.send(b"\x1b[C");
    ui.marker(&format!(
        "N/lecture now uses {}.",
        member["label"].as_str().unwrap()
    ));
    ui.send(b"e");
    ui.marker("Exported");
    let calendar = read_latest_export(dir, &output);
    assert!(calendar.contains(&format!("LOCATION:{}", member["room"].as_str().unwrap())));
    assert!(calendar.contains(&format!("T{:02}0000Z", first_time / 60 + 4)));
    ui.send(b"\x1b");
    ui.marker("Sessions: Enter options");
    ui.send(b"\x1b");
    ui.marker("Enter sessions");
    ui.send(b"\t");
    ui.marker("> Subjects");
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
    println!(
        "UX_OBSERVATION nested_navigation: real executable, 2 equal optima, all hjkl/arrows verified by terminal focus color, fixed-time member and exported room/time verified, Escape backs out, Tab exits, terminal restored"
    );
}

#[test]
fn actual_tui_empty_timetable_is_readable_and_closed_past_1330() {
    let temp = tempfile::tempdir().unwrap();
    let args = [
        "--catalog",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json"),
        "--term",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
        "tui",
    ];
    let mut ui = Driver::with_size(temp.path(), &temp.path().join("unused.ics"), 40, 120, &args);
    ui.until("daytime empty timetable rendered", |s| {
        s.contains("No timetable yet.") && s.contains("14:00")
    });
    let screen = ui.screen.screen().contents();
    let table = pane_contents(&screen, "Timetable");
    let top = table.lines().find(|line| line.starts_with('┌')).unwrap();
    let bottom = table.lines().find(|line| line.starts_with('└')).unwrap();
    assert!(top.ends_with('┐') && bottom.ends_with('┘'));
    assert_eq!(top.chars().count(), bottom.chars().count());
    assert_eq!(bottom.matches('┴').count(), 5);
    let rows: Vec<_> = table
        .lines()
        .filter(|line| line.starts_with('│') && !line.contains("Time"))
        .collect();
    assert!(rows.first().unwrap().starts_with("│08:00"));
    assert!(rows.last().unwrap().starts_with("│14:00"));
    for (index, row) in rows.iter().enumerate() {
        let cells: Vec<_> = row.split('│').collect();
        assert!(cells[2..7].iter().all(|cell| cell.trim().is_empty()));
        assert_eq!(cells[1].trim().is_empty(), index % 2 == 1);
    }
    println!(
        "TIMETABLE_READABILITY_OBSERVATION {}",
        serde_json::json!({
            "terminal": "120x40", "first_hour": "08:00", "last_visible_hour": "14:00",
            "empty_day_cells": rows.len() * 5, "dots": 0,
            "hour_labels": rows.iter().filter(|row| row.contains(":00")).count(),
            "half_hour_rows": rows.len(), "border_width": bottom.chars().count(),
            "connected_bottom_junctions": 5, "bottom_corners": "└┘"
        })
    );
    ui.send(b"q");
    ui.marker("TERMINAL_RESTORED");
    assert!(ui.child.wait().unwrap().success());
}
