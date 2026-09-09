# Hydrant Optimizer

A local MIT course scheduler. Select a fixed set of subjects, supply missing section times, find an exact compact weekly timetable, and export a local `.ics` file.

The optimizer is open source under the [MIT License](LICENSE). Working demo features a TUI. The Rust library exposes the scheduling model and optimizer independently of the TUI through `hydrant_optimizer::model` and `hydrant_optimizer::optimizer`. A web backend can call that library, or integrate with the CLI's JSON output. A web frontend and hosting are not included in this demo.

Read [ARCHITECTURE.MD](ARCHITECTURE.MD) for the module boundaries and [OPTIMIZER.MD](OPTIMIZER.MD) for the exactness contract.

## Build and run

Install a current stable Rust toolchain, then:

```sh
cargo build --release --locked
./target/release/hydrant-optimizer
```

The executable opens a searchable multi-select terminal UI. The first launch downloads Hydrant's public catalog. Subsequent launches refresh it, falling back to the validated cache with a visible notice if fetching fails. Use `--offline` to skip the network. Launches fetch before entering terminal raw mode.

```sh
# Fetch/cache the catalog, then search without a network connection.
hydrant-optimizer refresh
hydrant-optimizer --offline search 'mathematics'

# Optimize fixed subjects, showing the timetable, score and unresolved notices.
hydrant-optimizer --offline optimize 6.1200 18.01

# Create a manual section. The meetings are one atomic alternative, not independent choices.
hydrant-optimizer --offline manual add --course 6.1200 --kind recitation \
  --label 'Announced section' --room '32-123' \
  --meetings 'Wed 13:05-14:00;Fri 13:05-14:00'

# Export the selected schedule. Every export gains a Unix-time suffix
hydrant-optimizer --offline --output my-schedule.ics optimize 6.1200 18.01 --export

# Machine-readable results include same-time members and their stable IDs.
hydrant-optimizer --offline --json optimize 6.1200 18.01
hydrant-optimizer --offline optimize 6.1200 18.01 \
  --member '6.1200/recitation=OPTION_ID'
```

Use `./target/release/hydrant-optimizer` in place of `hydrant-optimizer` unless installed on `PATH`. `cargo install --path . --locked` installs the executable locally. Subject availability depends on the downloaded term. Run `--help` and `manual --help` for the full CLI.

### Calendar recurrence

ICS exports group equal-duration meetings in each selected course section into a
recurring series with a stable UID and explicit `RDATE` occurrences. This lets
calendar apps that support iCalendar recurrence offer series-level editing and
deletion. Lectures and recitations stay separate, as do meetings with different
durations. Exact dates preserve holidays, alternate class days, PE date bounds,
and New York daylight-saving transitions. Export counts still count individual
class occurrences, not series.

Previously imported standalone events will not be converted automatically.
Remove the old import before importing a newly generated file to avoid duplicates.
The exact deletion prompt depends on the calendar app.

### Live TUI development

Run once in an interactive terminal from the repository:

```sh
./scripts/dev-tui.sh --offline
# Reproducible preview without network access or an existing cache:
./scripts/dev-tui.sh --catalog tests/fixtures/catalog.json \
  --term tests/fixtures/term.json tui --select A --select B
```

The launcher watches `src/`, `Cargo.toml`, `Cargo.lock`, and `.cargo/` for edits (including agent edits), then automatically rebuilds and restarts the TUI. It uses incremental **debug builds**, not release builds, in `target/tui-dev`. No extra watcher package is required. Changes are polled every half-second and debounced. Rust code still needs to compile, so this is automatic rebuild/restart rather than in-process hot patching.

Cargo output is hidden by default and saved to `target/tui-dev/build.log` (replaced on each build). Only a short build status or error notice appears in the terminal. To inspect diagnostics separately, run `tail -f target/tui-dev/build.log` in another terminal. Failed builds wait for the next edit and retry automatically. `q` exits the TUI and watcher, and Ctrl+C stops the launcher. Terminal modes and the cursor are restored on reload and exit. Arguments are forwarded unchanged on every launch. Use `--offline` after caching the catalog, or the fixture arguments above, to avoid fetching on every restart.

Verify the watcher with `cargo test --locked --test dev_tui`. Add `-- --include-ignored --test-threads=1` to also exercise real Cargo rebuilds, a compiler error, and recovery in an isolated source copy.

Selected classes now survive reloads and normal restarts automatically. `tui --select SUBJECT` (repeatable) replaces the saved selection on the watcher's first launch only. Later rebuilds resume your edits, including an empty selection. Search, scroll position, section-member preferences, and unsaved editor input still reset. Pass `--data-dir PATH` to isolate development data, or `--no-restore` for an ephemeral preview that neither loads nor saves selections.

See [the TUI development guide](docs/tui-development.md) for the launcher, hidden build logs, and reload behavior.

### Terminal controls

| Key | Action |
|---|---|
| `/` | Focus the top search bar. Ctrl+U clears the query. Type a number/title, then Enter to return to the list. |
| Up/Down or `k`/`j` | Move through the focused list, or scroll the timetable. Space/Enter adds a subject or removes a selected class. |
| Left/Right or `h`/`l` | Focus the previous/next pane: Subjects, Selected classes, Timetable. The active pane has a cyan border (heading for Timetable) and `>` title marker. |
| Tab / Shift+Tab | Cycle Subjects, Selected classes, Timetable panels. |
| `s` | Focus Selected classes. This list stays visible independently of the search filter. |
| `a` / `m` | Add a manual section for the highlighted class / open retained manual entries in an overlay. Escape or `m` closes the overlay. |
| Enter / `x` in Manual | Edit an entry / toggle enabled state. |
| `c` | Cancel a running search. No optimum is claimed for cancelled work. |
| `t` | Focus Timetable and reset its scroll position. |
| Enter in Timetable | Navigate blocks, then same-time members. Use h/l to switch members and Escape to go back. |
| PgUp/PgDn in Timetable | Scroll the timetable. |
| Home/End in Timetable | Jump to the first/last line. |
| `e` | Export the chosen sections to `--output`. |
| `?` / `q` | Help / quit. Escape closes search, help, an editor, or the manual overlay first. Otherwise it quits. |

The manual editor's fields are Subject, Component, Section label, Meetings, Room, Start date, End date. Tab/Shift+Tab changes fields, Ctrl+U clears one, Enter or Ctrl+S saves, and Escape cancels. Enter multiple meetings separated by semicolons. Selection survives catalog searches. Selection or manual changes automatically optimize the timetable in the background, cancelling any outdated run. Preselected subjects are optimized on startup.

The top search bar replaces the old status panel. Navigation letters remain ordinary text while searching or editing. Action feedback stays in the footer.

All panels and overlays resize to the current terminal pane, leaving one unused column on the right so borders stay visible beside the host pane's edge.

Subjects and Selected classes share the upper row equally. Optimized schedules appear below them as a **separate edge-to-edge timetable**, without an outer container or legend. A single heading shows focus and scroll position above the grid. **Monday–Friday columns and 30-minute rows** span the earliest through latest chosen weekday meetings. Colored cells show subject numbers and components: **Lec**, **Rec**, **Lab**, **PE**, or another component label. Narrow cells shorten the subject number before the component. Off-grid meetings occupy each intersecting half-hour cell without changing their actual times. A `2×` (or higher) marker indicates multiple meetings sharing a half-hour cell, not necessarily a scheduling conflict. Crowded cells retain a component summary rather than suggesting incorrect subject/component pairings. Weekend meetings are explicitly disclosed outside the grid and remain in optimization and export.

The timetable scrolls at **80×24** using PgUp/PgDn and Home/End. The Results pane and its shortcuts have been removed. Full machine-readable optimization details remain available through the CLI.

### Local files

- macOS default: `~/Library/Application Support/hydrant-optimizer`.
- Other Unix default: `~/.local/share/hydrant-optimizer`.
- `$XDG_DATA_HOME/hydrant-optimizer` takes precedence when set.
- `--data-dir PATH` overrides the location for all commands.
- Downloaded cache and versioned `manual.json` are separate. Refreshing never rewrites manual sections.
- Versioned `sessions.json` stores selected subject IDs separately for each term. Every add/remove is saved atomically before optimization, including clearing the last class. Startup restores the current term and recomputes the timetable from the current catalog and manual overlays. Calendar exports are not used as session state.
- `tui --select SUBJECT` explicitly replaces the current term's saved selection. Unknown explicit subjects are rejected without changing it. `--no-restore` neither reads nor writes selection state and does not disable the catalog cache or manual-section persistence.
- Unavailable saved subjects are disclosed and retained on disk, but omitted from optimization. A different term starts empty without erasing older terms. To replace retained unavailable IDs, start with explicit `tui --select` subjects.
- Corrupt or newer-version session files are preserved. IO failures or a second TUI using the same data directory show a persistent **Selections NOT saved** warning. The UI remains usable without overwriting saved state. Close the other TUI and restart, or use a separate `--data-dir` / `--no-restore`. `sessions.lock` is an advisory lock file whose lock is released by the OS even after forced termination. Do not delete it while the app is running.
- Manual entries are scoped by term, subject, component, and stable option ID. `manual list` works even without a cache. `manual disable ID` and `manual enable ID` control retained entries. `manual add --id ID ...` edits an existing entry, preserving its scope and enabled state. Components can be lecture, recitation, lab, design, pe, or another component published for that subject.
- For reproducible inputs, `--catalog FILE --term FILE` reads local Hydrant `latest.json` and matching `latestTerm.json` snapshots through the same adapter. These do not overwrite the network cache.

## Scheduling contract

- Search subject numbers/titles and preserve selected subjects across searches.
- Minimize **occupied days first, then total gaps between classes**, strictly in that order. Weekends count. Adjacent meetings do not conflict.
- Collapse only interchangeable complete time patterns. After optimization, switch actual sections/rooms without changing the timetable or violating pairing restrictions.
- Manual sections use the same normalized model as published sections. Alternative sections may contain several meetings. Keep known components when supplying TBA information.
- Show selected sections, timetable, score, unresolved-time notices, and explicit infeasibility. An interrupted search never claims optimality.
- Export only selected, known, supported meetings. Calendar export uses `America/New_York`, semester bounds, holidays and alternate-weekday dates. Every export filename carries a Unix-time signature, so repeats never overwrite. ICS exports contain meeting details only, without color metadata or color hints. Calendar apps control imported event colors independently of the TUI's per-course colors.
- Infeasible CLI optimization returns explicit text/JSON and exit code **2**, including with `--export`. No calendar is created or modified for an infeasible result.

## Boundaries

**Assume Hydrant's data is accurate.** Fetch its public JSON. No independent scraping, cross-source verification, separate validation service, Kerberos, or login. Schema and user-input validation are local application responsibilities, not a second source of schedule truth.

TBA means missing information. Conflict checks and optimality cover only known, supported options. Scheduling supports full-term academic meetings and fully dated PE offerings. Other date-limited academic options are preserved and explicitly omitted rather than silently treated as full-term. Manual date bounds can be recorded with `--start-date` / `--end-date`. General partial-term academic optimization remains unsupported.

No registration, seats, degree planning, PE credit tracking, travel objective, or cloud sync. The application does not import events into a calendar account or send anything beyond requests for public catalog data.

## Verification

Run `bash scripts/verify-mvp.sh` for the local suite, release regressions, and an isolated installed-binary smoke test. Add `--live` to fetch a fresh Hydrant cache, run all three opt-in checks, and export with the installed binary using live data. This requires Bash and Rust development tools, not Python or a verification service. It never replaces your normal installation or touches calendar accounts. Logs, isolated data, and calendars are retained in a unique directory under `HYDRANT_VERIFY_DIR`, then `JCODE_SCRATCH_DIR` if set, otherwise `target/verification`. The final line prints that directory. Live subjects and the snapshot assertions currently target **f26**.

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked

# Opt-in real public-data and native-terminal acceptance on Unix/macOS.
cargo test --locked --test live_acceptance -- --ignored --nocapture
cargo test --locked --test tui_acceptance -- --ignored --nocapture
```

The solver is checked against independent exhaustive enumeration. Acceptance requires the **actual executable** to complete subject selection → manual section entry → optimization → same-time member switching → local ICS export. Live fetch/cache, offline behavior, terminal restoration, timezone transitions, and failure cases must be checked separately from synthetic solver fixtures. See [VALIDATION.MD](VALIDATION.MD) for observed results, the requirement-to-check mapping, reproduction commands, and remaining limits.
