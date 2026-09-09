# TUI development mode

The launcher is [`scripts/dev-tui.sh`](../scripts/dev-tui.sh). It watches agent or editor changes and automatically rebuilds and restarts the TUI.

Run once in an interactive terminal from the repository root:

```sh
./scripts/dev-tui.sh --offline
# Reproducible preview without network access or an existing cache:
./scripts/dev-tui.sh --catalog tests/fixtures/catalog.json \
  --term tests/fixtures/term.json tui --select A --select B
```

The launcher watches `src/`, `Cargo.toml`, `Cargo.lock`, and `.cargo/` for edits (including agent edits), then automatically rebuilds and restarts the TUI. It uses incremental **debug builds**, not release builds, in `target/tui-dev`. No extra watcher package is required. Changes are polled every tenth of a second and debounced. Rust code still needs to compile, so this is automatic rebuild/restart rather than in-process hot patching.

Cargo output is hidden by default and saved to `target/tui-dev/build.log` (replaced on each build). No watcher status messages or build diagnostics appear in the terminal. The old TUI stays visible and interactive during background builds, and is replaced only after a successful build. Failed builds leave the old TUI running. To inspect diagnostics separately, run `tail -f target/tui-dev/build.log` in another terminal. Failed builds wait for the next edit and retry automatically. `q` exits the TUI and watcher, and Ctrl+C stops the launcher. Terminal modes and the cursor are restored on reload and exit. Arguments are forwarded unchanged on every launch. Use `--offline` after caching the catalog, or the fixture arguments above, to avoid fetching on every restart.

Verify the watcher with `cargo test --locked --test dev_tui`. Add `-- --include-ignored --test-threads=1` to also exercise real Cargo rebuilds, a compiler error, and recovery in an isolated source copy.

Reloads reset in-memory selections, search, scroll position, and unsaved editor input. Use `tui --select SUBJECT` (repeatable) to restore a preview selection. Saved manual entries remain in the normal data directory. Pass `--data-dir PATH` to isolate development data if desired.

## Inspect hidden build logs

```sh
# Latest build diagnostics:
cat target/tui-dev/build.log

# Follow builds from another terminal:
tail -f target/tui-dev/build.log
```

The log is replaced on every build. The TUI still needs an interactive terminal. Only compiler output is hidden, not the TUI itself. The current TUI remains visible and interactive during compilation and after failed builds. It restarts only when a successful, up-to-date build is ready. On the very first launch, the terminal remains quiet until the initial build completes.

If the launcher script itself changes, quit and rerun it once. Changes to watched Rust sources are picked up automatically.
