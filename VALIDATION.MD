# Validation record

Verified on **2026-09-08**, macOS **26.6.2**, **arm64**, Rust **1.98.0**. This records checks against this repository's real library and executable, not the earlier isolated feasibility probes.

## Observed results

- `cargo fmt --check`: passed.
- `cargo test --locked`: **39 passed**, zero failures. Three environment-dependent tests were ignored by default and explicitly run below.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- The real f26 snapshot adapter test: **1 passed**, using downloaded `latest.json` and `latestTerm.json` files.
- Release-mode live CLI acceptance: **1 passed**. Public Hydrant returned **2,276 subjects** for **f26**. Subject `6.1200` optimized to **4 occupied days, 0 gap minutes**, with **50 exported events**. Manual records survived refresh byte-for-byte. Switching to a same-time manual member preserved the score and exported its room. Offline use, cache fallback with age, failed-refresh preservation, JSON-search warning visibility, and overwrite rejection passed.
- Release-mode native-terminal acceptance: **1 passed**. The actual executable ran in a native Unix PTY. Selected `6.1200` and `18.01` across separate searches, used the manual editor, optimized to **5 occupied days, 60 gap minutes**, switched the actual section, and exported **113 events**. A second export was rejected without changing the file. Both the real terminal's termios and alternate-screen mode were restored on exit.
- `cargo build --release --locked`: passed. The native Mach-O arm64 executable was approximately **7.7 MiB**.
- `cargo install --path . --locked --offline --root <temporary-directory>`: passed. With an isolated environment and `PATH=/usr/bin:/bin`, the installed binary printed its version and optimized/exported the downloaded two-subject snapshot to **113 events**. Rust toolchain directories were excluded from PATH. The normal user installation and calendar accounts were not changed.

That is **42 passing tests including the three opt-in checks**, plus release build and isolated installation checks.

## Requirement-to-evidence mapping

| Requirement | Concrete check and result |
|---|---|
| Rust handoff and standalone local application | README, ARCHITECTURE, and OPTIMIZER describe the implemented module boundaries. Locked release build and isolated installed-binary run passed. |
| Search numbers/titles and retain subject selections | `subject_search_selection_persists_across_filters`, multiword-search/Ctrl+U regression, and live PTY selection across two queries passed. |
| Minimize occupied days before gaps, exactly | `optimizer_exact` compares score and returned assignment against an independent Cartesian-product oracle using interval comparisons and independent gap computation. Day priority, later gap filling, and equal-day subsets passed. |
| Complete atomic meeting bundles and exact times | Oracle checks cover adjacency, overlap, multi-day bundles, different durations, exact-minute manual times, week/word boundaries, and midnight. Parser and model bound checks passed. |
| Safe interchangeable actual-section selection | Grouping tests retain members and distinguish inbound/outbound incompatibilities. App-boundary tests reject incompatible final members before output creation. CLI and live PTY switching preserve the timetable and export the chosen room. |
| Manual entry, editing, persistence, and source separation | CLI round-trip and storage tests passed for stable IDs, known components, custom published component kinds, term scoping, independent files, disabled edits, rejected scope changes, and invalid-write protection. Live refresh preserved manual bytes. |
| TBA/unsupported data and explicit outcomes | Empty/unknown-only inputs remain unresolved, supported conflict domains return infeasible, invalid input is an error, and a set cancellation flag never produces an optimum. CLI infeasibility exits 2. Date-limited entries remain stored and visibly omitted. |
| Validated catalog, cache, and failure behavior | Real f26 parsing, corrupt metadata, integer overflow, blank titles, duplicate API sections, official partial-term dates, offline missing/corrupt cache, and failed-refresh cache preservation passed. A child-only unreachable proxy exercised real HTTP failure without changing machine network settings. |
| Correct dated calendar expansion | `data_calendar` checks semester limits, holidays, alternate weekdays, New York DST offsets, deterministic UIDs, explicit omissions, next-day midnight, UTF-8 75-octet folding, CR/LF escaping, duplicate events, and invalid calendars. Ambiguous/nonexistent local times are rejected. Independent `icalendar` parsing passed. |
| Output and terminal safety | Existing ICS content remains byte-identical after CLI/TUI overwrite attempts. Invalid member or missing calendar creates no output. Live PTY verifies a failed export remains recoverable and normal exit restores termios/alternate screen. |

## Reproduce

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked

# Actual public network and native Unix PTY, against the release executable.
cargo test --release --locked --test live_acceptance --test tui_acceptance -- --ignored --nocapture

# Optional additional check for a retained, matching f26 snapshot.
HYDRANT_REAL_CATALOG=/path/to/latest.json \
HYDRANT_REAL_TERM=/path/to/latestTerm.json \
  cargo test --locked --test data_calendar real_f26 -- --ignored --nocapture
```

The live acceptance cases deliberately require their named subjects to exist in the current term. A later term can require updating those inputs rather than substituting a synthetic result for a failed live check.

## Fixes found by the feedback loop

The first real terminal run exposed incorrect driver input: entering another search retained the old query. The driver now sends Ctrl+U, and the documented query-reset behavior is explicit. Separately, a real UI defect prevented multiword title queries because Space toggled selection while searching. That was fixed and regression-tested.

Review also found that CLI manual editing re-enabled disabled records and allowed scope changes. CLI and TUI edits now preserve enabled state and reject scope moves. Final member resolution defensively rejects pairing incompatibilities. Calendar review found the valid `24:00` boundary, line folding, and newline escaping needed correction. New regressions passed after those fixes.

## Limits of the evidence

- Only macOS arm64 packaging and native terminal behavior were exercised. No Linux, Windows, cross-compilation, or minimum-toolchain result is claimed.
- Full-term weekly optimization is the MVP boundary. Partial-term data is retained and disclosed but omitted, not treated as full-term.
- Calendar files were parsed independently and checked by date/time assertions. They were not imported into a calendar account or verified in every calendar client.
- Normal terminal restoration and recovery from export errors were exercised. Forced termination and injected panics were not acceptance-tested. Cancellation semantics were tested at the actual solver API, not with a timed long-running native-terminal workload.
- Exactness evidence is the independent oracle and explicit edge cases. No broad performance benchmark or claim that this is the fastest solver is made.
