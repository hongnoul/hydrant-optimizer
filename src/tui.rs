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
        KeyModifiers,
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

use crate::{app, model::*, storage};

pub const KEYMAP: &str = "q/Esc quit, / search, Ctrl+U clear search, Up/Down move, Space select subject, Tab next panel/field, m manual, a add manual, Enter edit/toggle, x enable-disable manual, o optimize/cancel, c cancel, n/p or Left/Right switch same-time member, e export, ? help, Ctrl+S save editor";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Subjects,
    Search,
    Manual,
    Results,
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
    OptimizeOrCancel,
    CancelOptimize,
    Export,
    SaveManual,
    ToggleManualEnabled,
}

#[derive(Debug)]
pub struct AppState {
    base: Dataset,
    pub dataset: Dataset,
    pub manual: ManualStore,
    pub data_dir: PathBuf,
    pub output: PathBuf,
    pub query: String,
    pub filtered: Vec<String>,
    pub cursor: usize,
    pub selected: BTreeSet<String>,
    pub focus: Focus,
    pub manual_cursor: usize,
    pub result_cursor: usize,
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
            dataset,
            manual,
            data_dir,
            output,
            query: String::new(),
            filtered: Vec::new(),
            cursor: 0,
            selected,
            focus: Focus::Subjects,
            manual_cursor: 0,
            result_cursor: 0,
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
                "Preselected {} subject(s). Press o to optimize.",
                state.selected.len()
            );
        }
        Ok(state)
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

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<AppAction> {
        if self.show_help {
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Enter => {
                    self.show_help = false;
                    return Ok(AppAction::None);
                }
                _ => {}
            }
        }

        if self.editor.is_some() {
            return self.handle_editor_key(key);
        }

        if self.focus == Focus::Search {
            return self.handle_search_key(key);
        }

        match key.code {
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
                self.move_cursor(-10);
                Ok(AppAction::None)
            }
            KeyCode::PageDown => {
                self.move_cursor(10);
                Ok(AppAction::None)
            }
            KeyCode::Char(' ') => {
                if self.focus == Focus::Subjects {
                    self.toggle_current_subject();
                }
                Ok(AppAction::None)
            }
            KeyCode::Enter => match self.focus {
                Focus::Subjects => {
                    self.toggle_current_subject();
                    Ok(AppAction::None)
                }
                Focus::Manual => {
                    self.open_edit_current_manual();
                    Ok(AppAction::None)
                }
                Focus::Results => {
                    self.cycle_current_member(1);
                    Ok(AppAction::None)
                }
                Focus::Search => Ok(AppAction::None),
            },
            KeyCode::Char('o') => Ok(AppAction::OptimizeOrCancel),
            KeyCode::Char('c') => Ok(AppAction::CancelOptimize),
            KeyCode::Char('e') => Ok(AppAction::Export),
            KeyCode::Char('m') => {
                self.focus = Focus::Manual;
                Ok(AppAction::None)
            }
            KeyCode::Char('r') => {
                self.focus = Focus::Results;
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
            KeyCode::Left | KeyCode::Char('p') => {
                if self.focus == Focus::Results {
                    self.cycle_current_member(-1);
                }
                Ok(AppAction::None)
            }
            KeyCode::Right | KeyCode::Char('n') => {
                if self.focus == Focus::Results {
                    self.cycle_current_member(1);
                }
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
        let order = [Focus::Subjects, Focus::Manual, Focus::Results];
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
            Focus::Manual => {
                self.manual_cursor =
                    moved_index(self.manual_cursor, self.manual_visible_len(), delta)
            }
            Focus::Results => {
                self.result_cursor = moved_index(self.result_cursor, self.result_len(), delta)
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
        self.invalidate("Selection changed. Press o to optimize.");
    }

    pub fn open_add_manual(&mut self) {
        let course = self
            .selected
            .iter()
            .next()
            .cloned()
            .or_else(|| self.current_subject().map(str::to_owned))
            .or_else(|| self.base.courses.keys().next().cloned())
            .unwrap_or_default();
        self.editor = Some(ManualForm::new(course.clone()));
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
        self.invalidate("Manual entries saved. Press o to re-optimize.");
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
            "Manual entry enabled. Press o to re-optimize."
        } else {
            "Manual entry disabled. Press o to re-optimize."
        });
        self.status = format!("{} {id}", if enabled { "Enabled" } else { "Disabled" });
        Ok(())
    }

    pub fn install_solution(&mut self, solution: Solution) {
        self.result_cursor = 0;
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
            path: self.output.clone(),
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
        self.generation = self.generation.wrapping_add(1);
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
    let mut app = AppState::new(base, manual, data_dir, output, initial)?;
    let mut terminal = TerminalSession::enter()?;
    let mut worker: Option<OptimizeWorker> = None;
    let tick = Duration::from_millis(100);

    loop {
        drain_worker(&mut app, &mut worker);
        terminal.terminal.draw(|frame| draw(frame, &app))?;
        if app.should_quit {
            break;
        }

        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let before_generation = app.generation;
                    let action = app.handle_key(key)?;
                    if app.generation != before_generation {
                        cancel_worker(&mut worker);
                    }
                    match action {
                        AppAction::None => {}
                        AppAction::Quit => break,
                        AppAction::OptimizeOrCancel => {
                            if worker.is_some() {
                                cancel_worker(&mut worker);
                                app.optimize_running = false;
                                app.status = "Cancellation requested.".to_string();
                            } else {
                                start_worker(&mut app, &mut worker);
                            }
                        }
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
                            if app.generation != before_generation {
                                cancel_worker(&mut worker);
                            }
                        }
                        AppAction::ToggleManualEnabled => {
                            if let Err(error) = app.toggle_current_manual_enabled() {
                                app.status = format!("Manual toggle failed: {error:#}");
                            }
                            if app.generation != before_generation {
                                cancel_worker(&mut worker);
                            }
                        }
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    cancel_worker(&mut worker);
    Ok(())
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
    app.status = "Optimizing in background. Press o or c to cancel.".to_string();
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

fn draw(frame: &mut Frame<'_>, app: &AppState) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let title = format!(
        "Hydrant Optimizer | term {} | selected {} | {}",
        app.base.term_id,
        app.selected.len(),
        if app.optimize_running {
            "optimizing"
        } else {
            "idle"
        }
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )]))
        .block(Block::default().borders(Borders::ALL).title("Status")),
        root[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(root[1]);
    draw_subjects(frame, app, body[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(body[1]);
    draw_manual(frame, app, right[0]);
    draw_results(frame, app, right[1]);
    draw_footer(frame, app, root[2]);

    if app.show_help {
        draw_help(frame, frame.area());
    }
    if let Some(form) = &app.editor {
        draw_editor(frame, form, frame.area());
    }
}

fn draw_subjects(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let title = if app.focus == Focus::Search {
        format!("Subjects search: {}_", app.query)
    } else {
        format!(
            "Subjects (/ search: {})",
            if app.query.is_empty() {
                "all"
            } else {
                &app.query
            }
        )
    };
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
                    format!("{:<10}", id),
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
        .border_style(if matches!(app.focus, Focus::Subjects | Focus::Search) {
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
        .title("Manual entries (a add, Enter edit, x on/off)");
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_symbol("> ")
            .highlight_style(Style::default().fg(Color::Yellow)),
        area,
        &mut state,
    );
}

fn draw_results(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let mut lines = Vec::new();
    if app.optimize_running {
        lines.push(Line::from(Span::styled(
            "Optimizing in background. Press o or c to cancel.",
            Style::default().fg(Color::Yellow),
        )));
    }
    if let Some(solution) = &app.solution {
        lines.push(Line::from(format!("Status: {:?}", solution.status)));
        if let Some(score) = solution.score {
            lines.push(Line::from(format!(
                "Score: {} occupied day(s), {} gap minute(s)",
                score.occupied_days, score.gap_minutes
            )));
        }
        if solution.status == SolveStatus::OptimalKnown {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Timetable",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            match timetable_lines(app) {
                Ok(rows) if rows.is_empty() => {
                    lines.push(Line::from("No dated weekly meetings in chosen sections."))
                }
                Ok(rows) => lines.extend(rows.into_iter().map(Line::from)),
                Err(error) => lines.push(Line::from(Span::styled(
                    format!("Cannot render timetable: {error:#}"),
                    Style::default().fg(Color::Red),
                ))),
            }
        }
        if !solution.choices.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Same-time actual sections (n/p switch highlighted)",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for (index, choice) in solution.choices.iter().enumerate() {
                let marker = if index == app.result_cursor { ">" } else { " " };
                let current = current_member(choice, &app.actual_members);
                let suffix = if choice.members.len() > 1 {
                    let pos = current
                        .and_then(|member| {
                            choice
                                .members
                                .iter()
                                .position(|candidate| candidate.id == member.id)
                        })
                        .unwrap_or(0)
                        + 1;
                    format!(" ({pos}/{})", choice.members.len())
                } else {
                    String::new()
                };
                let text = if let Some(member) = current {
                    format!(
                        "{marker} {}: {} {}{}",
                        choice.requirement_id, member.label, member.room, suffix
                    )
                } else {
                    format!("{marker} {}: no members", choice.requirement_id)
                };
                lines.push(Line::from(text));
            }
        }
        if !solution.unresolved.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Notices",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.extend(
                solution
                    .unresolved
                    .iter()
                    .take(6)
                    .map(|notice| Line::from(format!("- {notice}"))),
            );
            if solution.unresolved.len() > 6 {
                lines.push(Line::from(format!(
                    "... {} more",
                    solution.unresolved.len() - 6
                )));
            }
        }
    } else if !app.optimize_running {
        lines.push(Line::from(
            "No current result. Select subjects and press o.",
        ));
    }
    if let Some(export) = &app.export {
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "Exported {} event(s) to {}",
            export.event_count,
            export.path.display()
        )));
        for notice in export.notices.iter().take(3) {
            lines.push(Line::from(format!("Export notice: {notice}")));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.focus == Focus::Results {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .title("Result, timetable, notices");
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, app: &AppState, area: Rect) {
    let notices = app
        .dataset
        .notices
        .iter()
        .take(2)
        .map(|notice| format!("Notice: {notice}"))
        .collect::<Vec<_>>()
        .join(" | ");
    let text = vec![
        Line::from(app.status.clone()),
        Line::from(KEYMAP),
        Line::from(if notices.is_empty() {
            format!("Output: {}", app.output.display())
        } else {
            format!("Output: {} | {notices}", app.output.display())
        }),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(76, 54, area);
    frame.render_widget(Clear, popup);
    let help = vec![
        Line::from(Span::styled(
            "Hydrant Optimizer keymap",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(KEYMAP),
        Line::from(""),
        Line::from(
            "Manual editor fields: subject, component (lecture/recitation/lab/design), label, meetings, room, start date, end date.",
        ),
        Line::from("Meetings are atomic bundles: Mon 09:00-10:00;Wed 09:00-10:00."),
        Line::from(
            "Manual and selection edits clear old optimization/export results and reapply the immutable base catalog plus manual overlay.",
        ),
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

fn timetable_lines(app: &AppState) -> Result<Vec<String>> {
    let solution = app.solution.as_ref().context("no solution")?;
    let actual = app::actual_sections(&app.dataset, solution, &app.actual_members)?;
    let mut rows = Vec::new();
    for section in actual {
        for meeting in &section.section.meetings {
            rows.push((
                meeting.weekday,
                meeting.start_minute,
                format!(
                    "{}  {} {}  {}  {}",
                    meeting.display(),
                    section.course_id,
                    section.kind,
                    section.section.label,
                    section.section.room
                ),
            ));
        }
    }
    rows.sort();
    Ok(rows.into_iter().map(|(_, _, line)| line).collect())
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
