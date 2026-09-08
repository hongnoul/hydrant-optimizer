# Validation record

Verified on **2026-09-08**, macOS **26.6.2**, **arm64**, Rust **1.98.0**. This records checks against this repository's real library and executable, not the earlier isolated feasibility probes.

**MVP complete within the documented full-term weekly scheduling scope.** The two remaining blockers, inaccessible small-terminal results/notices and lost CLI infeasibility with `--export`, are fixed and regression-tested. Additional platform, performance, and release-automation work is separate from MVP completion.

## Observed results

- `cargo fmt --check`: passed.
- `cargo test --locked`: **44 passed**, zero failures. Three environment-dependent tests were ignored by default and explicitly run below.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- The real f26 snapshot adapter test: **1 passed**, using downloaded `latest.json` and `latestTerm.json` files.
- Release-mode live CLI acceptance: **1 passed**. Public Hydrant returned **2,276 subjects** for **f26**. Subject `6.1200` optimized to **4 occupied days, 0 gap minutes**, with **50 exported events**. Manual records survived refresh byte-for-byte. Switching to a same-time manual member preserved the score and exported its room. Offline use, cache fallback with age, failed-refresh preservation, JSON-search warning visibility, and overwrite rejection passed.
- Release-mode native-terminal acceptance: **1 passed**. The actual executable ran in a native Unix PTY. Selected `6.1200` and `18.01` across separate searches, used the manual editor, optimized to **5 occupied days, 60 gap minutes**, switched the actual section, and exported **113 events**. A second export was rejected without changing the file. Both the real terminal's termios and alternate-screen mode were restored on exit.
- Native **80×24** regression: **1 passed**, in both debug and release. The actual executable exposed the timetable, same-time member picker, final unresolved notice, and final wrapped export omission. Switching members exported **6 events** with the chosen room, and terminal restoration passed.
- `cargo build --release --locked`: passed. The native Mach-O arm64 executable was approximately **7.7 MiB**.
- `cargo install --path . --locked --offline --root <temporary-directory>`: passed. With an isolated environment and `PATH=/usr/bin:/bin`, the installed binary printed its version and optimized/exported the downloaded two-subject snapshot to **113 events**. Rust toolchain directories were excluded from PATH. The normal user installation and calendar accounts were not changed.

That is **47 passing tests including the three opt-in checks**, plus release build and isolated installation checks. The full requirement replay also rechecked data-directory precedence, cache-independent manual listing, local-snapshot cache preservation, text/JSON/help outputs, and infeasible export through the installed binary.

## Requirement-to-evidence mapping

| Requirement | Concrete check and result |
|---|---|
| Rust handoff and standalone local application | README, ARCHITECTURE, and OPTIMIZER describe the implemented module boundaries. Locked release build and isolated installed-binary run passed. |
| Search numbers/titles and retain subject selections | `subject_search_selection_persists_across_filters`, multiword-search/Ctrl+U regression, and live PTY selection across two queries passed. |
| Minimize occupied days before gaps, exactly | `optimizer_exact` compares score and returned assignment against an independent Cartesian-product oracle using interval comparisons and independent gap computation. Day priority, later gap filling, and equal-day subsets passed. |
| Complete atomic meeting bundles and exact times | Oracle checks cover adjacency, overlap, multi-day bundles, different durations, exact-minute manual times, week/word boundaries, and midnight. Parser and model bound checks passed. |
| Safe interchangeable actual-section selection | Grouping tests retain members and distinguish inbound/outbound incompatibilities. App-boundary tests reject incompatible final members before output creation. CLI and live PTY switching preserve the timetable and export the chosen room. |
| Manual entry, editing, persistence, and source separation | CLI round-trip and storage tests passed for stable IDs, known components, custom published component kinds, term scoping, independent files, disabled edits, rejected scope changes, and invalid-write protection. Live refresh preserved manual bytes. |
| TBA/unsupported data and explicit outcomes | Empty/unknown-only inputs remain unresolved, supported conflict domains return infeasible, invalid input is an error, and a set cancellation flag never produces an optimum. CLI infeasibility exits 2 with or without `--export`, preserving explicit text/JSON and notices without touching output. Date-limited entries remain stored and visibly omitted. |
| Accessible results and complete disclosure | Actual 80×24 PTY regression reaches the section picker and the final unresolved/export notices. Renderer tests reach every source/export notice, long action/output text, and verify wrapped scrolling, resize bounds, section navigation, and invalidation. |
| Validated catalog, cache, and failure behavior | Real f26 parsing, corrupt metadata, integer overflow, blank titles, duplicate API sections, official partial-term dates, offline missing/corrupt cache, and failed-refresh cache preservation passed. A child-only unreachable proxy exercised real HTTP failure without changing machine network settings. |
| Correct dated calendar expansion | `data_calendar` checks semester limits, holidays, alternate weekdays, New York DST offsets, deterministic UIDs, explicit omissions, next-day midnight, UTF-8 75-octet folding, CR/LF escaping, duplicate events, and invalid calendars. Ambiguous/nonexistent local times are rejected. Independent `icalendar` parsing passed. |
| Output and terminal safety | Existing ICS content remains byte-identical after CLI/TUI overwrite attempts. Invalid member or missing calendar creates no output. Live PTY verifies a failed export remains recoverable and normal exit restores termios/alternate screen. |

### Native acceptance follow-up

A follow-up audit exercised the review fixes through the **actual release executable**, live Hydrant data, real PTY keyboard events, and persisted output files. These are additional assertions within `actual_tui_live_selection_editor_solver_member_switch_export_and_restore`, not replacement state-machine fixtures.

| Public behavior | Action and observed result |
|---|---|
| Multiword title search preserves selection | Typed `mathematics for` into the actual search field. The complete query appeared, including its space, and the displayed selected-subject count remained **2**. |
| Disabling invalidates the previous timetable | Disabled the manual entry and attempted export. The UI reported **optimize before exporting**. The existing ICS remained byte-identical. |
| Editing does not silently re-enable a section | Opened the disabled entry's editor, changed its label, and saved. `manual.json` contained the revised label, the same option ID, and `enabled: false`. |
| Editing cannot move an entry to another subject | Changed the existing entry's subject to `18.01` and attempted save. The UI reported **manual edits cannot move**. Persisted manual data remained byte-identical to the last valid save. |
| Validation errors are recoverable | Cancelled the rejected edit, explicitly enabled the entry, and optimized again. The UI returned to **5 occupied days, 60 gap minutes**. |
| Exit remains safe after the extended workflow | Quit after all those actions. The child exited successfully, the original termios settings matched, and the alternate-screen leave sequence was observed. |

All six observations passed in the same live session that selected the subjects, saved an atomic manual bundle, switched an actual same-time member, and exported **113 events**. The audit did not require further application changes, only stronger direct acceptance assertions for the earlier fixes.

### MVP completion acceptance

Both new actual-executable regressions first failed against the previous implementation. After the fixes, the complete offline suite, all three opt-in checks, release CLI/small-terminal regressions, and isolated installation were rerun against the committed result. Existing live CLI and manual-editor workflows still passed. The 44-requirement review matrix was replayed with the two failed product behaviors changed to observed passes, retaining its explicit platform and hardening limits.

| Changed behavior | Concrete check and observed result |
|---|---|
| Infeasible `--export`, text | `infeasible_export_preserves_status_and_never_touches_output`: explicit `Infeasible:` output, unresolved recitation warning, exit **2**. Both missing and existing output paths remain untouched. |
| Infeasible `--export`, JSON | Same regression: `status: infeasible`, null score/export, empty sections, retained unresolved notices, exit **2**. Existing calendar bytes remain identical. Installed-binary infeasible export also preserves the prior 113-event calendar. |
| Small-terminal results and member switching | `actual_tui_80x24_reaches_members_all_notices_and_exports`: `r` reveals the selected section, `n` changes to member **2/2**, arrows reveal either section, and export independently parses to **6 events**, including **Room A**. |
| Complete unresolved and export notices | Same native test reaches the last of **10** added unknown-subject notices and its wrapped export omission, beyond both old notice caps. |
| Page and section navigation remain separate | `result_pages_and_member_navigation_keep_independent_positions`: PgUp/PgDn and Home/End move the viewport without changing the selected section. Rendering preserves deliberate scrolling, while section/member navigation reveals the active row. Native key sequences also pass. |
| Wrapping, resizing, and stale results | `result_wrapping_resize_and_invalidation_clamp_scroll`: long Unicode labels remain navigable, scrolling reaches the last notice, resizing clamps offset, and selection changes clear stale result/scroll state. |
| Source notices, long messages, output path, and indicators | `source_and_export_notices_are_uncapped_and_reachable`: page through the real renderer and find all **12 source + 12 export notices**, full long action/path endings, notice count **24**, and scroll-position hints. Source notices are reachable before optimization. |
| Discoverable controls at 80×24 | Native help test displays PgUp/PgDn and Home/End instructions without clipping. The compact footer and results border retain scroll/navigation hints. |

## Reproduce

The repository includes `bash scripts/verify-mvp.sh --live` to run build/lint, all tests, fresh-cache snapshot acceptance, real CLI/PTY workflows, and an isolated installed-binary export without session-specific paths. Without `--live`, public-data checks are skipped explicitly and local fixture/PTY/install checks still run. It retains stdout/stderr, exit-code records, isolated cache, and exported calendars, reporting the evidence directory on success or failure. Rust dependency downloads may still require network access when not cached. Existing manual snapshot commands below remain supported.

Handoff checks passed with macOS `/bin/bash` in both local and live modes, including an evidence path containing spaces. The installed fixture workflow produced **6 events**, the live workflow **113 events**, and infeasible text/JSON exports preserved existing bytes and created no missing output. Help exited 0 and malformed arguments exited 2 without creating artifacts. An injected Cargo failure stopped the runner with exit 1 before installation, retaining the underlying exit 77 and its diagnostics. Evidence includes start time and Git revision/worktree status when Git is available. These runner checks supplement the 47 Rust tests, not replace them.

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked

# New actual-executable regressions, with local fixtures and no network.
cargo test --release --locked --test cli_workflow --test tui_acceptance

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
