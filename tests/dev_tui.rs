//! Watcher lifecycle tests using a real TUI in a PTY and a controlled build stub.
#![cfg(unix)]
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[test]
fn watcher_reloads_recovers_and_restores_terminal() {
    run_watcher(b"q", false);
    run_watcher(b"\x03", false);
}

#[test]
#[ignore = "runs real Cargo rebuilds in an isolated source copy"]
fn watcher_with_real_cargo() {
    run_watcher(b"q", true);
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn run_watcher(quit: &[u8], real: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    for dir in ["scripts", "src", "bin"] {
        fs::create_dir(path.join(dir)).unwrap();
    }
    fs::write(
        path.join("scripts/dev-tui.sh"),
        include_str!("../scripts/dev-tui.sh"),
    )
    .unwrap();
    fs::write(path.join("Cargo.toml"), "fixture").unwrap();
    fs::write(path.join("Cargo.lock"), "fixture").unwrap();
    fs::write(path.join("src/change.rs"), "first").unwrap();
    fs::write(
        path.join("bin/cargo"),
        r#"#!/bin/sh
if grep -q broken src/change.rs; then echo EXPECTED_BUILD_ERROR; exit 1; fi
mkdir -p target/tui-dev/debug
cp "$REAL_TUI" target/tui-dev/debug/hydrant-optimizer
"#,
    )
    .unwrap();
    fs::set_permissions(path.join("bin/cargo"), fs::Permissions::from_mode(0o755)).unwrap();
    let watched = if real {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        copy_tree(&repo.join("src"), &path.join("src"));
        for file in ["Cargo.toml", "Cargo.lock"] {
            fs::copy(repo.join(file), path.join(file)).unwrap();
        }
        fs::remove_file(path.join("bin/cargo")).unwrap();
        fs::create_dir(path.join("target")).unwrap();
        std::os::unix::fs::symlink(repo.join("target"), path.join("target/tui-dev")).unwrap();
        path.join("src/tui.rs")
    } else {
        path.join("src/change.rs")
    };
    let original = fs::read_to_string(&watched).unwrap();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "stty -g > before; bash scripts/dev-tui.sh \"$@\"; code=$?; stty -g > after; echo WATCHER_EXITED; exit $code", "--"]);
    command.args([
        "--catalog",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json"),
        "--term",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/term.json"),
        "--data-dir",
    ]);
    command.arg(path.join("data"));
    command.cwd(path);
    command.env(
        "PATH",
        format!(
            "{}:{}",
            path.join("bin").display(),
            std::env::var("PATH").unwrap()
        ),
    );
    command.env("REAL_TUI", env!("CARGO_BIN_EXE_hydrant-optimizer"));
    command.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = std::sync::Mutex::new(pair.master.take_writer().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut output = String::new();
        let mut until = |needle: &str| {
            let deadline = Instant::now() + Duration::from_secs(if real { 180 } else { 20 });
            while !output.contains(needle) {
                assert!(
                    Instant::now() < deadline,
                    "waiting for {needle} (quit={quit:?}): {output}"
                );
                if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                    if output.contains("\x1b[6n") {
                        writer.lock().unwrap().write_all(b"\x1b[1;1R").unwrap();
                        output = output.replace("\x1b[6n", "");
                    }
                }
            }
            output.clear();
        };
        until("Subjects");
        let edited = if real {
            assert!(original.contains("{}Subjects | {} found"));
            original.replace("{}Subjects | {} found", "{}ReloadVerified | {} found")
        } else {
            format!("{original}\n// reload\n")
        };
        fs::write(&watched, edited).unwrap();
        until("[dev-tui] Building");
        until(if real { "ReloadVerified" } else { "Subjects" });
        fs::write(&watched, "broken").unwrap();
        until("Build failed.");
        fs::write(&watched, &original).unwrap();
        until("Subjects");
        writer.lock().unwrap().write_all(quit).unwrap();
        writer.lock().unwrap().flush().unwrap();
        until("WATCHER_EXITED");
        let normalize = |value: String| {
            value
                .trim()
                .split(':')
                .map(|field| {
                    // PENDIN is a transient kernel retype flag, not a terminal mode.
                    if cfg!(target_os = "macos") && field.starts_with("lflag=") {
                        let flags = u32::from_str_radix(&field[6..], 16).unwrap();
                        format!("lflag={:x}", flags & !0x20000000)
                    } else {
                        field.to_string()
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            normalize(fs::read_to_string(path.join("before")).unwrap()),
            normalize(fs::read_to_string(path.join("after")).unwrap())
        );
        assert!(child.wait().unwrap().success());
    }));
    if result.is_err() {
        eprintln!("CHILD STATUS: {:?}", child.try_wait());
        let _ = child.kill();
    }
    result.unwrap();
}
