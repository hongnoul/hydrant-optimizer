use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use chrono::NaiveDate;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{app, model::*, session::SelectionSession, storage};

mod timetable;
mod week;

pub const KEYMAP: &str = "q/Esc quit, / search, Ctrl+U clear search, j/k or Down/Up move/scroll within pane, h/l or Left/Right previous/next pane, Space select/remove class, Tab next panel/field, click pane to focus, s selected classes, m manual overlay, a add manual, Enter select/remove/edit, x enable-disable manual, c cancel, t timetable (h/l alternatives, Enter blocks, hjkl move, Enter then h/l same-time member, Esc back), PgUp/PgDn scroll active pane, Home/End first/last line, e export, ? help, Ctrl+S save editor";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Subjects,
    Search,
    Selected,
    Manual,
    Timetable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorField {
    Course,
    Kind,
    Label,
    Meetings,
    Room,
    StartDate,
    EndDate,
}

impl EditorField {
    fn all() -> &'static [EditorField] {
        &[
            EditorField::Course,
            EditorField::Kind,
            EditorField::Label,
            EditorField::Meetings,
            EditorField::Room,
            EditorField::StartDate,
            EditorField::EndDate,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            EditorField::Course => "Subject",
            EditorField::Kind => "Component",
            EditorField::Label => "Section label",
            EditorField::Meetings => "Meetings",
            EditorField::Room => "Room",
            EditorField::StartDate => "Start date",
            EditorField::EndDate => "End date",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualForm {
    pub entry_id: Option<String>,
    pub enabled: bool,
    pub course_id: String,
    pub kind: String,
    pub label: String,
    pub meetings: String,
    pub room: String,
    pub start_date: String,
    pub end_date: String,
    pub field: EditorField,
    pub error: Option<String>,
}

impl ManualForm {
    pub fn new(course_id: String) -> Self {
        Self {
            entry_id: None,
            enabled: true,
            course_id,
            kind: "lecture".to_string(),
            label: "Manual section".to_string(),
            meetings: String::new(),
            room: String::new(),
            start_date: String::new(),
            end_date: String::new(),
            field: EditorField::Course,
            error: None,
        }
    }

    fn from_entry(entry: &ManualEntry) -> Self {
        let start_date = common_date(&entry.option.meetings, true).unwrap_or_default();
        let end_date = common_date(&entry.option.meetings, false).unwrap_or_default();
        Self {
            entry_id: Some(entry.option.id.clone()),
            enabled: entry.enabled,
            course_id: entry.course_id.clone(),
            kind: entry.kind.clone(),
            label: entry.option.label.clone(),
            meetings: entry
                .option
                .meetings
                .iter()
                .map(Meeting::display)
                .collect::<Vec<_>>()
                .join(";"),
            room: entry.option.room.clone(),
            start_date,
            end_date,
            field: EditorField::Course,
            error: None,
        }
    }

    fn field_value_mut(&mut self) -> &mut String {
        match self.field {
            EditorField::Course => &mut self.course_id,
            EditorField::Kind => &mut self.kind,
            EditorField::Label => &mut self.label,
            EditorField::Meetings => &mut self.meetings,
            EditorField::Room => &mut self.room,
            EditorField::StartDate => &mut self.start_date,
            EditorField::EndDate => &mut self.end_date,
        }
    }

    fn next_field(&mut self, reverse: bool) {
        let fields = EditorField::all();
        let index = fields
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        let next = if reverse {
            if index == 0 {
                fields.len() - 1
            } else {
                index - 1
            }
        } else {
            (index + 1) % fields.len()
        };
        self.field = fields[next];
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub event_count: usize,
    pub notices: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
    CancelOptimize,
    Export,
    SaveManual,
    ToggleManualEnabled,
}

#[derive(Debug, Default)]
struct ScrollViewport {
    offset: u16,
    max_offset: u16,
    height: u16,
}

impl ScrollViewport {
    fn page(&mut self, direction: isize) {
        let step = self.height.saturating_sub(1).max(1) as isize;
        self.offset = (self.offset as usize)
            .saturating_add_signed(direction * step)
            .min(self.max_offset as usize) as u16;
    }
}

#[derive(Debug)]
pub struct AppState {
    base: Dataset,
    selection_session: Option<SelectionSession>,
    selection_saved: bool,
    unavailable_selected: BTreeSet<String>,
    pub selection_notice: Option<String>,
    pub dataset: Dataset,
    pub manual: ManualStore,
    pub data_dir: PathBuf,
    pub output: PathBuf,
    pub query: String,
    pub filtered: Vec<String>,
    pub cursor: usize,
    pub selected: BTreeSet<String>,
    pub focus: Focus,
    focus_regions: Vec<(Rect, Focus)>,
    pub selected_cursor: usize,
    pub manual_cursor: usize,
    pub result_cursor: usize,
    timetable_viewport: ScrollViewport,
    timetable_navigation: timetable::Navigation,
    pub editor: Option<ManualForm>,
    pub solution: Option<Solution>,
    pub actual_members: BTreeMap<String, String>,
    pub optimize_running: bool,
    pub status: String,
    pub export: Option<ExportSummary>,
    pub should_quit: bool,
    pub show_help: bool,
    pub generation: u64,
}

impl AppState {
    pub fn new(
        base: Dataset,
        manual: ManualStore,
        data_dir: PathBuf,
        output: PathBuf,
        initial: Vec<String>,
    ) -> Result<Self> {
        let dataset = storage::apply_manual(&base, &manual)?;
        let mut selected = BTreeSet::new();
        let mut unknown = Vec::new();
        for requested in initial {
            if base.courses.contains_key(&requested) {
                selected.insert(requested);
            } else if let Some(id) = base
                .courses
                .keys()
                .find(|id| id.eq_ignore_ascii_case(&requested))
            {
                selected.insert(id.clone());
            } else {
                unknown.push(requested);
            }
        }
        let mut state = Self {
            base,
            selection_session: None,
            selection_saved: false,
            unavailable_selected: BTreeSet::new(),
            selection_notice: None,
            dataset,
            manual,
            data_dir,
            output,
            query: String::new(),
            filtered: Vec::new(),
            cursor: 0,
            selected,
            focus: Focus::Subjects,
            focus_regions: Vec::new(),
            selected_cursor: 0,
            manual_cursor: 0,
            result_cursor: 0,
            timetable_viewport: ScrollViewport::default(),
            timetable_navigation: timetable::Navigation::default(),
            editor: None,
            solution: None,
            actual_members: BTreeMap::new(),
            optimize_running: false,
            status: String::from("Ready. Press ? for help."),
            export: None,
            should_quit: false,
            show_help: false,
            generation: 0,
        };
        state.recompute_filter();
        if !unknown.is_empty() {
            state.status = format!("Unknown initial subject(s): {}", unknown.join(", "));
        } else if !state.selected.is_empty() {
            state.status = format!(
                "Preselected {} subject(s). Optimizing automatically.",
                state.selected.len()
            );
        }
        Ok(state)
    }

    /// The ordinary constructor remains side-effect free for library callers.
    /// The executable opts into durable selections through this constructor.
    pub fn new_with_session(
        base: Dataset,
        manual: ManualStore,
        data_dir: PathBuf,
        output: PathBuf,
        initial: Vec<String>,
        no_restore: bool,
    ) -> Result<Self> {
        // An explicit typo must not replace a valid saved selection with empty state.
        for id in &initial {
            ensure!(
                base.courses
                    .keys()
                    .any(|known| known.eq_ignore_ascii_case(id)),
                "unknown initial subject {id}; saved selections were not changed"
            );
        }
        if no_restore {
            let mut state = Self::new(base, manual, data_dir, output, initial)?;
            state.selection_notice = Some("Ephemeral selections (--no-restore): not saved".into());
            return Ok(state);
        }
        let (session, error) = match SelectionSession::open(&data_dir) {
            Ok(session) => (Some(session), None),
            Err(error) => (None, Some(format!("Selections NOT saved: {error:#}"))),
        };
        let restoring = initial.is_empty();
        let requested = if restoring {
            session
                .as_ref()
                .and_then(|s| s.selected(&base.term_id))
                .map(|ids| ids.iter().cloned().collect())
                .unwrap_or_default()
        } else {
            initial
        };
        let unavailable = requested
            .iter()
            .filter(|id| {
                !base
                    .courses
                    .keys()
                    .any(|known| known.eq_ignore_ascii_case(id))
            })
            .cloned()
            .collect();
        let mut state = Self::new(base, manual, data_dir, output, requested)?;
        state.selection_session = session;
        state.unavailable_selected = unavailable;
        state.selection_notice = error;
        if restoring && !state.selected.is_empty() {
            state.status = format!(
                "Restored {} subject(s). Optimizing automatically.",
                state.selected.len()
            );
        }
        state.persist_selection();
        Ok(state)
    }

    /// Save synchronously before a changed selection is displayed or optimized.
    /// A failed save leaves the UI usable, but the warning survives solver updates.
    pub fn persist_selection(&mut self) {
        let Some(session) = self.selection_session.as_mut() else {
            return;
        };
        let selected = self
            .selected
            .union(&self.unavailable_selected)
            .cloned()
            .collect();
        let result = session.save(&self.dataset.term_id, &selected);
        self.selection_saved = result.is_ok();
        self.selection_notice = match result {
            Err(error) => Some(format!("Selections NOT saved: {error:#}")),
            Ok(()) if !self.unavailable_selected.is_empty() => Some(format!(
                "Unavailable saved subjects retained: {}",
                self.unavailable_selected
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Ok(()) => None,
        };
    }

    pub fn base(&self) -> &Dataset {
        &self.base
    }

    pub fn selected_subjects(&self) -> Vec<String> {
        self.selected.iter().cloned().collect()
    }

    pub fn current_subject(&self) -> Option<&str> {
        self.filtered.get(self.cursor).map(String::as_str)
    }

    pub fn recompute_filter(&mut self) {
        let terms = self
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.filtered = self
            .base
            .courses
            .iter()
            .filter(|(id, course)| {
                if terms.is_empty() {
                    return true;
                }
                let haystack = format!("{} {}", id.to_lowercase(), course.title.to_lowercase());
                terms.iter().all(|term| haystack.contains(term))
            })
            .map(|(id, _)| id.clone())
            .collect();
        self.clamp_cursors();
    }

    /// Focus only: clicking never selects/removes a class or changes a schedule.
    pub fn handle_mouse(&mut self, event: MouseEvent) {
        if event.kind != MouseEventKind::Down(MouseButton::Left)
            || self.show_help
            || self.editor.is_some()
            || self.focus == Focus::Manual
        {
            return;
        }
        let position = ratatui::layout::Position::new(event.column, event.row);
        if let Some((_, focus)) = self
            .focus_regions
            .iter()
            .find(|(area, _)| area.contains(position))
        {
            self.focus = *focus;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<AppAction> {
        if self.show_help {
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Enter => {
                    self.show_help = false;
                    return Ok(AppAction::None);
                }
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return Ok(AppAction::Quit);
                }
                _ => return Ok(AppAction::None),
            }
        }

        if self.editor.is_some() {
            return self.handle_editor_key(key);
        }

        if self.focus == Focus::Search {
            return self.handle_search_key(key);
        }

        if self.focus == Focus::Timetable && self.handle_timetable_key(key.code) {
            return Ok(AppAction::None);
        }

        match key.code {
            KeyCode::Esc if self.focus == Focus::Manual => {
                self.focus = Focus::Selected;
                Ok(AppAction::None)
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
                Ok(AppAction::Quit)
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                Ok(AppAction::None)
            }
            KeyCode::Char('/') => {
                self.focus = Focus::Search;
                Ok(AppAction::None)
            }
            KeyCode::Tab => {
                self.next_focus(false);
                Ok(AppAction::None)
            }
            KeyCode::BackTab => {
                self.next_focus(true);
                Ok(AppAction::None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_cursor(-1);
                Ok(AppAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_cursor(1);
                Ok(AppAction::None)
            }
            KeyCode::PageUp => {
                if self.focus == Focus::Timetable {
                    self.timetable_viewport.page(-1);
                } else {
                    self.move_cursor(-10);
                }
                Ok(AppAction::None)
            }
            KeyCode::PageDown => {
                if self.focus == Focus::Timetable {
                    self.timetable_viewport.page(1);
                } else {
                    self.move_cursor(10);
                }
                Ok(AppAction::None)
            }
            KeyCode::Home | KeyCode::End if self.focus == Focus::Timetable => {
                let viewport = &mut self.timetable_viewport;
                viewport.offset = if key.code == KeyCode::Home {
                    0
                } else {
                    viewport.max_offset
                };
                Ok(AppAction::None)
            }
            KeyCode::Char(' ') => {
                if self.focus == Focus::Subjects {
                    self.toggle_current_subject();
                } else if self.focus == Focus::Selected {
                    self.remove_current_selected();
                }
                Ok(AppAction::None)
            }
            KeyCode::Enter => match self.focus {
                Focus::Subjects => {
                    self.toggle_current_subject();
                    Ok(AppAction::None)
                }
                Focus::Selected => {
                    self.remove_current_selected();
                    Ok(AppAction::None)
                }
                Focus::Manual => {
                    self.open_edit_current_manual();
                    Ok(AppAction::None)
                }
                Focus::Search | Focus::Timetable => Ok(AppAction::None),
            },
            KeyCode::Char('c') => Ok(AppAction::CancelOptimize),
            KeyCode::Char('e') => Ok(AppAction::Export),
            KeyCode::Char('s') => {
                self.focus = Focus::Selected;
                Ok(AppAction::None)
            }
            KeyCode::Char('t') => {
                self.focus = Focus::Timetable;
                self.timetable_viewport.offset = 16.min(self.timetable_viewport.max_offset);
                Ok(AppAction::None)
            }
            KeyCode::Char('m') => {
                self.focus = if self.focus == Focus::Manual {
                    Focus::Selected
                } else {
                    Focus::Manual
                };
                Ok(AppAction::None)
            }
            KeyCode::Char('a') => {
                self.open_add_manual();
                Ok(AppAction::None)
            }
            KeyCode::Char('x') => {
                if self.focus == Focus::Manual {
                    Ok(AppAction::ToggleManualEnabled)
                } else {
                    Ok(AppAction::None)
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.next_focus(true);
                Ok(AppAction::None)
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.next_focus(false);
                Ok(AppAction::None)
            }
            _ => Ok(AppAction::None),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> Result<AppAction> {
        match key.code {
            KeyCode::Esc => {
                self.focus = Focus::Subjects;
                Ok(AppAction::None)
            }
            KeyCode::Enter => {
                self.focus = Focus::Subjects;
                Ok(AppAction::None)
            }
            KeyCode::Tab => {
                self.next_focus(false);
                Ok(AppAction::None)
            }
            KeyCode::BackTab => {
                self.next_focus(true);
                Ok(AppAction::None)
            }
            KeyCode::Up => {
                self.focus = Focus::Subjects;
                self.move_cursor(-1);
                Ok(AppAction::None)
            }
            KeyCode::Down => {
                self.focus = Focus::Subjects;
                self.move_cursor(1);
                Ok(AppAction::None)
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.recompute_filter();
                Ok(AppAction::None)
            }
            KeyCode::Delete => {
                self.query.clear();
                self.recompute_filter();
                Ok(AppAction::None)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.recompute_filter();
                Ok(AppAction::None)
            }
            KeyCode::Char(c) if text_modifier(key.modifiers) => {
                self.query.push(c);
                self.recompute_filter();
                Ok(AppAction::None)
            }
            _ => Ok(AppAction::None),
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) -> Result<AppAction> {
        let form = self.editor.as_mut().context("editor missing")?;
        form.error = None;
        match key.code {
            KeyCode::Esc => {
                self.editor = None;
                Ok(AppAction::None)
            }
            KeyCode::Tab => {
                form.next_field(false);
                Ok(AppAction::None)
            }
            KeyCode::BackTab => {
                form.next_field(true);
                Ok(AppAction::None)
            }
            KeyCode::Enter => Ok(AppAction::SaveManual),
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Ok(AppAction::SaveManual)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                form.field_value_mut().clear();
                Ok(AppAction::None)
            }
            KeyCode::Backspace => {
                form.field_value_mut().pop();
                Ok(AppAction::None)
            }
            KeyCode::Delete => {
                form.field_value_mut().clear();
                Ok(AppAction::None)
            }
            KeyCode::Char(c) if text_modifier(key.modifiers) => {
                form.field_value_mut().push(c);
                Ok(AppAction::None)
            }
            _ => Ok(AppAction::None),
        }
    }

    fn next_focus(&mut self, reverse: bool) {
        if self.focus == Focus::Manual {
            self.focus = Focus::Selected;
            return;
        }
        let order = [Focus::Subjects, Focus::Selected, Focus::Timetable];
        let index = order
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        let next = if reverse {
            if index == 0 {
                order.len() - 1
            } else {
                index - 1
            }
        } else {
            (index + 1) % order.len()
        };
        self.focus = order[next];
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.focus {
            Focus::Subjects | Focus::Search => {
                self.cursor = moved_index(self.cursor, self.filtered.len(), delta)
            }
            Focus::Selected => {
                self.selected_cursor =
                    moved_index(self.selected_cursor, self.selected.len(), delta);
            }
            Focus::Timetable => {
                self.timetable_viewport.offset = (self.timetable_viewport.offset as usize)
                    .saturating_add_signed(delta)
                    .min(self.timetable_viewport.max_offset as usize)
                    as u16;
            }
            Focus::Manual => {
                self.manual_cursor =
                    moved_index(self.manual_cursor, self.manual_visible_len(), delta)
            }
        }
    }

    pub fn toggle_current_subject(&mut self) {
        let Some(id) = self.current_subject().map(str::to_owned) else {
            return;
        };
        if !self.selected.insert(id.clone()) {
            self.selected.remove(&id);
        }
        self.invalidate("Selection changed. Optimizing automatically.");
    }

    pub fn current_selected(&self) -> Option<&str> {
        self.selected
            .iter()
            .nth(self.selected_cursor)
            .map(String::as_str)
    }

    fn remove_current_selected(&mut self) {
        if let Some(id) = self.current_selected().map(str::to_owned) {
            self.selected.remove(&id);
            self.invalidate("Selection changed. Optimizing automatically.");
        }
    }

    pub fn open_add_manual(&mut self) {
        let course = self
            .current_subject()
            .filter(|_| self.focus == Focus::Subjects)
            .map(str::to_owned)
            .or_else(|| {
                self.current_selected()
                    .filter(|_| self.focus == Focus::Selected)
                    .map(str::to_owned)
            })
            .or_else(|| self.selected.iter().next().cloned())
            .or_else(|| self.current_subject().map(str::to_owned))
            .or_else(|| self.base.courses.keys().next().cloned())
            .unwrap_or_default();
        let mut form = ManualForm::new(course.clone());
        if let Some(requirement) = self.base.courses.get(&course).and_then(|course| {
            course
                .requirements
                .iter()
                .find(|requirement| requirement.kind == "pe")
        }) {
            form.kind = "pe".to_string();
            if let Some(meeting) = requirement
                .options
                .iter()
                .flat_map(|option| &option.meetings)
                .next()
            {
                form.start_date = meeting
                    .start_date
                    .map(|date| date.to_string())
                    .unwrap_or_default();
                form.end_date = meeting
                    .end_date
                    .map(|date| date.to_string())
                    .unwrap_or_default();
            }
        }
        self.editor = Some(form);
        self.status = format!("Manual editor opened for {course}.");
    }

    pub fn open_edit_current_manual(&mut self) {
        let Some(index) = self.current_manual_index() else {
            self.open_add_manual();
            return;
        };
        let entry_id = self.manual.entries[index].option.id.clone();
        self.editor = Some(ManualForm::from_entry(&self.manual.entries[index]));
        self.status = format!("Editing manual entry {entry_id}.");
    }

    pub fn current_manual_index(&self) -> Option<usize> {
        self.manual
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.term_id == self.base.term_id)
            .nth(self.manual_cursor)
            .map(|(index, _)| index)
    }

    pub fn manual_visible_len(&self) -> usize {
        self.manual
            .entries
            .iter()
            .filter(|entry| entry.term_id == self.base.term_id)
            .count()
    }

    fn result_len(&self) -> usize {
        self.solution
            .as_ref()
            .map(|solution| solution.choices.len())
            .unwrap_or(0)
    }

    pub fn save_current_manual(&mut self) -> Result<()> {
        let Some(form) = self.editor.clone() else {
            return Ok(());
        };
        let mut meetings = storage::parse_meetings(&form.meetings).with_context(|| {
            format!(
                "invalid meetings '{}'; use e.g. Mon 09:00-10:00;Wed 09:00-10:00",
                form.meetings
            )
        })?;
        let start_date = parse_optional_date(&form.start_date, "start date")?;
        let end_date = parse_optional_date(&form.end_date, "end date")?;
        for meeting in &mut meetings {
            meeting.start_date = start_date;
            meeting.end_date = end_date;
        }
        let course_id = form.course_id.trim();
        let kind = form.kind.trim();
        let preserved_enabled = if let Some(id) = &form.entry_id {
            let existing = self
                .manual
                .entries
                .iter()
                .find(|entry| entry.term_id == self.base.term_id && entry.option.id == *id)
                .with_context(|| format!("manual section not found in this term: {id}"))?;
            ensure!(
                existing.course_id == course_id && existing.kind == kind,
                "manual edits cannot move an entry between subjects or components; add a new manual entry instead"
            );
            existing.enabled
        } else {
            form.enabled
        };
        let mut entry = app::manual_entry(
            &self.base,
            course_id,
            kind,
            form.label.trim(),
            form.room.trim(),
            meetings,
            form.entry_id.as_deref(),
        )?;
        entry.enabled = preserved_enabled;

        let mut manual = self.manual.clone();
        if let Some(existing) = manual.entries.iter_mut().find(|existing| {
            existing.term_id == entry.term_id && existing.option.id == entry.option.id
        }) {
            *existing = entry;
        } else {
            manual.entries.push(entry);
        }
        let dataset = storage::apply_manual(&self.base, &manual)?;
        storage::save_manual(&self.data_dir, &manual)?;
        self.manual = manual;
        self.dataset = dataset;
        self.editor = None;
        self.clamp_cursors();
        self.invalidate("Manual entries saved. Optimizing automatically.");
        Ok(())
    }

    pub fn toggle_current_manual_enabled(&mut self) -> Result<()> {
        let Some(index) = self.current_manual_index() else {
            self.status = "No manual entry is selected.".to_string();
            return Ok(());
        };
        let mut manual = self.manual.clone();
        let entry = manual
            .entries
            .get_mut(index)
            .context("manual entry disappeared")?;
        entry.enabled = !entry.enabled;
        let enabled = entry.enabled;
        let id = entry.option.id.clone();
        let dataset = storage::apply_manual(&self.base, &manual)?;
        storage::save_manual(&self.data_dir, &manual)?;
        self.manual = manual;
        self.dataset = dataset;
        self.invalidate(if enabled {
            "Manual entry enabled. Optimizing automatically."
        } else {
            "Manual entry disabled. Optimizing automatically."
        });
        self.status = format!("{} {id}", if enabled { "Enabled" } else { "Disabled" });
        Ok(())
    }

    pub fn install_solution(&mut self, solution: Solution) {
        self.result_cursor = 0;
        self.timetable_viewport = ScrollViewport::default();
        self.timetable_navigation = timetable::Navigation::default();
        self.actual_members.clear();
        self.export = None;
        self.optimize_running = false;
        self.status = match solution.status {
            SolveStatus::OptimalKnown => {
                if let Some(score) = solution.score {
                    format!(
                        "Optimal: {} occupied day(s), {} gap minute(s).",
                        score.occupied_days, score.gap_minutes
                    )
                } else {
                    "Optimal solution returned without a score.".to_string()
                }
            }
            SolveStatus::Infeasible => "Infeasible: known supported meetings conflict.".to_string(),
            SolveStatus::Cancelled => "Optimization cancelled.".to_string(),
        };
        self.solution = Some(solution);
        self.clamp_cursors();
    }

    pub fn cycle_current_member(&mut self, delta: isize) {
        let Some(solution) = &self.solution else {
            return;
        };
        if solution.status != SolveStatus::OptimalKnown || solution.choices.is_empty() {
            return;
        }
        let index = self
            .result_cursor
            .min(solution.choices.len().saturating_sub(1));
        let choice = &solution.choices[index];
        if choice.members.len() <= 1 {
            return;
        }
        let current = self
            .actual_members
            .get(&choice.requirement_id)
            .and_then(|id| choice.members.iter().position(|member| &member.id == id))
            .unwrap_or(0);
        let next = moved_index(current, choice.members.len(), delta);
        self.actual_members.insert(
            choice.requirement_id.clone(),
            choice.members[next].id.clone(),
        );
        self.export = None;
        self.status = format!(
            "{} now uses {}.",
            choice.requirement_id, choice.members[next].label
        );
    }

    pub fn export_solution(&mut self) -> Result<ExportSummary> {
        let solution = self
            .solution
            .as_ref()
            .context("optimize before exporting")?;
        ensure!(
            solution.status == SolveStatus::OptimalKnown,
            "only a feasible optimal solution can be exported"
        );
        let report =
            app::write_calendar(&self.dataset, solution, &self.actual_members, &self.output)?;
        let summary = ExportSummary {
            path: report.path.clone(),
            event_count: report.event_count,
            notices: report.notices,
        };
        self.status = format!(
            "Exported {} event(s) to {}.",
            summary.event_count,
            summary.path.display()
        );
        self.export = Some(summary.clone());
        Ok(summary)
    }

    fn invalidate(&mut self, message: &str) {
        self.persist_selection();
        self.generation = self.generation.wrapping_add(1);
        self.timetable_viewport = ScrollViewport::default();
        self.timetable_navigation = timetable::Navigation::default();
        self.solution = None;
        self.actual_members.clear();
        self.export = None;
        self.optimize_running = false;
        self.status = message.to_string();
        self.clamp_cursors();
    }

    fn clamp_cursors(&mut self) {
        let filtered_len = self.filtered.len();
        let manual_len = self.manual_visible_len();
        let result_len = self.result_len();
        clamp_index(&mut self.cursor, filtered_len);
        clamp_index(&mut self.selected_cursor, self.selected.len());
        clamp_index(&mut self.manual_cursor, manual_len);
        clamp_index(&mut self.result_cursor, result_len);
    }
}

struct OptimizeWorker {
    generation: u64,
    cancel: Arc<AtomicBool>,
    rx: Receiver<(u64, std::result::Result<Solution, String>)>,
    _handle: thread::JoinHandle<()>,
}

impl OptimizeWorker {
    fn spawn(dataset: Dataset, selected: Vec<String>, generation: u64) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = app::optimize(&dataset, &selected, Some(worker_cancel.as_ref()))
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send((generation, result));
        });
        Self {
            generation,
            cancel,
            rx,
            _handle: handle,
        }
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("enable terminal raw mode")?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error).context("enter terminal alternate screen");
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(mut terminal) => {
                terminal.clear().ok();
                Ok(Self { terminal })
            }
            Err(error) => {
                let _ = disable_raw_mode();
                let mut stdout = io::stdout();
                let _ = execute!(stdout, LeaveAlternateScreen, DisableMouseCapture);
                Err(error).context("initialize terminal")
            }
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

pub fn run(
    base: Dataset,
    manual: ManualStore,
    data_dir: PathBuf,
    output: PathBuf,
    initial: Vec<String>,
) -> Result<()> {
    run_with_options(base, manual, data_dir, output, initial, false)
}

pub fn run_with_options(
    base: Dataset,
    manual: ManualStore,
    data_dir: PathBuf,
    output: PathBuf,
    initial: Vec<String>,
    no_restore: bool,
) -> Result<()> {
    // Do not save startup overrides if no interactive terminal can be opened.
    let mut terminal = TerminalSession::enter()?;
    let mut app = AppState::new_with_session(base, manual, data_dir, output, initial, no_restore)?;
    let mut worker: Option<OptimizeWorker> = None;
    let tick = Duration::from_millis(100);
    let mut optimized_generation = None;
    let mut watcher_ready = false;

    loop {
        if !watcher_ready && app.selection_saved {
            // A rebuild can interrupt the initial network/catalog load. Only
            // suppress the launcher's seeds after they have actually been saved,
            // and acknowledge before displaying a UI the user can edit.
            if let Some(path) = std::env::var_os("HYDRANT_TUI_READY_FILE") {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(error).context("acknowledge saved selections to watcher");
                    }
                }
            }
            watcher_ready = true;
        }
        optimize_changed_selection(&mut app, &mut worker, &mut optimized_generation);
        drain_worker(&mut app, &mut worker);
        terminal.terminal.draw(|frame| draw(frame, &mut app))?;
        if app.should_quit {
            break;
        }

        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        break;
                    }
                    let action = app.handle_key(key)?;
                    match action {
                        AppAction::None => {}
                        AppAction::Quit => break,
                        AppAction::CancelOptimize => {
                            cancel_worker(&mut worker);
                            app.optimize_running = false;
                            app.status = "Cancellation requested.".to_string();
                        }
                        AppAction::Export => {
                            if let Err(error) = app.export_solution() {
                                app.status = format!("Export failed: {error:#}");
                            }
                        }
                        AppAction::SaveManual => {
                            if let Err(error) = app.save_current_manual() {
                                if let Some(form) = &mut app.editor {
                                    form.error = Some(format!("{error:#}"));
                                }
                                app.status = format!("Manual save failed: {error:#}");
                            }
                        }
                        AppAction::ToggleManualEnabled => {
                            if let Err(error) = app.toggle_current_manual_enabled() {
                                app.status = format!("Manual toggle failed: {error:#}");
                            }
                        }
                    }
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                Event::Resize(_, _) => app.focus_regions.clear(),
                _ => {}
            }
        }
    }

    cancel_worker(&mut worker);
    app.persist_selection();
    // Keep failure details available after leaving the alternate screen too.
    let notice = app.selection_notice.clone();
    drop(terminal);
    if let Some(notice) = notice {
        eprintln!("{notice}");
    }
    Ok(())
}

// Each generation is attempted once, so cancellation or errors do not restart it.
fn optimize_changed_selection(
    app: &mut AppState,
    worker: &mut Option<OptimizeWorker>,
    optimized_generation: &mut Option<u64>,
) {
    if *optimized_generation == Some(app.generation) {
        return;
    }
    *optimized_generation = Some(app.generation);
    cancel_worker(worker);
    if !app.selected.is_empty() {
        start_worker(app, worker);
    } else if app.generation != 0 {
        app.status = "No classes selected. Select classes to optimize automatically.".into();
    }
}

fn start_worker(app: &mut AppState, worker: &mut Option<OptimizeWorker>) {
    let selected = app.selected_subjects();
    if selected.is_empty() {
        app.status = "Select at least one subject before optimizing.".to_string();
        return;
    }
    let generation = app.generation;
    *worker = Some(OptimizeWorker::spawn(
        app.dataset.clone(),
        selected,
        generation,
    ));
    app.optimize_running = true;
    app.solution = None;
    app.export = None;
    app.status = "Optimizing in background. Press c to cancel.".to_string();
}

fn cancel_worker(worker: &mut Option<OptimizeWorker>) {
    if let Some(worker) = worker.take() {
        worker.cancel();
    }
}

fn drain_worker(app: &mut AppState, worker: &mut Option<OptimizeWorker>) {
    let Some(current) = worker.as_ref() else {
        return;
    };
    match current.rx.try_recv() {
        Ok((generation, result)) => {
            let worker_generation = current.generation;
            *worker = None;
            app.optimize_running = false;
            if generation != app.generation || worker_generation != app.generation {
                app.status = "Ignored stale optimization result.".to_string();
                return;
            }
            match result {
                Ok(solution) => app.install_solution(solution),
                Err(error) => app.status = format!("Optimization failed: {error}"),
            }
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            *worker = None;
            app.optimize_running = false;
            app.status = "Optimization worker stopped unexpectedly.".to_string();
        }
    }
}

fn draw(frame: &mut Frame<'_>, app: &mut AppState) {
    // Leave the terminal's final column unused. Pane hosts can clip that column
    // against their own border, and writing there can trigger terminal wrapping.
    // Derive every pane and overlay from this same safe area on each redraw.
    let mut area = frame.area();
    area.width = area.width.saturating_sub(1);
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(5),
        ])
        .split(area);

    draw_search(frame, app, root[0]);

    // Keep the calendar full-width beneath the subject and selection lists.
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(root[1]);
    let lists = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body[0]);
    // Hit targets use the actual rendered rectangles, including their borders.
    app.focus_regions = vec![
        (root[0], Focus::Search),
        (lists[0], Focus::Subjects),
        (lists[1], Focus::Selected),
        (body[1], Focus::Timetable),
    ];
    draw_subjects(frame, app, lists[0]);
    draw_selected(frame, app, lists[1]);
    timetable::draw(frame, app, body[1]);
    draw_footer(frame, app, root[2]);

    if app.focus == Focus::Manual {
        let popup = centered_rect(90, 70, area);
        frame.render_widget(Clear, popup);
        draw_manual(frame, app, popup);
    }
    if app.show_help {
        draw_help(frame, area);
    }
    if let Some(form) = &app.editor {
        draw_editor(frame, form, area);
    }
}

fn draw_search(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let active = app.focus == Focus::Search && app.editor.is_none() && !app.show_help;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .title(format!(
            "{}Search subjects | {}",
            if active { "> " } else { "" },
            if active {
                "Enter to browse · Ctrl+U clear"
            } else {
                "/ to type"
            }
        ))
        .title_bottom(format!(" {} ", app.base.term_id));
    let inner = block.inner(area);
    let text = if app.query.is_empty() && !active {
        Line::styled(
            "Search by course number or title…",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Line::from(app.query.clone())
    };
    let width = text.width();
    let scroll = if app.query.is_empty() {
        0
    } else {
        width
            .saturating_sub(inner.width.saturating_sub(1) as usize)
            .min(u16::MAX as usize) as u16
    };
    frame.render_widget(Paragraph::new(text).block(block).scroll((0, scroll)), area);
    if active && inner.width > 0 && inner.height > 0 {
        frame.set_cursor_position((
            inner.x
                + width
                    .saturating_sub(scroll as usize)
                    .min(inner.width as usize - 1) as u16,
            inner.y,
        ));
    }
}

fn draw_subjects(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let title = format!(
        "{}Subjects | {} found",
        if app.focus == Focus::Subjects {
            "> "
        } else {
            ""
        },
        app.filtered.len()
    );
    let items = app
        .filtered
        .iter()
        .map(|id| {
            let course = &app.base.courses[id];
            let checked = if app.selected.contains(id) {
                "[x]"
            } else {
                "[ ]"
            };
            let style = if app.selected.contains(id) {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{checked} "), style),
                Span::styled(
                    format!("{id:<10} "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(course.title.clone()),
            ]))
            .style(style)
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !app.filtered.is_empty() {
        state.select(Some(app.cursor));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.focus == Focus::Subjects {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .title(title);
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_symbol("> ")
            .highlight_style(Style::default().fg(Color::Yellow)),
        area,
        &mut state,
    );
}

fn draw_selected(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let active = app.focus == Focus::Selected;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .title(format!(
            "{}Selected classes | {}",
            if active { "> " } else { "" },
            app.selected.len()
        ))
        .title_bottom(" s focus · Space remove ");
    if app.selected.is_empty() {
        frame.render_widget(
            Paragraph::new("No classes selected. Space in Subjects to add.")
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
        return;
    }
    let items = app
        .selected
        .iter()
        .map(|id| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{id} "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(
                    app.base
                        .courses
                        .get(id)
                        .map(|course| course.title.clone())
                        .unwrap_or_default(),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(app.selected_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_symbol("> ")
            .highlight_style(Style::default().fg(Color::Yellow)),
        area,
        &mut state,
    );
}

fn draw_manual(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let entries = app
        .manual
        .entries
        .iter()
        .filter(|entry| entry.term_id == app.base.term_id)
        .collect::<Vec<_>>();
    let items = entries
        .iter()
        .map(|entry| {
            let enabled = if entry.enabled { "on" } else { "off" };
            let meetings = entry
                .option
                .meetings
                .iter()
                .map(Meeting::display)
                .collect::<Vec<_>>()
                .join("; ");
            let style = if entry.enabled {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(format!(
                "[{enabled}] {} {} {}  {}  {}",
                entry.course_id, entry.kind, entry.option.label, entry.option.room, meetings
            ))
            .style(style)
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !entries.is_empty() {
        state.select(Some(app.manual_cursor.min(entries.len() - 1)));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.focus == Focus::Manual {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .title(if app.focus == Focus::Manual {
            "> Manual entries | a add · x toggle · Esc close"
        } else {
            "Manual entries | a add · x toggle · Esc close"
        });
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_symbol("> ")
            .highlight_style(Style::default().fg(Color::Yellow)),
        area,
        &mut state,
    );
}

fn draw_footer(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let text = vec![
        Line::from(app.status.clone()),
        match &app.selection_notice {
            Some(notice) => Line::styled(notice.clone(), Style::default().fg(Color::Yellow)),
            None => Line::from("/ search  s selected  c cancel  t timetable  e export  ? help"),
        },
        Line::from(if app.focus == Focus::Manual {
            "Enter edit | a add | x toggle | Esc/m close manual entries"
        } else if app.focus == Focus::Timetable {
            app.timetable_navigation.hint()
        } else {
            "Click pane to focus | h/l ←/→ panes | j/k ↑/↓ move | Space select/remove | m manual | q quit"
        }),
    ];
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    let popup = if area.width < 100 || area.height < 32 {
        area
    } else {
        let mut popup = centered_rect(76, 54, area);
        popup.height = popup.height.max(24);
        popup.y = area.y + (area.height - popup.height) / 2;
        popup
    };
    frame.render_widget(Clear, popup);
    let help = vec![
        Line::from(Span::styled(
            "Hydrant Optimizer keymap",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("/                  Search subjects. Enter to browse, Ctrl+U to clear."),
        Line::from("h/l or Left/Right  Previous/next pane. Tab/Shift+Tab or click also works."),
        Line::from("j/k or Down/Up     Move within the active pane."),
        Line::from("Space or Enter     Select a subject / remove a selected class."),
        Line::from("a / m              Add manual section / manual overlay (Esc closes)."),
        Line::from("Enter / x          Edit / enable-disable the selected manual entry."),
        Line::from("c                  Cancel background optimization."),
        Line::from("s / t              Focus selected classes / timetable."),
        Line::from("PgUp/PgDn          Scroll the timetable."),
        Line::from("Home/End           First/last line in the timetable."),
        Line::from(
            "Timetable: h/l choices, Enter blocks, hjkl move, Enter members, h/l switch, Esc back.",
        ),
        Line::from("e                  Export chosen sections to a new local ICS file."),
        Line::from("q / Esc            Quit. Esc closes search, help, or an editor first."),
        Line::from(""),
        Line::from("Editor: Tab fields, Ctrl+U clear, Enter/Ctrl+S save, Esc cancel."),
        Line::from("Meetings: Mon 09:00-10:00;Wed 09:00-10:00. Exact minutes are kept."),
        Line::from("Selection or manual edits automatically re-optimize the timetable."),
        Line::from("Press ? or Esc to close help."),
    ];
    frame.render_widget(
        Paragraph::new(help)
            .block(Block::default().borders(Borders::ALL).title("Help"))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn draw_editor(frame: &mut Frame<'_>, form: &ManualForm, area: Rect) {
    let popup = centered_rect(78, 68, area);
    frame.render_widget(Clear, popup);
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        if form.entry_id.is_some() {
            "Edit manual entry"
        } else {
            "Add manual entry"
        },
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(
        "Tab/BackTab move fields, Ctrl+S or Enter save, Esc cancel.",
    ));
    lines.push(Line::from(""));
    for field in EditorField::all() {
        let marker = if *field == form.field { ">" } else { " " };
        let value = match field {
            EditorField::Course => &form.course_id,
            EditorField::Kind => &form.kind,
            EditorField::Label => &form.label,
            EditorField::Meetings => &form.meetings,
            EditorField::Room => &form.room,
            EditorField::StartDate => &form.start_date,
            EditorField::EndDate => &form.end_date,
        };
        let style = if *field == form.field {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} {:<14} ", field.label()), style),
            Span::raw(value.clone()),
            if *field == form.field {
                Span::styled("_", Style::default().fg(Color::Yellow))
            } else {
                Span::raw("")
            },
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(format!(
        "Enabled: {} (x toggles from manual list after saving)",
        form.enabled
    )));
    if let Some(id) = &form.entry_id {
        lines.push(Line::from(format!("ID: {id}")));
    }
    if let Some(error) = &form.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Manual editor"),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn current_member<'a>(
    choice: &'a TimeChoice,
    actual_members: &BTreeMap<String, String>,
) -> Option<&'a SectionOption> {
    actual_members
        .get(&choice.requirement_id)
        .and_then(|id| choice.members.iter().find(|member| &member.id == id))
        .or_else(|| choice.members.first())
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn moved_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    (current as isize + delta).rem_euclid(len) as usize
}

fn clamp_index(index: &mut usize, len: usize) {
    if len == 0 {
        *index = 0;
    } else if *index >= len {
        *index = len - 1;
    }
}

fn text_modifier(modifiers: KeyModifiers) -> bool {
    !modifiers.contains(KeyModifiers::CONTROL) && !modifiers.contains(KeyModifiers::ALT)
}

fn parse_optional_date(value: &str, label: &str) -> Result<Option<NaiveDate>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<NaiveDate>()
        .map(Some)
        .with_context(|| format!("invalid {label}; expected YYYY-MM-DD"))
}

fn common_date(meetings: &[Meeting], start: bool) -> Option<String> {
    let mut dates = meetings.iter().map(|meeting| {
        if start {
            meeting.start_date
        } else {
            meeting.end_date
        }
    });
    let first = dates.next().flatten()?;
    if dates.all(|date| date == Some(first)) {
        Some(first.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod viewport_tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn fixture_state(dir: &std::path::Path) -> AppState {
        let dataset = crate::adapter::parse_catalog(
            include_str!("../tests/fixtures/catalog.json"),
            include_str!("../tests/fixtures/term.json"),
        )
        .unwrap();
        let selected = vec!["A".to_string(), "B".to_string()];
        let mut solution = app::optimize(&dataset, &selected, None).unwrap();
        for i in 0..12 {
            solution
                .unresolved
                .push(format!("Notice {i:02}: unannounced meeting"));
        }
        let mut state = AppState::new(
            dataset,
            ManualStore::default(),
            dir.into(),
            dir.join("out.ics"),
            selected,
        )
        .unwrap();
        state.install_solution(solution);
        state
    }

    fn render(state: &mut AppState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn press(state: &mut AppState, code: KeyCode) {
        state
            .handle_key(KeyEvent::new(code, KeyModifiers::NONE))
            .unwrap();
    }

    fn click(state: &mut AppState, column: u16, row: u16) {
        state.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn clicks_focus_rendered_panes_without_changing_selection() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        let selected = state.selected.clone();
        let generation = state.generation;
        for (width, height) in [(80, 24), (140, 50), (40, 16)] {
            render(&mut state, width, height);
            for (area, focus) in state.focus_regions.clone() {
                click(&mut state, area.x, area.y);
                assert_eq!(state.focus, focus, "pane border");
                click(
                    &mut state,
                    area.x + area.width / 2,
                    area.y + area.height / 2,
                );
                assert_eq!(state.focus, focus, "pane interior");
            }
            let focus = state.focus;
            click(&mut state, width - 1, 0);
            click(&mut state, 0, height - 1);
            assert_eq!(state.focus, focus, "unused column and footer are inert");
        }
        assert_eq!(state.selected, selected);
        assert_eq!(state.generation, generation);
        click(&mut state, 1, 1);
        press(&mut state, KeyCode::Char('A'));
        assert_eq!(state.query, "A");
    }

    #[test]
    fn mouse_focus_respects_overlays_and_ignores_non_click_events() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        render(&mut state, 80, 24);
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::ScrollDown,
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            state.handle_mouse(MouseEvent {
                kind,
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(state.focus, Focus::Subjects);
        }
        state.show_help = true;
        click(&mut state, 1, 1);
        assert_eq!(state.focus, Focus::Subjects);
        state.show_help = false;
        state.editor = Some(ManualForm::new("A".into()));
        click(&mut state, 1, 1);
        assert_eq!(state.focus, Focus::Subjects);
        state.editor = None;
        state.focus = Focus::Manual;
        click(&mut state, 1, 1);
        assert_eq!(state.focus, Focus::Manual);
    }

    #[test]
    fn automatic_optimization_tracks_changes_and_respects_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        let mut worker = None;
        let mut generation = None;
        optimize_changed_selection(&mut state, &mut worker, &mut generation);
        assert!(
            state.optimize_running,
            "preselected classes optimize on startup"
        );
        let old_cancel = Arc::clone(&worker.as_ref().unwrap().cancel);
        state.focus = Focus::Selected;
        press(&mut state, KeyCode::Enter);
        optimize_changed_selection(&mut state, &mut worker, &mut generation);
        assert!(old_cancel.load(Ordering::Relaxed));
        assert_eq!(worker.as_ref().unwrap().generation, state.generation);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while worker.is_some() {
            assert!(std::time::Instant::now() < deadline);
            drain_worker(&mut state, &mut worker);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            state.solution.as_ref().unwrap().status,
            SolveStatus::OptimalKnown
        );
        assert_eq!(state.selected.len(), 1);
        assert_eq!(
            state
                .handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
                .unwrap(),
            AppAction::None
        );
        press(&mut state, KeyCode::Char(' '));
        optimize_changed_selection(&mut state, &mut worker, &mut generation);
        assert!(state.selected.is_empty());
        assert!(state.solution.is_none());
        assert!(worker.is_none());
        assert!(!state.optimize_running);
        state.focus = Focus::Subjects;
        press(&mut state, KeyCode::Char(' '));
        optimize_changed_selection(&mut state, &mut worker, &mut generation);
        assert!(worker.is_some(), "adding a class starts optimization");
        cancel_worker(&mut worker);
        state.optimize_running = false;
        optimize_changed_selection(&mut state, &mut worker, &mut generation);
        assert!(worker.is_none(), "cancelled generation must not restart");
        state.open_add_manual();
        state.editor.as_mut().unwrap().meetings = "Mon 09:00-10:00".into();
        state.save_current_manual().unwrap();
        optimize_changed_selection(&mut state, &mut worker, &mut generation);
        assert!(worker.is_some(), "manual changes trigger optimization");
        cancel_worker(&mut worker);
    }

    #[test]
    fn panes_keep_right_borders_inside_the_terminal_after_resize() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        let mut terminal = Terminal::new(TestBackend::new(107, 61)).unwrap();
        for (width, height) in [(107, 61), (80, 24), (53, 24), (170, 55), (107, 61)] {
            terminal.backend_mut().resize(width, height);
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let buffer = terminal.backend().buffer();
            let right = width - 2;
            for (row, border) in [
                (0, "┐"),
                (1, "│"),
                (2, "┘"),
                (3, "┐"),
                (height - 5, "┐"),
                (height - 1, "┘"),
            ] {
                assert_eq!(
                    buffer[(right, row)].symbol(),
                    border,
                    "{width}x{height} row {row}"
                );
            }
            let table_top = (0..height)
                .find(|&y| {
                    buffer[(0, y)].symbol() == "┌"
                        && (0..width).any(|x| buffer[(x, y)].symbol() == "┬")
                })
                .unwrap();
            assert_eq!(buffer[(right, table_top)].symbol(), "┐");
            assert_eq!(buffer[(right, table_top + 1)].symbol(), "│");
            for y in 0..height {
                assert_eq!(
                    buffer[(width - 1, y)].symbol(),
                    " ",
                    "right gutter at {width}x{height}"
                );
            }
        }
    }

    #[test]
    fn overlays_and_search_caret_respect_the_right_gutter_at_all_sizes() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        state.query = "界α".repeat(100);
        for (width, height) in [(107, 61), (80, 24), (36, 12), (12, 5), (1, 1), (0, 0)] {
            for mode in 0..4 {
                state.focus = if mode == 1 {
                    Focus::Manual
                } else {
                    Focus::Search
                };
                state.show_help = mode == 2;
                state.editor = (mode == 3).then(|| ManualForm::new("A".into()));
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| draw(frame, &mut state)).unwrap();
                if width > 0 {
                    for y in 0..height {
                        assert_eq!(terminal.backend().buffer()[(width - 1, y)].symbol(), " ");
                    }
                }
                if mode == 0 && width >= 12 {
                    let cursor = terminal.get_cursor_position().unwrap();
                    assert!(
                        cursor.x < width - 2,
                        "caret must stay inside the search border"
                    );
                }
            }
        }
    }

    #[test]
    fn search_replaces_status_header_and_week_has_full_width_at_80_columns() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        let screen = render(&mut state, 80, 24);
        let header = screen.chars().take(240).collect::<String>();
        assert!(header.contains("Search subjects"));
        assert!(header.contains("Search by course number or title"));
        assert!(!header.contains("Status"));
        assert!(screen.contains("> Subjects"));
        for day in ["Mon", "Tue", "Wed", "Thu", "Fri"] {
            assert!(screen.contains(day), "missing {day}: {screen}");
        }
        assert!(screen.contains("08:00"));
        assert!(screen.contains("09:00"));
        press(&mut state, KeyCode::Char('l'));
        assert!(render(&mut state, 80, 24).contains("> Selected classes"));
        press(&mut state, KeyCode::Right);
        assert!(render(&mut state, 80, 24).contains("> Timetable"));
        press(&mut state, KeyCode::Char('/'));
        for c in "hello hjkl".chars() {
            press(&mut state, KeyCode::Char(c));
        }
        let screen = render(&mut state, 80, 24);
        let header = screen.chars().take(240).collect::<String>();
        assert!(header.contains("hello hjkl"));
        assert!(header.contains("> Search subjects"));
        assert!(screen.contains("0 found"));
    }

    #[test]
    fn removed_results_shortcuts_do_not_focus_a_hidden_pane() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        for focus in [Focus::Subjects, Focus::Selected, Focus::Timetable] {
            state.focus = focus;
            for code in ['r', 'n', 'p'] {
                press(&mut state, KeyCode::Char(code));
                assert_eq!(state.focus, focus);
                assert!(state.actual_members.is_empty());
            }
        }
        assert!(!KEYMAP.contains("results"));
    }

    #[test]
    fn subjects_and_selected_share_upper_row_without_results() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        for (width, height) in [(80, 24), (120, 40)] {
            let screen = render(&mut state, width, height);
            for title in ["Subjects", "Selected classes", "Timetable"] {
                assert!(
                    screen.contains(title),
                    "missing {title} at {width}x{height}"
                );
            }
            assert!(!screen.contains("Manual entries"));
            assert!(!screen.contains("Results"));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| timetable::draw(frame, &mut state, frame.area()))
                .unwrap();
            let calendar = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            for day in ["Mon", "Tue", "Wed", "Thu", "Fri"] {
                assert!(calendar.contains(day));
            }
            assert!(!calendar.contains("Score:"));
            assert!(!calendar.contains("Notice 11"));
        }
        state.query = "nonexistent".into();
        state.recompute_filter();
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|frame| draw_selected(frame, &state, frame.area()))
            .unwrap();
        let selected = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(selected.contains("A ") && selected.contains("B "));
        press(&mut state, KeyCode::Char('m'));
        assert!(render(&mut state, 80, 24).contains("Manual entries"));
        press(&mut state, KeyCode::Esc);
        assert!(!render(&mut state, 80, 24).contains("Manual entries"));
        assert!(!state.should_quit);
    }

    #[test]
    fn timetable_uses_its_full_width_without_a_parent_box_or_legend() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        for (width, height) in [(37, 12), (80, 24), (120, 40), (170, 55)] {
            for focus in [Focus::Subjects, Focus::Timetable] {
                state.focus = focus;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| timetable::draw(frame, &mut state, frame.area()))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let rows = buffer
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>();
                let active = focus == Focus::Timetable;
                assert!(rows[0].starts_with(if active { "> Timetable" } else { "Timetable" }));
                assert!(!rows[0].contains('─'), "heading must not be a box border");
                assert_eq!(
                    buffer[(0, 0)].fg,
                    if active { Color::Cyan } else { Color::Reset }
                );
                assert_eq!(buffer[(0, 1)].symbol(), "┌");
                assert_eq!(buffer[(width - 1, 1)].symbol(), "┐");
                assert!(rows[2].starts_with("│Time"));
                assert_eq!(rows[2].matches('│').count(), 7, "no extra parent sides");
                assert_eq!(state.timetable_viewport.height, (height - 5).min(48));
                assert_eq!(
                    state.timetable_viewport.max_offset,
                    48u16.saturating_sub(height - 5)
                );
                let bottom = rows
                    .iter()
                    .position(|row| row.starts_with('└'))
                    .expect("closing rule stays visible");
                assert!(rows[bottom].ends_with('┘'));
                assert_eq!(rows[bottom].matches('┴').count(), 5);
                if height == 40 {
                    assert!(
                        rows.iter().any(|row| row.starts_with("│14:00")),
                        "afternoon is visible without scrolling"
                    );
                }
                assert!(
                    rows[bottom + 1..].iter().all(|row| row.trim().is_empty()),
                    "no legend, parent sides, or bottom border beneath the grid"
                );
            }
        }
    }

    #[test]
    fn timetable_scrolls_and_resets_on_invalidation() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        // Long daily span forces a real scroll in the normal 80x24 layout.
        let choice = &mut state.solution.as_mut().unwrap().choices[0];
        choice.meetings[0].end_minute = 23 * 60;
        for member in &mut choice.members {
            member.meetings = choice.meetings.clone();
        }
        render(&mut state, 80, 24);
        assert!(state.timetable_viewport.max_offset > 0);
        press(&mut state, KeyCode::Char('t'));
        press(&mut state, KeyCode::Home);
        for key in [KeyCode::Char('j'), KeyCode::Down] {
            press(&mut state, key);
        }
        assert_eq!(state.timetable_viewport.offset, 2);
        for key in [KeyCode::Char('k'), KeyCode::Up] {
            press(&mut state, key);
        }
        assert_eq!(state.timetable_viewport.offset, 0);
        press(&mut state, KeyCode::PageDown);
        assert!(state.timetable_viewport.offset > 0);
        press(&mut state, KeyCode::End);
        let timetable_offset = state.timetable_viewport.offset;
        assert_eq!(timetable_offset, state.timetable_viewport.max_offset);
        assert_eq!(state.timetable_viewport.offset, timetable_offset);
        press(&mut state, KeyCode::Char('t'));
        assert_eq!(state.timetable_viewport.offset, 16);
        press(&mut state, KeyCode::End);
        render(&mut state, 200, 160);
        assert_eq!(
            state.timetable_viewport.offset, 0,
            "resize clamps calendar offset"
        );
        assert_eq!(state.timetable_viewport.max_offset, 0);
        render(&mut state, 80, 24);
        press(&mut state, KeyCode::End);
        state.focus = Focus::Selected;
        press(&mut state, KeyCode::Char(' '));
        assert!(state.solution.is_none());
        assert_eq!(state.timetable_viewport.offset, 0);
        assert_eq!(state.timetable_viewport.max_offset, 0);
        assert!(render(&mut state, 80, 24).contains("08:00"));
        for (width, height) in [(36, 12), (12, 5), (1, 1), (0, 0)] {
            render(&mut state, width, height);
        }
    }

    #[test]
    fn long_unicode_search_keeps_caret_and_tail_visible_after_resize() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        state.focus = Focus::Search;
        state.query = format!("{}TAIL", "界α".repeat(50));
        for (width, height) in [(80, 24), (36, 12), (12, 5), (1, 1), (0, 0), (120, 40)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            if width >= 36 {
                let screen = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(
                    screen.contains("TAIL"),
                    "search tail clipped at {width}x{height}"
                );
                let cursor = terminal.get_cursor_position().unwrap();
                assert!(cursor.x < width && cursor.y < 3, "caret outside search bar");
            }
        }
    }

    #[test]
    fn help_shows_all_new_controls_and_close_hint_at_common_sizes() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = fixture_state(temp.path());
        state.show_help = true;
        for (width, height) in [(80, 24), (120, 32), (170, 55)] {
            let screen = render(&mut state, width, height);
            for text in [
                "h/l or Left/Right",
                "j/k or Down/Up",
                "Scroll the timetable.",
                "First/last line in the timetable",
                "Timetable: h/l choices",
                "Press ? or Esc to close help.",
            ] {
                assert!(screen.contains(text), "missing {text} at {width}x{height}");
            }
        }
    }
}
