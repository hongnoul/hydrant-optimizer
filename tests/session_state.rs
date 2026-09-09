use std::{collections::BTreeSet, fs};

use hydrant_optimizer::{adapter, model::ManualStore, session::SelectionSession, tui::AppState};
use tempfile::TempDir;

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

fn state(dir: &TempDir, initial: &[&str], ephemeral: bool) -> AppState {
    AppState::new_with_session(
        adapter::parse_catalog(
            include_str!("fixtures/catalog.json"),
            include_str!("fixtures/term.json"),
        )
        .unwrap(),
        ManualStore::default(),
        dir.path().into(),
        dir.path().join("schedule.ics"),
        initial.iter().map(|s| (*s).to_owned()).collect(),
        ephemeral,
    )
    .unwrap()
}

#[test]
fn round_trip_empty_selection_terms_and_noop_save() {
    let dir = TempDir::new().unwrap();
    let mut session = SelectionSession::open(dir.path()).unwrap();
    assert!(session.selected("f26").is_none());
    session.save("f26", &ids(&["6.1200", "1.27"])).unwrap();
    let bytes = fs::read(dir.path().join("sessions.json")).unwrap();
    session.save("f26", &ids(&["1.27", "6.1200"])).unwrap();
    assert_eq!(bytes, fs::read(dir.path().join("sessions.json")).unwrap());
    session.save("s27", &ids(&["18.01"])).unwrap();
    session.save("f26", &BTreeSet::new()).unwrap();
    drop(session);
    let session = SelectionSession::open(dir.path()).unwrap();
    assert_eq!(session.selected("f26"), Some(&BTreeSet::new()));
    assert_eq!(session.selected("s27"), Some(&ids(&["18.01"])));
}

#[test]
fn second_writer_is_rejected_and_drop_releases_lock() {
    let dir = TempDir::new().unwrap();
    let session = SelectionSession::open(dir.path()).unwrap();
    assert!(SelectionSession::open(dir.path()).is_err());
    drop(session);
    assert!(SelectionSession::open(dir.path()).is_ok());
}

#[test]
fn invalid_or_future_state_is_preserved_and_ui_stays_usable() {
    for text in [
        "not json",
        r#"{"version":999,"future_data":"preserve me"}"#,
        r#"{"version":1,"terms":{},"unknown_field":true}"#,
        r#"{"version":1,"terms":{"f26":{"selected":["A"],"saved_at":"bad"}}}"#,
        r#"{"version":1,"terms":{"f26":{"selected":[""],"saved_at":"2026-09-09T00:00:00Z"}}}"#,
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sessions.json");
        fs::write(&path, text).unwrap();
        assert!(SelectionSession::open(dir.path()).is_err());
        let mut app = state(&dir, &[], false);
        assert!(
            app.selection_notice
                .as_ref()
                .unwrap()
                .contains("Selections NOT saved")
        );
        app.query = "A".into();
        app.recompute_filter();
        app.toggle_current_subject();
        assert!(!app.selected.is_empty());
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }
}

#[test]
fn external_edit_is_not_overwritten() {
    let dir = TempDir::new().unwrap();
    let mut session = SelectionSession::open(dir.path()).unwrap();
    session.save("f26", &ids(&["A"])).unwrap();
    let path = dir.path().join("sessions.json");
    fs::write(&path, "external edit").unwrap();
    let error = session.save("f26", &ids(&["B"])).unwrap_err();
    assert!(error.to_string().contains("changed outside"));
    // Returning to the old in-memory baseline must not falsely clear the warning.
    assert!(session.save("f26", &ids(&["A"])).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "external edit");
    assert_eq!(session.selected("f26"), Some(&ids(&["A"])));
}

#[test]
fn explicit_selection_replaces_saved_and_ephemeral_never_opens_storage() {
    let dir = TempDir::new().unwrap();
    drop(state(&dir, &["A"], false));
    drop(state(&dir, &["B"], false));
    let bytes = fs::read(dir.path().join("sessions.json")).unwrap();
    let mut app = state(&dir, &["A"], true);
    assert_eq!(app.selected, ids(&["A"]));
    app.toggle_current_subject();
    app.persist_selection();
    drop(app);
    assert_eq!(fs::read(dir.path().join("sessions.json")).unwrap(), bytes);
    assert_eq!(state(&dir, &[], false).selected, ids(&["B"]));

    let empty = TempDir::new().unwrap();
    drop(state(&empty, &[], true));
    assert_eq!(fs::read_dir(empty.path()).unwrap().count(), 0);
}

#[test]
fn unavailable_saved_ids_survive_edits_and_other_terms_do_not_leak() {
    let dir = TempDir::new().unwrap();
    let term = state(&dir, &[], true).dataset.term_id.clone();
    let mut session = SelectionSession::open(dir.path()).unwrap();
    session.save(&term, &ids(&["A", "GONE"])).unwrap();
    session.save("another-term", &ids(&["B"])).unwrap();
    drop(session);
    let mut app = state(&dir, &[], false);
    assert_eq!(app.selected, ids(&["A"]));
    assert!(app.selection_notice.as_ref().unwrap().contains("GONE"));
    app.query = "B".into();
    app.recompute_filter();
    app.toggle_current_subject();
    drop(app);
    let session = SelectionSession::open(dir.path()).unwrap();
    assert_eq!(session.selected(&term), Some(&ids(&["A", "B", "GONE"])));
    assert_eq!(session.selected("another-term"), Some(&ids(&["B"])));
}

#[test]
fn invalid_explicit_seed_does_not_destroy_previous_selection() {
    let dir = TempDir::new().unwrap();
    let app = state(&dir, &["A"], false);
    let base = app.base().clone();
    drop(app);
    let bytes = fs::read(dir.path().join("sessions.json")).unwrap();
    assert!(
        AppState::new_with_session(
            base,
            ManualStore::default(),
            dir.path().into(),
            dir.path().join("schedule.ics"),
            vec!["does-not-exist".into()],
            false,
        )
        .is_err()
    );
    assert_eq!(fs::read(dir.path().join("sessions.json")).unwrap(), bytes);
}

#[test]
fn selection_is_saved_before_solver_and_errors_survive_solver_install() {
    let dir = TempDir::new().unwrap();
    let mut app = state(&dir, &[], false);
    app.query = "A".into();
    app.recompute_filter();
    app.toggle_current_subject();
    assert!(app.solution.is_none());
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("sessions.json")).unwrap()).unwrap();
    assert_eq!(
        value["terms"][&app.dataset.term_id]["selected"],
        serde_json::json!(["A"])
    );

    fs::write(dir.path().join("sessions.json"), "external edit").unwrap();
    app.toggle_current_subject();
    let warning = app.selection_notice.clone();
    assert!(warning.as_ref().unwrap().contains("Selections NOT saved"));
    let solution = hydrant_optimizer::app::optimize(&app.dataset, &[], None).unwrap();
    app.install_solution(solution);
    assert_eq!(warning, app.selection_notice);
}

#[cfg(unix)]
#[test]
fn write_failure_keeps_old_state_and_can_be_retried() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let mut session = SelectionSession::open(dir.path()).unwrap();
    session.save("f26", &ids(&["A"])).unwrap();
    let bytes = fs::read(dir.path().join("sessions.json")).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let result = session.save("f26", &ids(&["B"]));
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    // Root can bypass permissions, so only assert rollback when the OS denied it.
    if result.is_err() {
        assert_eq!(session.selected("f26"), Some(&ids(&["A"])));
        assert_eq!(fs::read(dir.path().join("sessions.json")).unwrap(), bytes);
    }
    session.save("f26", &ids(&["B"])).unwrap();
    assert_eq!(session.selected("f26"), Some(&ids(&["B"])));
}
