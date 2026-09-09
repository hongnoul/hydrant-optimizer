# Architecture

## Rust implementation

One Cargo package provides a reusable library and the `hydrant-optimizer` executable. No Python process, FFI boundary, database server, web service, or general-purpose optimization framework is needed.

```text
Hydrant JSON -> adapter + separate local manual overlays -> normalized Dataset
CLI/TUI subject selection -> equivalent-time grouping -> pure exact optimizer
                         -> timetable + actual-section picker -> local ICS
```

| Module | Responsibility |
|---|---|
| `model` | Owned, serializable domain records, explicit statuses, interval validation |
| `adapter` | Hydrant raw JSON and calendar metadata -> normalized dataset |
| `storage` | Public HTTP fetching, validated cache, independent manual records, merge |
| `session` | Versioned, term-scoped selected subject IDs, atomic persistence, single-writer lock |
| `optimizer` | Canonical grouping, minute-bitset search, deterministic exact result |
| `calendar` | Dated event expansion, holidays/alternate weekdays, New York timezone, ICS |
| `app` | Shared CLI/TUI orchestration, actual-member resolution, safe export |
| `tui` | Keyboard state machine, selection/editor/result views, solver worker |
| `tui::week` | Monday–Friday, 30-minute grid with component labels and weekend disclosure |
| `main` | CLI options, local-file input, text/JSON output, application startup |

Dependencies: `serde`/`serde_json` for data, `reqwest::blocking` for HTTPS, Ratatui/Crossterm for the terminal, `chrono`/`chrono-tz` for dates, `clap` for CLI parsing, `sha2` for content-derived IDs, `fs2` for crash-released session locks, and `tempfile` for safe file replacement/output. The calendar module emits RFC 5545 text, checked with the independent `icalendar` parser in tests. `Cargo.lock` pins the executable's dependency graph.

## Sources

- `https://hydrant.mit.edu/latest.json`: `classes` keyed by subject number, `termInfo`, `lastUpdated`, `locations`, and `pe` grouped by quarter.
- `https://hydrant.mit.edu/latestTerm.json`: current/pre-semester calendar metadata. Match the downloaded catalog's term rather than assuming the current semester.
- Reference: <https://github.com/sipb/hydrant>. Upstream implementation is GPL-licensed. Consume the public data without copying upstream implementation.

The upstream `src/lib/raw.ts` and `scrapers/utils.py` define lowercase `sectionKinds`, `{kind}Sections`, and sections `[meetings, room]`. Each raw meeting is `[start_slot, length]`:

```text
weekday = start_slot / 34                  # integer division, Monday = 0
start_minute = 360 + 30 * (start_slot % 34)
end_minute = start_minute + 30 * length
```

Published encoding uses 34 half-hour slots per weekday beginning at 06:00. The domain model is not restricted to that resolution. Manual entries preserve exact minutes and can use weekends. Academic `half` and `quarterInfo` identify date-limited cases that remain visible but unsupported. PE `sections` use the same encoding and have explicit `startDate`/`endDate`. Normalize each PE offering to a quarter-qualified course ID with one `pe` requirement. Keep official section numbers in labels and stable option identity. Unknown times and incomplete/out-of-term dates are disclosed.

## Shared model

```text
Dataset: term_id, last_updated, optional calendar, courses, notices
TermCalendar: start, end, holidays, alternate_days[date -> weekday]
Course: id, title, requirements, notices
Requirement: id, kind, options, has_unknown_times
SectionOption: id, label, room, source(api|manual), meetings,
               incompatible_with[option IDs], optional unsupported_reason
Meeting: weekday(0=Mon..6=Sun), start_minute, end_minute,
         optional start_date/end_date
TimeChoice: id, requirement_id, canonical meetings, members[SectionOption]
Score: occupied_days, gap_minutes
Solution: status(optimal_known|infeasible|cancelled), choices, optional score,
          unresolved notices
ManualEntry: term_id, course_id, kind, option, enabled
ManualStore: version, entries
ChosenSection: course_id, course_title, kind, actual SectionOption
```

Use owned records and stable IDs across UI state, persisted edits, solver results, and export. Do not hold references into mutable UI collections. A requirement is one subject/component, e.g. `6.1200/recitation`. Select an option's complete meeting bundle. Retain available rooms/labels, but do not invent official section numbers. Canonical ordering is explicit rather than dependent on hash-map iteration.

## Data and manual edits

- Fetch and parse failures are errors, not empty catalogs. Validate before replacing the last good cache. Cache raw catalog and matching term metadata together, atomically, with a fetch timestamp.
- Offline mode reads only the cache. Online failure can use the last good cache with a visible explanation and data age. A missing/bad cache never masquerades as a valid schedule.
- Manual records live separately, keyed by term/subject/component/option. Refresh never overwrites edits. Load and validate the store before applying enabled entries to a cloned dataset.
- Manual entry supports alternative sections with multiple meetings, optional room and dates, and exact times. Reject malformed time order/ranges. Preserve known components when filling TBA information.
- Retain manual records when published options arrive. Show their presence and let users enable, disable, or edit them. Identical interchangeable meeting patterns are grouped, avoiding duplicate search work while preserving actual members.
- Different terms never share manual entries. Disabled records stay available for later review. Invalid persisted data yields a descriptive error rather than silent loss.

## Search and UI

The optimizer is a pure in-memory function. It receives selected merged requirements, an optional cancellation flag, and, through `solve_with_calendar`, the loaded term calendar for dated PE conflicts. See [OPTIMIZER.MD](OPTIMIZER.MD). All input sources follow this path.

The TUI owns selection IDs, search text, editor state, chosen result members, and notices. Selection survives filtering. Any selection/manual edit invalidates the old result. Search runs on an ordinary worker thread, communicating its result to the event loop. Escape can cancel, and cancellation never claims an optimum or infeasibility. There is no need for application-wide async. Initial HTTP loading happens before entering terminal raw mode.

The executable uses `AppState::new_with_session` to restore selections and save each change before launching the optimizer or rendering the new state. `AppState::new` remains side-effect free for library callers. Only subject IDs and save timestamps are persisted in `sessions.json`, not solver results, member choices, or editor drafts. An advisory lock prevents cooperating TUIs from overwriting one another, and disk-byte checks detect external edits. Corrupt/newer-version files remain untouched and persistence errors stay visible independently of solver status. `--no-restore` disables both selection reads and writes. Restored unavailable IDs are retained on disk but excluded from optimization.

Terminal setup has restoration guards for normal exit, errors, and panics. The interface provides catalog selection, manual entry management, score/timetable, unresolved notices, actual same-time member switching, explicit infeasibility, and local export.

The top-level search input replaces the former status panel. Subjects and Selected classes occupy equal halves of the upper row. The selected-class list reads the selected IDs independently of the search filter, supports removal with selection invalidation, and supplies the highlighted class to the manual editor. Retained manual entries are available through the `m` overlay rather than a permanent pane. A separate full-width Timetable pane, implemented in `src/tui/timetable.rs`, shows five Monday–Friday columns at 80 columns. Cells retain lecture/recitation/lab/PE component labels when subject numbers must be shortened. Weekend meetings are disclosed and retained in details/export rather than assigned hidden grid columns. The weekly timetable uses one row per 30-minute interval, including all intersecting exact-minute meetings. This is only display bucketing, never solver or export rounding. Left/Right and h/l cycle panes, while Up/Down and k/j navigate within the active pane. Search/editor input consumes text before navigation shortcuts.

The timetable has its own viewport with PgUp/PgDn and Home/End scrolling. Its grid renders edge to edge without a parent Block or legend. A single fixed heading carries the focus marker and scroll position. The `t` shortcut resets the viewport. Result invalidation or installing a new solution also resets it. Tab/Shift+Tab visits Subjects, Selected classes, and Timetable. The Results pane is removed. Action feedback stays in the footer, and detailed optimization output remains available through the CLI. Escape closes the manual overlay without quitting.

## Calendar and output safety

Expand each selected actual section into dated occurrences within reliable term/meeting bounds. Fully bounded PE is supported, while other partial-term sections remain explicitly omitted. The weekly score/display combines all selected PE quarters, but conflict checks and event expansion respect their actual date windows. Apply holidays and alternate-weekday substitutions before timezone conversion. Interpret wall-clock times in `America/New_York`, then serialize each occurrence in UTC. This preserves 09:00 classes across DST rather than repeating a fixed UTC hour.

Reject or disclose ambiguous/nonexistent local times, missing calendar bounds, and unsupported sections. Export known supported meetings and surface omissions. Include stable event UIDs and escaped labels/rooms. Do not claim that successful serialization alone proves calendar-client interoperability.

Write output atomically with a Unix-time filename suffix, so every export is a new file. Export does not mutate a user's calendar account. Release artifacts are native executables per target; only platforms actually built and exercised are claimed as verified.

CLI `--export` invokes calendar writing only for a completed feasible result. Infeasible optimization retains its explicit text/JSON, unresolved notices, and exit code 2 without touching the output path.

The timetable uses a fixed Monday–Friday sheet with 48 half-hour rows spanning 00:00–24:00, including empty schedules. Session navigation scrolls to the selected block within that fixed sheet. PgUp/PgDn and Home/End browse the full day without changing the schedule.
