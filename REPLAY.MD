# Whole-result feedback-loop replay

> Historical record: the Results pane was subsequently removed on 2026-09-09. Current navigation has Subjects, Selected classes, and Timetable only. Results-specific assertions and shortcuts below describe the earlier UI. Current coverage includes `subjects_and_selected_share_upper_row_without_results`, `removed_results_shortcuts_do_not_focus_a_hidden_pane`, `actual_tui_timetable_scrolls_and_resets_independently_of_subject_lists`, and `actual_tui_80x24_navigates_sessions_and_exports_with_unknown_subjects`.

## Scope and execution

This record closes **both** the TUI UX refresh and PE/weekday-calendar goals on the combined application, not on isolated changes. The latest Monday–Friday requirement supersedes the earlier seven-column display requirement. Weekend data must still survive in exact details, optimization, and export.

Before execution, the checks below were frozen as **23 requirement groups and 87 requirement/check pairs** in `requirement-checks.tsv`. The complete verifier then ran on clean application commit **`1a7f9fc`** at **2026-09-09 01:05 UTC**, with an executable built from archived **`149c684`** source enabled for the historical comparison. Every frozen pair was individually matched against the new logs. All 87 matches passed. No application or test changes were needed in this replay.

The full run also passed **86 local tests, all 3 opt-in tests, and all 16 verification gates**. Those totals supplement, rather than replace, the per-requirement observations below. The replay includes debug tests, release native-terminal/CLI/PE tests, fresh Hydrant data, strict Clippy, formatting, release packaging, and an isolated installation.

Retained evidence is under `$JCODE_SCRATCH_DIR/hydrant-whole-replay.v06YtR/`: `requirement-checks.tsv` predates execution, `trace-results.tsv` records every matched pair, and `mvp.jtN3uM/` contains the complete stdout/stderr, `checks.tsv`, tested revision, empty worktree record, fresh data, and exported calendars. Both `tests.stdout` and `release-regressions.stdout` contain an asserted `UX_COMPARISON`, so the optional historical comparison demonstrably ran in both configurations.

## Exact check names

The tables use these unambiguous abbreviations:

- `week::name` = `tui::week::tests::name` in [src/tui/week.rs](src/tui/week.rs).
- `view::name` = `tui::viewport_tests::name` in [src/tui.rs](src/tui.rs).
- `state::name` = `name` in [tests/tui_state.rs](tests/tui_state.rs).
- `pe::name` = `name` in [tests/pe_catalog.rs](tests/pe_catalog.rs).
- `dates::name` = `name` in [tests/pe_dates.rs](tests/pe_dates.rs).
- **native-grid** = `actual_tui_week_replay_preserves_exact_times_and_records_observations`.
- **native-small** = `actual_tui_80x24_reaches_members_all_notices_and_exports`.
- **native-PE** = `actual_tui_pe_search_selection_labels_and_bounded_export`.
- **native-live** = `actual_tui_live_selection_editor_solver_member_switch_export_and_restore`.
- All four native cases are in [tests/tui_acceptance.rs](tests/tui_acceptance.rs). **live-CLI** is `live_catalog_manual_refresh_optimize_switch_and_export` in [tests/live_acceptance.rs](tests/live_acceptance.rs). Other fully named checks below are in the stated test suite. Installed checks are the named stages in [scripts/verify-mvp.sh](scripts/verify-mvp.sh).

## TUI UX refresh: observed requirements and outputs

| ID and requirement | Concrete checks executed | Observed result in this replay |
|---|---|---|
| U1. Full-width calendar, day columns, 30-minute rows and placement | native-grid; native-small; `view::search_replaces_status_header_and_week_has_full_width_at_80_columns`; `week::rows_are_spaced_every_thirty_minutes`; `week::occupancy_appears_on_correct_day_and_rows` | The actual 120×72 terminal has exactly Mon–Fri and five rows from 09:00 through 11:00 for the replay input, with every occupied day/slot asserted. The 80×24 executable exposes the full-width table. The historical executable has **zero** day-column or half-hour table rows for identical input. |
| U2. Search replaces Status, with term/query/counts and selection retention | native-grid; native-small; `state::subject_search_selection_persists_across_filters`; `state::search_accepts_spaces_for_multiword_titles_and_ctrl_u_clears`; `view::long_unicode_search_keeps_caret_and_tail_visible_after_resize` | Query `week` appears in the first three rows with f26 and **2 selections**, without Status. The historical query is in the Subjects border, below a Status header. Multiword search, Ctrl+U, zero-result filtering, and visible Unicode tail/caret after resize pass their assertions. |
| U3. h/l and arrows change panes, j/k and arrows navigate items | native-grid; native-small; `state::vim_and_arrow_navigation_are_equivalent_in_every_pane` | Actual focus traverses **Subjects → Manual → Results → Manual → Subjects**. j/k and Up/Down move subject/result rows, with state checks covering every pane. On identical `l` then Enter input, the old executable deselects a subject, while the new one opens its Manual editor and keeps **2 selections**. |
| U4. Search/editor/help modes consume input correctly and expose controls | native-small; `state::navigation_letters_remain_text_in_search_and_editor`; `state::help_does_not_navigate_or_edit_a_hidden_pane`; `state::editor_key_handling_tabs_fields_accepts_text_and_requests_save`; `view::help_shows_all_new_controls_and_close_hint_at_common_sizes` | `hjkl` is entered as search text without changing selection. Editor text/Tab/save and hidden-pane isolation pass. Help exposes paging, navigation, member controls, and the closing hint at 80×24, 120×32, and 170×55. |
| U5. Same-time member picker remains separate from navigation | native-small; `state::same_time_member_switching_updates_mapping_and_selection_invalidates_result` | `n` reaches member **2/2**. Arrow/j/k navigation selects the component instead of changing its actual member. Export contains **Room A**, with **6 events**. Selection changes invalidate the previous result. |
| U6. All result/source/export notices, paths and scroll indicators remain reachable | native-small; `view::result_pages_and_member_navigation_keep_independent_positions`; `view::result_wrapping_resize_and_invalidation_clamp_scroll`; `view::source_and_export_notices_are_uncapped_and_reachable` | r/Home return to the grid, PgUp/PgDn page independently of section selection, and End reaches both final **U9** notices. All **12 source + 12 export notices** and long action/output paths are reachable in renderer assertions. Resize clamps and invalidation resets the viewport. |
| U7. Display buckets do not round actual intervals or create false conflicts | native-grid; `week::exact_minute_intersection_fills_each_intersecting_slot`; `week::adjacency_and_end_boundaries_do_not_create_false_overlap` | Details retain **09:05–09:20**, **09:35–10:05**, and Sunday **23:35–24:00**. The shared Monday cell reads **`2×W Lec/X Lec`**, not a conflict. Solver score remains **7 days, 0 gap minutes**. Export has **17 events**, selected room `Week room one`, and asserted exact UTC boundaries including the next-day midnight end. |
| U8. Highlighting, narrow/Unicode widths and empty states remain valid | `week::selected_requirement_is_highlighted`; `week::selected_style_and_unicode_width_are_preserved`; `week::combining_marks_emoji_and_zwj_are_measured_by_ratatui_width`; `week::width_below_thirty_seven_shows_resize_hint_not_ambiguous_days`; `week::empty_input_is_graceful` | Selected styling survives rendering. Measured lines remain within the tested Unicode/narrow widths. Insufficient width produces a resize hint instead of ambiguous day columns, and empty input produces a meaningful empty state. |
| U9. Native operation and terminal restoration survive the whole workflow | native-grid; native-small; native-PE; native-live | All actual-terminal workflows exit successfully and restore termios/alternate-screen state. Live selection, manual editing, solving and switching still export **113 events**. |

**Measured improvement, not an inspection judgment:** both actual executables received the same fixture and 120×72 terminal. The new one changed 0 table columns/rows to 5/5, moved the query into the top bar, and changed a mistaken deselection into the intended pane/editor action. Those differences are asserted and emitted by `UX_COMPARISON` in debug and release. This establishes improvement against the specified requirements, not a claim about unmeasured user preference.

## PE and weekday calendar: observed requirements and outputs

| ID and requirement | Concrete checks executed | Observed result in this replay |
|---|---|---|
| P1. PE discovery, search and selection | `pe::pe_quarters_are_selectable_stable_courses_with_dated_sections`; native-PE; `installed-pe-search`; live-CLI | Swimming search returns Q1 and Q2. Real terminal input selects Q1 alongside A. Installed search returns both quarter IDs. Fresh f26 data has **2,314 offerings including 38 PE offerings**. |
| P2. Stable quarter/section identity, published metadata, IAP and schema validation | `pe::pe_quarters_are_selectable_stable_courses_with_dated_sections`; `pe::pe_iap_and_malformed_catalog_boundaries_are_checked` | Q1 options retain identity after another quarter is removed and source arrays are reordered. Titles/labels retain quarter and dates, Q5 is labeled IAP, and malformed quarter/date/section-number/slot inputs are rejected. |
| P3. Unknown or unsupported information stays visible, never invented | `pe::pe_unannounced_or_missing_dates_are_disclosed_not_invented`; `dates::invalid_or_incomplete_pe_dates_are_rejected_or_left_unresolved`; `dates::non_pe_date_limited_behavior_stays_omitted_and_unsupported` | Unknown trip times remain unknown. Missing/incomplete PE dates yield errors or unresolved choices rather than full-term expansion. Unsupported non-PE partial-term behavior remains explicitly omitted. |
| P4. Dated feasibility, academic conflicts, adjacency, holidays and substitutions | `dates::independent_cartesian_oracle_matches_mixed_academic_pe_calendar_solution`; `dates::app_uses_calendar_to_reject_academic_pe_conflicts_and_select_safe_option`; `dates::overlapping_pe_dates_conflict_but_exact_minute_adjacency_is_allowed`; `dates::holiday_and_alternate_weekday_boundaries_affect_pe_collisions` | Independent enumeration agrees with the selected feasible optimum. Academic/PE collisions reject the early option, overlapping active PE dates conflict, exact-minute adjacency passes, and holiday/alternate-weekday cases follow actual active dates. Debug and release pass. |
| P5. Disjoint quarters, date-sensitive grouping and honest union-week score | `dates::disjoint_pe_quarters_can_share_weekly_time_with_union_template_notice`; `dates::grouping_is_date_sensitive_only_for_bounded_pe`; the independent dated oracle | Separate quarters can share a clock time without a false conflict. Different bounds are not grouped as interchangeable members. The combined-week scoring notice is present and the oracle agrees with its score. No per-actual-week cost is claimed. |
| P6. PE export is bounded and timezone/calendar-aware | `pe::pe_catalog_optimizes_with_academics_and_exports_only_offering_dates`; `dates::export_bounded_pe_is_clipped_to_bounds_and_term_with_midnight_handling`; native-PE; `installed-pe-export`; live-CLI | Native PE fixture: **7 events, exactly 1 PE**. Installed two-quarter fixture: **9 events, exactly 3 PE** on October 26, November 3 and November 9, with exact UTC stamps. Live `PE.0201.Q1`: **7 events**, each inside **2026-09-22 through 2026-11-10**. Midnight and term clipping assertions pass. |
| P7. Focused PE editor, dates, spacing, validation and persistence | `pe::pe_search_and_manual_editor_prefill_preserve_published_dates`; `dates::pe_manual_roundtrip_overlay_supports_bounded_entries_with_no_unsupported_reason`; invalid-bound tests; native-PE | With A and PE selected, `a` on PE opens **PE.1000.Q1**, not A, with `pe` and **2026-10-26 / 2026-10-30** in the actual editor fields. The displayed ID/title has a separating space. Supported manual PE survives storage/overlay, while incomplete or internally conflicting meetings are rejected. |
| P8. Exactly Mon–Fri columns without loss of weekend meetings | `week::header_includes_weekday_columns_only`; `week::weekend_meetings_are_disclosed_but_do_not_set_weekday_bounds`; `week::only_weekend_input_shows_disclosure_without_time_rows`; native-grid; native-PE | Headers contain exactly the five requested weekdays. Weekend meetings do not lengthen the grid but are explicitly disclosed, including an all-weekend empty-grid case. Sunday exact details and all **17** replay events remain available/exported. |
| P9. Lecture/recitation/lab/PE and other component labels remain legible | `week::component_labels_cover_known_design_and_other_kinds`; `week::clipping_truncates_subject_before_component_at_inner_78`; `week::short_component_legend_is_complete_at_seventy_eight_columns`; native-PE | Actual cells read **`A Lec`, `A Rec`, `A Lab`, `PE.1000.Q1 PE`**. Design/other-kind assertions pass. Narrow cells shorten the subject before its component, and the complete short legend fits 78 inner columns. |
| P10. Shared buckets never imply incorrect subject/component pairings | `week::shared_bucket_keeps_different_course_component_pairs_when_they_fit`; `week::shared_bucket_summarizes_components_instead_of_mismatching_long_pairs`; `week::shared_bucket_with_same_course_different_components_shows_multiplicity`; `week::multiple_meetings_in_one_bucket_show_indicator_and_legend` | Fitting pairs remain associated with the correct subject, same-course multiple components retain multiplicity, and crowded different-course cells use a component summary instead of mismatched separately sorted lists. The multiplicity legend is present. |

## Shared preservation and delivery requirements

| ID | Concrete checks executed | Observed result |
|---|---|---|
| C1. Existing exact solver and CLI workflow | `optimizer_exact`: `exhaustive_oracle_covers_adjacency_atomic_bundles_exact_minutes_and_boundaries`, `day_priority_later_gap_filling_and_equal_day_subsets_are_exact`, `grouping_retains_members_but_splits_outbound_and_inbound_incompatibilities`, `infeasible_invalid_bounds_and_cancellation_never_claim_false_optimum`; `cli_workflow::actual_cli_selection_manual_entry_optimization_switch_and_export` | Existing oracles still agree, pairing-sensitive grouping remains safe, cancellation does not claim an optimum, and CLI manual/member/export workflow passes. |
| C2. Manual/cache/refresh and failure behavior | `state::editing_disabled_manual_entry_preserves_enabled_state`; `state::editing_manual_entry_cannot_move_course_or_kind_scope`; live-CLI; native-live | Disabled state and scope are retained. Live refresh preserves manual bytes, offline operation and fallback-with-age work, failed refresh preserves cache, and stale output cannot be exported. |
| C3. Output integrity and explicit failure | `app_contract::actual_member_resolution_rejects_incompatibilities_in_either_direction`; `app_contract::missing_calendar_and_invalid_member_never_create_output`; `cli_workflow::infeasible_export_preserves_status_and_never_touches_output`; `data_calendar::calendar_export_enumerates_actual_new_york_dates_holidays_alternates_dst_and_stable_ids`; `data_calendar::calendar_folds_utf8_escapes_text_handles_midnight_and_duplicate_events`; installed infeasible stages | Incompatible members and missing calendars cannot create output. Infeasible text/JSON exits **2**, preserves existing bytes, and creates no new file. Existing UTC/DST, stable UID, escaping/folding and midnight checks pass. |
| C4. Runnable release and safe installation | `format`, `clippy`, `release`, `install`, `installed-version`, `installed-export`, `installed-pe-search`, `installed-pe-export`, `fresh-fetch`, `opt-in`, `installed-live-export`, plus all other verifier gates | Locked release and strict lint pass. An isolated installed binary runs without Rust on PATH and exports **6 fixture / 9 mixed-PE / 113 live events**. No normal installation or calendar account is changed. |

## Reproduction

Use the baseline-build and `HYDRANT_UX_BASELINE_BIN` command in [VALIDATION.MD](VALIDATION.MD), with `RUST_TEST_NOCAPTURE=1 bash scripts/verify-mvp.sh --live`. Do not replace this with only the new PE or renderer tests. Require both debug and release `UX_COMPARISON` records and the named per-requirement passes above.

**Outcome:** the complete current result satisfies all mapped requirements for both goals. The feedback loop now has an observation for every row and an actual before/after comparison for the UX improvements. Limits remain macOS arm64 verification, known supported inputs, combined-week PE scoring, and parsed local ICS rather than calendar-account import.

## Session room rows: actual-terminal comparison (2026-09-09 13:37 UTC)

The room-row replay ran the pre-change executable built from `ab9e9a8` and the updated executable against the same offline CLI-created fixture in separate **120-column × 110-row native PTYs**. This is an observed screen comparison, not a source-inspection judgment. The test `actual_tui_week_replay_preserves_exact_times_and_records_observations` asserted and emitted:

| Requirement | Before | After |
|---|---|---|
| Preserve the first-row legend for Tuesday 09:35–10:05 | `W Lec` at 09:30 | `W Lec` at 09:30 |
| Show the room on the second occupied row | Blank at 10:00 | `Week room two` at 10:00 |
| Omit rooms for one-row sessions | No `Bucket room` in the timetable | No `Bucket room` in the timetable |
| Do not leak the room past the session end | Not the improvement target | 10:30 is `·`, and the selected room occurs exactly once in the entire timetable |
| Refresh when switching same-time sections | Not the improvement target | Enter, Enter, l changes the room to `Week room one`, with zero occurrences of the stale room |

Both executables restored terminal state. The updated workflow still exported 17 events with the selected room and exact minute/midnight boundaries. The 29 weekly-renderer tests also passed, including 165 start/duration cases, widths 0–120 with Unicode room labels, missing/multiline rooms, duplicate and overlapping rooms, later continuation rows, invalid/weekend meetings, and preserved selection style. Strict Clippy passed. The earlier full offline run passed 118 tests with four opt-in tests ignored.

Evidence is retained under `$JCODE_SCRATCH_DIR/hydrant-room-replay.TJ9Mwm/`: `comparison.log` contains the asserted `session_room_row` comparison and `session_room_switch` observations, `renderer-tests.log` contains the edge-case results, and `clippy.log` records lint completion. The directory also contains the archived baseline, build log, and current revision/worktree snapshot.

A second actual-terminal replay at 13:38 UTC also passed after concurrent changes hid half-hour time labels and replaced unoccupied `·` cells with blanks. `latest-comparison.log` records the same legend/room improvement, zero short-session or stale-room occurrences, and a blank unoccupied cell after the meeting. These unrelated presentation changes do not alter the two occupied session rows.

To reproduce, build an archive of `ab9e9a8` in an isolated scratch directory, then run:

```sh
HYDRANT_ROOM_BASELINE_BIN=/absolute/path/to/baseline/target/debug/hydrant-optimizer \
  cargo test --locked --test tui_acceptance \
  actual_tui_week_replay_preserves_exact_times_and_records_observations -- --nocapture
```

The normal acceptance assertions and observations always run. Setting `HYDRANT_ROOM_BASELINE_BIN` additionally requires the observed pre-change blank room row and the before/after comparison to pass.

### Whole-result rerun at 13:43 UTC

The established comparison was then enabled for **the entire combined project suite** using `HYDRANT_ROOM_BASELINE_BIN=... cargo test --locked --no-fail-fast -- --nocapture`, followed by `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`. The completed run passed **121 tests, zero failures, four opt-in skips**, formatting, and strict lint. Its state is recorded at `d50412b` plus the concurrent working-tree changes listed in `whole-result-completed-state.log`.

`whole-result-verified.log` reports the actual comparison: first row `W Lec` before and after, second row blank before and `Week room two` after, zero one-row room occurrences, one current-room occurrence, and an empty cell after the end. Switching changes the room to `Week room one` with zero stale-room occurrences. This run also passed the development watcher, every offline native TUI acceptance case, CLI/optimizer/storage/calendar/PE regressions, and all room edge cases. The same log directory contains `whole-result-format.log` and `whole-result-clippy.log`.

Earlier whole-result attempts exposed an in-progress watcher-test synchronization failure and an hourly-label Clippy warning. The watcher assertions were repaired by concurrent work, the modulo check was replaced with `is_multiple_of(60)`, and the entire feedback loop was rerun after those repairs. The successful result above is not inferred from separate partial runs.
