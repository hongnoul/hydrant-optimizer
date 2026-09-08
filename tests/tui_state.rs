use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hydrant_optimizer::{
    model::*,
    tui::{AppAction, AppState, EditorField, Focus},
};
use tempfile::TempDir;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn meeting(day: u8, start: u16, end: u16) -> Meeting {
    Meeting {
        weekday: day,
        start_minute: start,
        end_minute: end,
        start_date: None,
        end_date: None,
    }
}

fn section(id: &str, label: &str, meetings: Vec<Meeting>) -> SectionOption {
    SectionOption {
        id: id.to_string(),
        label: label.to_string(),
        room: format!("Room {label}"),
        source: Source::Api,
        meetings,
        incompatible_with: BTreeSet::new(),
        unsupported_reason: None,
    }
}

fn dataset() -> Dataset {
    let mut courses = BTreeMap::new();
    courses.insert(
        "A".to_string(),
        Course {
            id: "A".to_string(),
            title: "Algorithms".to_string(),
            requirements: vec![Requirement {
                id: "A/lecture".to_string(),
                kind: "lecture".to_string(),
                options: vec![section("A-L1", "L1", vec![meeting(0, 9 * 60, 10 * 60)])],
                has_unknown_times: false,
            }],
            notices: Vec::new(),
        },
    );
    courses.insert(
        "B".to_string(),
        Course {
            id: "B".to_string(),
            title: "Molecular Biology".to_string(),
            requirements: vec![Requirement {
                id: "B/lecture".to_string(),
                kind: "lecture".to_string(),
                options: vec![section("B-L1", "L1", vec![meeting(1, 11 * 60, 12 * 60)])],
                has_unknown_times: false,
            }],
            notices: Vec::new(),
        },
    );
    Dataset {
        term_id: "test-term".to_string(),
        last_updated: "now".to_string(),
        calendar: Some(TermCalendar {
            start: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 12, 15).unwrap(),
            holidays: BTreeSet::new(),
            alternate_days: BTreeMap::new(),
        }),
        courses,
        notices: Vec::new(),
    }
}

fn state(temp: &TempDir, initial: Vec<String>) -> AppState {
    AppState::new(
        dataset(),
        ManualStore::default(),
        temp.path().to_path_buf(),
        temp.path().join("schedule.ics"),
        initial,
    )
    .unwrap()
}

#[test]
fn subject_search_selection_persists_across_filters() {
    let temp = TempDir::new().unwrap();
    let mut state = state(&temp, vec!["A".to_string()]);
    assert!(state.selected.contains("A"));

    state.handle_key(key(KeyCode::Char('/'))).unwrap();
    assert_eq!(state.focus, Focus::Search);
    state.handle_key(key(KeyCode::Char('b'))).unwrap();
    state.handle_key(key(KeyCode::Char('i'))).unwrap();
    assert_eq!(state.filtered, vec!["B".to_string()]);
    assert!(state.selected.contains("A"));

    state.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(state.focus, Focus::Subjects);
    state.handle_key(key(KeyCode::Char(' '))).unwrap();
    assert!(state.selected.contains("A"));
    assert!(state.selected.contains("B"));

    state.query.clear();
    state.recompute_filter();
    assert_eq!(state.filtered, vec!["A".to_string(), "B".to_string()]);
    assert!(state.selected.contains("A"));
    assert!(state.selected.contains("B"));
}

#[test]
fn search_accepts_spaces_for_multiword_titles_and_ctrl_u_clears() {
    let temp = TempDir::new().unwrap();
    let mut state = state(&temp, Vec::new());

    state.handle_key(key(KeyCode::Char('/'))).unwrap();
    for c in "molecular".chars() {
        state.handle_key(key(KeyCode::Char(c))).unwrap();
    }
    state.handle_key(key(KeyCode::Char(' '))).unwrap();
    for c in "biology".chars() {
        state.handle_key(key(KeyCode::Char(c))).unwrap();
    }

    assert_eq!(state.query, "molecular biology");
    assert_eq!(state.filtered, vec!["B".to_string()]);
    assert!(state.selected.is_empty());

    state.handle_key(ctrl('u')).unwrap();
    assert!(state.query.is_empty());
    assert_eq!(state.filtered, vec!["A".to_string(), "B".to_string()]);
}

fn add_manual_entry(state: &mut AppState, course_id: &str, kind: &str) {
    state.open_add_manual();
    let form = state.editor.as_mut().unwrap();
    form.course_id = course_id.to_string();
    form.kind = kind.to_string();
    form.label = "Study group".to_string();
    form.meetings = "Mon 09:00-10:00".to_string();
    form.room = "Room Z".to_string();

    state.save_current_manual().unwrap();
}

#[test]
fn manual_editor_saves_atomic_meeting_bundle_with_dates_and_toggle_invalidates() {
    let temp = TempDir::new().unwrap();
    let mut state = state(&temp, Vec::new());
    state.open_add_manual();
    let form = state.editor.as_mut().unwrap();
    form.course_id = "A".to_string();
    form.kind = "recitation".to_string();
    form.label = "Study group".to_string();
    form.meetings = "Mon 09:00-10:00;Wed 09:00-10:00".to_string();
    form.room = "Room Z".to_string();
    form.start_date = "2026-09-02".to_string();
    form.end_date = "2026-12-09".to_string();

    state.save_current_manual().unwrap();
    assert!(state.editor.is_none());
    assert_eq!(state.manual.entries.len(), 1);
    let entry = &state.manual.entries[0];
    assert_eq!(entry.course_id, "A");
    assert_eq!(entry.kind, "recitation");
    assert_eq!(entry.option.meetings.len(), 2);
    assert_eq!(entry.option.room, "Room Z");
    assert_eq!(
        entry.option.meetings[0].start_date.unwrap().to_string(),
        "2026-09-02"
    );
    assert_eq!(
        entry.option.meetings[1].end_date.unwrap().to_string(),
        "2026-12-09"
    );
    assert!(state.solution.is_none());

    let before = state.generation;
    state.focus = Focus::Manual;
    state.install_solution(Solution {
        status: SolveStatus::OptimalKnown,
        choices: Vec::new(),
        score: Some(Score {
            occupied_days: 0,
            gap_minutes: 0,
        }),
        unresolved: Vec::new(),
    });
    state.toggle_current_manual_enabled().unwrap();
    assert!(!state.manual.entries[0].enabled);
    assert!(state.solution.is_none());
    assert!(state.generation > before);
}

#[test]
fn editing_disabled_manual_entry_preserves_enabled_state() {
    let temp = TempDir::new().unwrap();
    let mut state = state(&temp, Vec::new());
    add_manual_entry(&mut state, "A", "recitation");

    state.focus = Focus::Manual;
    state.toggle_current_manual_enabled().unwrap();
    assert!(!state.manual.entries[0].enabled);

    state.open_edit_current_manual();
    let form = state.editor.as_mut().unwrap();
    form.enabled = true;
    form.label = "Renamed study group".to_string();

    state.save_current_manual().unwrap();

    assert_eq!(state.manual.entries.len(), 1);
    assert_eq!(state.manual.entries[0].option.label, "Renamed study group");
    assert!(!state.manual.entries[0].enabled);
}

#[test]
fn editing_manual_entry_cannot_move_course_or_kind_scope() {
    let temp = TempDir::new().unwrap();
    let mut state = state(&temp, Vec::new());
    add_manual_entry(&mut state, "A", "recitation");

    state.focus = Focus::Manual;
    state.open_edit_current_manual();
    state.editor.as_mut().unwrap().course_id = "B".to_string();
    let err = state.save_current_manual().unwrap_err().to_string();
    assert!(err.contains("cannot move"), "unexpected error: {err}");
    assert_eq!(state.manual.entries[0].course_id, "A");
    assert_eq!(state.manual.entries[0].kind, "recitation");

    state.editor.as_mut().unwrap().course_id = "A".to_string();
    state.editor.as_mut().unwrap().kind = "lab".to_string();
    let err = state.save_current_manual().unwrap_err().to_string();
    assert!(err.contains("cannot move"), "unexpected error: {err}");
    assert_eq!(state.manual.entries.len(), 1);
    assert_eq!(state.manual.entries[0].kind, "recitation");
}

#[test]
fn same_time_member_switching_updates_mapping_and_selection_invalidates_result() {
    let temp = TempDir::new().unwrap();
    let mut state = state(&temp, Vec::new());
    let meetings = vec![meeting(0, 9 * 60, 10 * 60)];
    state.install_solution(Solution {
        status: SolveStatus::OptimalKnown,
        choices: vec![TimeChoice {
            id: "A-L1".to_string(),
            requirement_id: "A/lecture".to_string(),
            meetings: meetings.clone(),
            members: vec![
                section("A-L1", "L1", meetings.clone()),
                section("A-L2", "L2", meetings),
            ],
        }],
        score: Some(Score {
            occupied_days: 1,
            gap_minutes: 0,
        }),
        unresolved: vec!["notice".to_string()],
    });

    state.focus = Focus::Results;
    state.handle_key(key(KeyCode::Right)).unwrap();
    assert_eq!(
        state.actual_members.get("A/lecture").map(String::as_str),
        Some("A-L2")
    );
    state.handle_key(key(KeyCode::Left)).unwrap();
    assert_eq!(
        state.actual_members.get("A/lecture").map(String::as_str),
        Some("A-L1")
    );

    state.focus = Focus::Subjects;
    state.handle_key(key(KeyCode::Enter)).unwrap();
    assert!(state.solution.is_none());
    assert!(state.actual_members.is_empty());
}

#[test]
fn editor_key_handling_tabs_fields_accepts_text_and_requests_save() {
    let temp = TempDir::new().unwrap();
    let mut state = state(&temp, Vec::new());
    state.open_add_manual();
    assert_eq!(state.editor.as_ref().unwrap().field, EditorField::Course);

    state.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(state.editor.as_ref().unwrap().field, EditorField::Kind);
    state.editor.as_mut().unwrap().kind.clear();
    state.handle_key(key(KeyCode::Char('l'))).unwrap();
    state.handle_key(key(KeyCode::Char('a'))).unwrap();
    state.handle_key(key(KeyCode::Char('b'))).unwrap();
    assert_eq!(state.editor.as_ref().unwrap().kind, "lab");

    assert_eq!(state.handle_key(ctrl('s')).unwrap(), AppAction::SaveManual);
}
