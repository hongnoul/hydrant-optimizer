# Validation record

## Selected classes and separate timetable: 2026-09-09

This refactor supersedes the prior manual-pane/results-owned-calendar layout. The acceptance map below was defined before the complete verifier replay. Native tests execute the real binary through a PTY, including at 80×24.

| ID | Requirement | Named checks |
|---|---|---|
| S1 | Selected classes replace the permanent Manual entries pane, survive filtering, support removal and clamp selection | `selected_classes_survive_search_and_removal_invalidates_and_clamps`, `selected_results_and_timetable_are_separate_visible_panes`, `actual_tui_selected_classes_survive_search_and_support_remove_reselect` |
| S2 | Selected-class manual prefill and retained manual editing remain available | `selected_class_prefills_manual_editor_and_manual_overlay_closes_without_quitting`, existing manual state tests, `actual_tui_live_selection_editor_solver_member_switch_export_and_restore` |
| T1 | Results and full-width timetable are separate simultaneously visible panes | `selected_results_and_timetable_are_separate_visible_panes`, `actual_tui_week_replay_preserves_exact_times_and_records_observations` |
| T2 | Four-pane focus, independent scroll/reset, resize and stale-result invalidation | `vim_and_arrow_navigation_are_equivalent_in_every_pane`, `timetable_and_results_scroll_independently_and_reset_on_invalidation`, `actual_tui_results_and_timetable_scroll_and_reset_independently` |
| T3 | Preserve precise weekday/weekend times, components, member switching, notices, exports and terminal restoration | all `tui::week::tests`, `actual_tui_pe_search_selection_labels_and_bounded_export`, `actual_tui_week_replay_preserves_exact_times_and_records_observations`, `actual_tui_80x24_reaches_members_all_notices_and_exports`, all existing calendar/PE/CLI tests |
| C1 | Discoverable shortcuts and isolated help/search/editor input | `help_shows_all_new_controls_and_close_hint_at_common_sizes`, `navigation_letters_remain_text_in_search_and_editor`, `help_does_not_navigate_or_edit_a_hidden_pane`, README controls and CLI keymap |

**Observed replay:** `RUST_TEST_NOCAPTURE=1 HYDRANT_UX_BASELINE_BIN=… bash scripts/verify-mvp.sh --live` passed all **16 gates**, **92 local tests**, and **3 opt-in tests** on macOS arm64. Evidence: `$JCODE_SCRATCH_DIR/mvp.sCwmBS/`. Debug and release both ran the historical-binary comparison. The release executable and an isolated installation were rebuilt and exercised.

- **S1:** Native input retained A and B while search hid one, removed/reselected classes using both Space and Enter, and kept the main Manual entries pane absent. State checks removed the final class safely, clamped the cursor to zero, and cleared stale solution/member/export data.
- **S2:** `a` on selected B prefilled B, not the first selected class. `m`, Enter, Escape, Escape edited/cancelled manual data and returned to Selected without quitting or losing two selections. Live persistence, disabled-entry preservation and edit-scope rejection passed.
- **T1:** Both native and renderer checks located the timetable below all three upper panes, asserted a full-width border, and rejected weekly grid content in Results. The calendar retained exactly five weekday columns and five half-hour rows from 09:00 to 11:00 in the exact-time fixture.
- **T2:** Actual input traversed Subjects → Selected → Results → Timetable and back. Native checks verified j/k, arrows, paging, Home/End, and independent `r`/`t` resets. Unit checks verified resize clamping and resetting both scroll positions on selection invalidation. Timetable scrolling did not move Results, and scrolling Results did not move the timetable.
- **T3:** The exact-time replay exported 17 events retaining minute and Sunday-midnight boundaries. At 80×24, the member picker reached 2/2, all U9 notices remained reachable, and six events used Room A. PE exported seven fixture events with one bounded PE event. Live discovery found 38 PE offerings, and seven live PE events stayed within Sep 22–Nov 10. The original live/installed workflow still exported 113 events with terminal restoration and no-clobber checks intact.
- **C1:** Help, top search, new shortcuts as ordinary editor/search text, reverse Tab navigation, and modal isolation all passed. README, architecture notes, footer/help and CLI keymap describe the new layout.

Earlier sections below are historical records, not descriptions of this new layout.

## Whole-result replay of both goals

[REPLAY.MD](REPLAY.MD) records the fresh post-mapping replay of **TUI UX refresh** and **PE and weekday calendar** together on `1a7f9fc`. All **23 requirement groups and 87 named check/evidence pairs** passed against the new logs. The entire 16-gate live verifier ran, including actual historical/current terminal comparison in debug and release. The report names each check and records its observed public output, rather than relying on aggregate test totals. The historical 0-column display became the requested five-column table, the query moved into the top bar, and identical pane-navigation input stopped deselecting a subject. Every PE, date, export, component-label, and preservation check was replayed over the same complete result.

## PE and weekday calendar: 2026-09-09

Verified on **macOS arm64** with the complete `RUST_TEST_NOCAPTURE=1 bash scripts/verify-mvp.sh --live` replay: **86 local tests + all 3 opt-in tests = 89 passing tests**, with **all 16 verification gates passing**. This includes strict Clippy, formatting, locked release build, release PE/CLI/native-terminal regressions, and isolated installed-binary checks. The executable in `target/release` was rebuilt.

Fresh public Hydrant data contained **2,314 selectable offerings**, including **38 PE offerings**. Live PE acceptance selected **`PE.0201.Q1`**, retained its **2026-09-22 through 2026-11-10** bounds, and independently parsed **7 exported events**, all checked against those dates. Existing academic live CLI and native-terminal/installed workflows still exported **50** and **113** events respectively.

| Requirement or changed behavior | Concrete checks and observed result |
|---|---|
| PE appears in search and selection | Five `pe_catalog` tests check quarter-qualified IDs, stable identity across refresh/reordering, titles, IAP, malformed schema, unknown times, and missing dates. Actual terminal input searches Swimming and selects Q1 alongside an academic subject. Installed-binary search returns Q1 and Q2. Fresh live CLI discovery finds 38 PE offerings. All passed. |
| PE scheduling respects dates | Ten `pe_dates` tests check overlapping/disjoint quarters, academic conflicts, adjacency, date-sensitive grouping, missing/invalid bounds, holidays, alternate weekdays, midnight, and manual overlays. An independent Cartesian-product oracle compares mixed academic/PE feasibility, returned choices, and union-week scores. Debug and release passed. |
| PE export stays inside its offering | The terminal fixture exports 7 events with exactly one Q1 PE event on October 26. The mixed two-quarter installed workflow exports 9 events with exactly three PE events on October 26, November 3, and November 9. Live PE independently exports 7 events inside its published range. All parsed/count/date assertions passed. |
| Manual PE defaults and validation | With A and PE selected, `a` on the highlighted PE offering opens its own editor, with `pe` and published dates prefilled. Both editor date fields are asserted from actual terminal output. Validation and persistence reject incomplete bounds or internally conflicting PE meetings and retain supported dated records. Passed. |
| Calendar columns are Monday through Friday | Renderer tests and real 80×24, 120×72, and PE terminal workflows assert exactly five day columns. The seven-day input replay now shows only five half-hour rows from 09:00 to 11:00, explicitly discloses weekends, and still exports all 17 events with exact weekend/midnight times. Passed. |
| Calendar identifies meeting components | Real terminal cells explicitly show `A Lec`, `A Rec`, `A Lab`, and `PE.1000.Q1 PE`. Renderer tests cover narrow/Unicode cells, subject truncation before component labels, other component kinds, shared buckets, selected highlighting, and complete short legends. Different-course shared buckets retain correct subject/component pairs or an explicit summary, never a misleading independently sorted pairing. Passed. |
| Preserve all prior workflows | The full suite retains academic exactness/oracle coverage, unsupported non-PE date disclosure, manual edits and refresh preservation, member switching, scrolling, search/navigation, overwrite protection, infeasible export, network fallback, and terminal restoration. All existing live workflows passed again. |

The feedback loop fixed the manual editor defaulting to the first selected academic subject instead of the highlighted PE offering, missing spacing after long PE IDs, ambiguous shared-bucket labels, and clipped legends. Terminal checks also caught fixture invocation and modal synchronization errors in the new harness. The entire verifier was rerun after the fixes, rather than only the new tests.

**Scope:** PE conflicts and export use actual bounded calendar dates. Its score/grid deliberately use the combined weekly pattern across selected quarters, with an explicit notice. General partial-term academic optimization, PE registration/credit tracking, other-platform verification, and calendar-account import remain outside this result. The older records below describe earlier commits and their then-current seven-day display and narrower date scope.

## TUI UX refresh: 2026-09-09

Implementation commit: **`2cc76fa`**. Complete-result replay commit: **`fc79e1e`**. Verified on **macOS arm64** with `bash scripts/verify-mvp.sh --live`.

- **62 local tests passed**, plus **all three opt-in tests**, for **65 passing tests**. Format, strict Clippy, locked release build, release CLI/native-terminal regressions, and isolated installation checks all passed.
- The actual **80×24 release executable** displays a full-width Monday–Sunday table with 30-minute rows. Its native-terminal test exercises the top search bar, both navigation key families, same-time member switching, complete notices, export, and terminal restoration.
- The live native-terminal workflow still selects subjects, edits manual entries, optimizes, switches members, and exports **113 events**. Stale/duplicate export rejection, disabled-entry preservation, reoptimization, selection across multiword searches, and terminal restoration passed.
- An isolated installed executable independently exported **113 events** from freshly fetched public Hydrant data. No calendar account or normal user installation was changed.

| Requested behavior | Concrete checks and observed result |
|---|---|
| Weekly table with 30-minute rows and weekday columns | Eleven `tui::week` tests cover all headers, row spacing, day placement, exact-minute intersection, adjacency, shared buckets, Sunday midnight, highlighting, empty input, and Unicode/narrow widths. Integrated renderer and actual 80×24 PTY checks confirm the table is visible at full width. All passed. |
| Search replaces the top status panel | Renderer and PTY assertions inspect the first three rows for the search input and absence of the old Status header. Long Unicode queries keep their tail and caret visible on resize. Searching retains selections and treats `hjkl` as text. All passed. |
| `hjkl` and arrow navigation | State tests exercise both vertical key families in every pane and both horizontal key families through the full focus cycle. Native input verifies pane markers, subject movement, and result-section movement. Pane navigation does not change actual members. Search/editor/help mode isolation and complete help text passed. |
| Preserve existing workflows | Existing optimizer/calendar/data/CLI tests, result scrolling and all-notice reachability, local/release PTY export, and live manual-entry/member/export workflow passed. The display grid does not round solver or exported meeting times. |

The review/verification loop caught and fixed a search bar squeezed out at small heights and a wrapped help assertion. The final help is a readable command list, and a read-only integration review found no remaining actionable issues. Platform verification remains scoped to macOS.

### Complete-result replay and measured improvement

The final replay explicitly reruns **the full suite covering this matrix**, not just the new renderer tests. A new native acceptance test, `actual_tui_week_replay_preserves_exact_times_and_records_observations`, drives the actual CLI and TUI with the same deterministic seven-day fixture and **120×72** terminal for both the refreshed executable and an isolated release build of pre-refresh commit **`149c684`**. It emits asserted `UX_OBSERVATION` and `UX_COMPARISON` JSON records. The existing **80×24** native test also emits an observation record after completing its workflow.

| Requirement or changed public output | Replay check | Concrete observation |
|---|---|---|
| Weekly schedule structure | Native seven-day replay plus all eleven `tui::week` tests | The old executable had no day-column/half-hour table. The new executable displays **7 day columns** and exactly **30 rows from 09:00 through 23:30** for this fixture. Every expected row and day cell is checked from the terminal output. |
| Exact placement, shared buckets, and legend | Native replay cell assertions | Monday shows **`2×W,X`** for adjacent 09:05–09:20 and 09:20–09:25 meetings. The legend explains shared buckets. Tuesday occupies 09:30 and 10:00, but not 10:30. Saturday and Sunday are present, including Sunday 23:35–24:00 in the 23:30 cell. The actual solver reports **7 occupied days, 0 gap minutes**, not a false conflict. |
| Search location, metadata, selection, empty filtering, caret | Native replay, 80×24 PTY, search renderer/state tests | Old search text appears in the Subjects border while Status occupies the top. New query **`week`** appears in the first three rows beside the Search heading, with term **f26**, no Status heading, and **2 subjects still selected**. `hjkl` is entered as text. Renderer checks confirm zero-match count and visible Unicode query tail/caret after resizing. |
| Pane and item navigation | Both native tests and `vim_and_arrow_navigation_are_equivalent_in_every_pane` | At 80×24, actual keys traverse **Subjects → Manual → Results → Manual → Subjects**. `j/k` and Up/Down move subject/result rows. State tests exercise both vertical families in all three panes. On identical before/after input, `l` then Enter previously deselected a subject because focus did not move. It now opens the manual editor with both subjects still selected. |
| Section picker, `r`, scrolling, notice count/position | 80×24 PTY plus all viewport regressions | `n` shows member **2/2**. `r`/Home return to the weekly grid, PgUp/PgDn page independently of section selection, and End reaches the final **U9** unresolved and export notices. All 12 source/export notices and long action/output text are reachable in renderer checks. Resize and invalidation clamp/reset scroll offsets. |
| Exact-time details, rooms, and ICS output | Native seven-day replay through `e`, independent ICS parser | Details retain **09:05–09:20**, **09:35–10:05**, and **23:35–24:00**. Switching members exports the chosen room and **17 events**. Exact UTC boundaries including `20261026T130500Z` and the Sunday-to-Monday midnight ending `20261109T050000Z` are asserted. The 80×24 workflow independently exports **6 events** with **Room A**. |
| Help and modal/input boundaries | Native help checks, common-size renderer checks, editor/search/help state tests | The new h/l and j/k help entries, paging instructions, member controls, and closing hint fit at **80×24**, **120×32**, and **170×55**. Navigation letters remain editor/search text. Help cannot mutate a hidden pane. |
| Existing solver/data/CLI/manual/export and terminal safety | Complete `verify-mvp.sh --live` run | Every existing oracle, parser, storage, CLI, renderer/state, and native test, all three opt-in tests, strict lint, release build, and isolated installation passed. Live and installed workflows again produced **113 events**, preserving manual records and overwrite protection and restoring termios/alternate-screen state. |

**Final feedback-loop outcome:** the full post-mapping replay passed on `fc79e1e`: **all 14 verification gates, 62 local tests, and 3 opt-in tests**. The observed before/after improvement above passed in **both debug and release native-terminal runs**, not just a renderer or inspection. The exact-minute fixture exported 17 events, the compact workflow exported 6, and the live/installed workflow exported 113. The verifier's `tests.stdout` and `release-regressions.stdout` contain the per-requirement JSON observations, while `opt-in.stdout` records the preserved live behaviors. No production fixes were needed after this complete replay. This closes the loop for the specified layout/navigation behaviors, without claiming measured user preference or verification on another platform.

Reproduce the comparison as part of the **whole-result** verifier:

```sh
scratch=${JCODE_SCRATCH_DIR:-$PWD/target}
mkdir -p "$scratch"
baseline=$(mktemp -d "$scratch/ux-baseline.XXXXXX")
git archive 149c684 | tar -x -C "$baseline"
cargo build --release --locked --manifest-path "$baseline/Cargo.toml" --target-dir "$baseline/target"
HYDRANT_UX_BASELINE_BIN="$baseline/target/release/hydrant-optimizer" \
  RUST_TEST_NOCAPTURE=1 bash scripts/verify-mvp.sh --live
```

Without `HYDRANT_UX_BASELINE_BIN`, the new seven-day current-executable acceptance always runs. Only the optional historical comparison is omitted. Both variants use isolated temporary data/output and never export into a calendar account.

## Original MVP verification

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

A clean-source relocation check exposed a test-driver assumption: with a long checkout/output path, the visible phrase `never overwritten` wrapped across terminal rows. The native assertion now accepts that legitimate wrapping while still requiring export failure and byte-identical calendar preservation. Portability claims remain limited to the macOS handoff, not untested platforms.

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
- Current scheduling supports full-term academics and fully dated PE. Other partial-term academic data remains retained and disclosed but omitted, not treated as full-term. PE scoring uses a combined weekly pattern, not an average or total of per-week costs.
- Calendar files were parsed independently and checked by date/time assertions. They were not imported into a calendar account or verified in every calendar client.
- Normal terminal restoration and recovery from export errors were exercised. Forced termination and injected panics were not acceptance-tested. Cancellation semantics were tested at the actual solver API, not with a timed long-running native-terminal workload.
- Exactness evidence is the independent oracle and explicit edge cases. No broad performance benchmark or claim that this is the fastest solver is made.
