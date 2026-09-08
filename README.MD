# Hydrant Optimizer

A local **Rust** MIT course scheduler. Select a fixed set of subjects, supply missing section times, find an exact compact weekly timetable, and export a local `.ics` file. No Python runtime, server, or solver service is required.

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

# Export the selected schedule. Existing files are never overwritten.
hydrant-optimizer --offline --output my-schedule.ics optimize 6.1200 18.01 --export

# Machine-readable results include same-time members and their stable IDs.
hydrant-optimizer --offline --json optimize 6.1200 18.01
hydrant-optimizer --offline optimize 6.1200 18.01 \
  --member '6.1200/recitation=OPTION_ID'
```

Use `./target/release/hydrant-optimizer` in place of `hydrant-optimizer` unless installed on `PATH`. `cargo install --path . --locked` installs the executable locally. Subject availability depends on the downloaded term. Run `--help` and `manual --help` for the full CLI.

### Terminal controls

| Key | Action |
|---|---|
| `/` | Search subjects. Ctrl+U clears the existing query. Type a number/title, then Enter to return to the list. |
| Up/Down or `k`/`j` | Move through the focused list. Space/Enter toggles a subject. |
| Tab / Shift+Tab | Cycle Subjects, Manual, Results panels. |
| `a` / `m` | Add a manual section / focus retained manual entries. |
| Enter / `x` in Manual | Edit an entry / toggle enabled state. |
| `o` | Optimize in the background, or cancel an already running search. |
| `c` | Cancel a running search. No optimum is claimed for cancelled work. |
| `r`, then `n`/`p` or Right/Left | Focus results and switch an interchangeable actual section. |
| PgUp/PgDn in Results | Scroll the timetable, section picker, notices, and export details. |
| Home/End in Results | Jump to the first/last result line. Up/Down selects a section and keeps it visible. |
| `e` | Export the chosen sections to `--output`. |
| `?` / `q` | Help / quit. Escape closes an editor or quits outside it. |

The manual editor's fields are Subject, Component, Section label, Meetings, Room, Start date, End date. Tab/Shift+Tab changes fields, Ctrl+U clears one, Enter or Ctrl+S saves, and Escape cancels. Enter multiple meetings separated by semicolons. Selection survives catalog searches. Selection or manual changes invalidate the previous timetable until re-optimization.

Results are scrollable at **80×24**, including all unresolved-time, source/cache, and export notices. The results heading shows the notice count, and the bottom border shows the scroll position. Long action messages and output paths remain available in the scrollable view.

### Local files

- macOS default: `~/Library/Application Support/hydrant-optimizer`.
- Other Unix default: `~/.local/share/hydrant-optimizer`.
- `$XDG_DATA_HOME/hydrant-optimizer` takes precedence when set.
- `--data-dir PATH` overrides the location for all commands.
- Downloaded cache and versioned `manual.json` are separate. Refreshing never rewrites manual sections.
- Manual entries are scoped by term, subject, component, and stable option ID. `manual list` works even without a cache. `manual disable ID` and `manual enable ID` control retained entries. `manual add --id ID ...` edits an existing entry, preserving its scope and enabled state. Components can be lecture, recitation, lab, design, or another component published for that subject.
- For reproducible inputs, `--catalog FILE --term FILE` reads local Hydrant `latest.json` and matching `latestTerm.json` snapshots through the same adapter. These do not overwrite the network cache.

## Scheduling contract

- Search subject numbers/titles and preserve selected subjects across searches.
- Minimize **occupied days first, then total gaps between classes**, strictly in that order. Weekends count. Adjacent meetings do not conflict.
- Collapse only interchangeable complete time patterns. After optimization, switch actual sections/rooms without changing the timetable or violating pairing restrictions.
- Manual sections use the same normalized model as published sections. Alternative sections may contain several meetings. Keep known components when supplying TBA information.
- Show selected sections, timetable, score, unresolved-time notices, and explicit infeasibility. An interrupted search never claims optimality.
- Export only selected, known, supported meetings. Calendar export uses `America/New_York`, semester bounds, holidays and alternate-weekday dates. Existing output files are protected.
- Infeasible CLI optimization returns explicit text/JSON and exit code **2**, including with `--export`. No calendar is created or modified for an infeasible result.

## Boundaries

**Assume Hydrant's data is accurate.** Fetch its public JSON. No independent scraping, cross-source verification, separate validation service, Kerberos, or login. Schema and user-input validation are local application responsibilities, not a second source of schedule truth.

TBA means missing information. Conflict checks and optimality cover only known, supported options. The MVP models full-term weekly meetings. Date-limited options are preserved and explicitly omitted from optimization rather than silently treated as full-term. Manual date bounds can be recorded with `--start-date` / `--end-date`, but partial-term optimization is not supported.

No registration, seats, degree planning, PE planning, travel objective, or cloud sync. The application does not import events into a calendar account or send anything beyond requests for public catalog data.

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
