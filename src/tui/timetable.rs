//! Edge-to-edge timetable with its own scrolling viewport.
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Paragraph, Wrap},
};

use super::{AppState, Focus, week};
use crate::{app, model::SolveStatus};

fn lines(app: &AppState, width: u16) -> Vec<Line<'static>> {
    if let Some(solution) = &app.solution {
        if solution.status == SolveStatus::OptimalKnown {
            return match app::actual_sections(&app.dataset, solution, &app.actual_members) {
                Ok(sections) => week::week_lines_focused(
                    &sections,
                    width,
                    solution
                        .choices
                        .get(app.result_cursor)
                        .filter(|_| {
                            app.focus != Focus::Timetable
                                || app.timetable_navigation.level != Level::Timetables
                        })
                        .map(|choice| choice.requirement_id.as_str()),
                    if app.focus == Focus::Timetable
                        && app.timetable_navigation.level != Level::Timetables
                    {
                        app.session_blocks()
                            .get(app.timetable_navigation.block)
                            .map(|b| (b.day, b.start, b.end))
                    } else {
                        None
                    },
                ),
                Err(error) => vec![Line::styled(
                    format!("Cannot render timetable: {error:#}"),
                    Style::default().fg(Color::Red),
                )],
            };
        }
        let mut sheet = week::week_lines_focused(&[], width, None, None);
        sheet.push(Line::from(format!(
            "No feasible timetable: {:?}.",
            solution.status
        )));
        return sheet;
    }
    let mut sheet = week::week_lines_focused(&[], width, None, None);
    sheet.push(Line::from(if app.optimize_running {
        "Optimizing. The timetable will appear here."
    } else {
        "No timetable yet. Select classes to optimize automatically."
    }));
    sheet
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut AppState, area: Rect) {
    let active = app.focus == Focus::Timetable;
    // Keep a single fixed heading for focus and scrolling, but no parent border
    // or inset around the grid. Its own table border reaches both screen edges.
    let header_height = area.height.min(1);
    let header = Rect::new(area.x, area.y, area.width, header_height);
    let grid = Rect::new(
        area.x,
        area.y.saturating_add(header_height),
        area.width,
        area.height.saturating_sub(header_height),
    );
    let paragraph = Paragraph::new(lines(app, grid.width)).wrap(Wrap { trim: false });
    let blocks = app.session_blocks();
    let reveal_row = if app.timetable_navigation.reveal {
        blocks
            .get(app.timetable_navigation.block)
            .map(|block| 3 + block.start / 30)
    } else {
        None
    };
    let viewport = &mut app.timetable_viewport;
    viewport.height = grid.height;
    viewport.max_offset = (paragraph.line_count(grid.width).min(u16::MAX as usize) as u16)
        .saturating_sub(grid.height);
    viewport.offset = viewport.offset.min(viewport.max_offset);
    if let Some(row) = reveal_row {
        if row < viewport.offset {
            viewport.offset = row;
        } else if row >= viewport.offset.saturating_add(grid.height) {
            viewport.offset = row.saturating_add(1).saturating_sub(grid.height);
        }
        viewport.offset = viewport.offset.min(viewport.max_offset);
        app.timetable_navigation.reveal = false;
    }
    let title = if active { "> Timetable" } else { "Timetable" };
    let count = app.solution.as_ref().map_or(0, |s| {
        if s.status == SolveStatus::OptimalKnown {
            s.alternatives.len().max(1)
        } else {
            0
        }
    });
    let mode = match app.timetable_navigation.level {
        Level::Timetables => "Enter sessions",
        Level::Blocks => "Sessions: Enter options",
        Level::Members => "Options: h/l switch",
    };
    let selection = app
        .solution
        .as_ref()
        .and_then(|s| s.choices.get(app.result_cursor))
        .map(|c| {
            let member = super::current_member(c, &app.actual_members);
            format!(
                " · {} {} ({}/{})",
                c.requirement_id,
                member.map_or("", |m| m.label.as_str()),
                member
                    .and_then(|m| c.members.iter().position(|option| option.id == m.id))
                    .map_or(0, |i| i + 1),
                c.members.len()
            )
        })
        .unwrap_or_default();
    frame.render_widget(
        Line::styled(
            format!(
                "{title} {}/{} · {mode}{selection} · 30-minute rows · t focus · PgUp/Dn | {}/{}",
                if count == 0 {
                    0
                } else {
                    app.timetable_navigation.alternative + 1
                },
                count,
                viewport.offset,
                viewport.max_offset
            ),
            if active {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            },
        ),
        header,
    );
    frame.render_widget(paragraph.scroll((viewport.offset, 0)), grid);
}

use crossterm::event::KeyCode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Level {
    #[default]
    Timetables,
    Blocks,
    Members,
}

#[derive(Debug, Default)]
pub(super) struct Navigation {
    level: Level,
    block: usize,
    alternative: usize,
    reveal: bool,
}

impl Navigation {
    pub(super) fn hint(&self) -> &'static str {
        match self.level {
            Level::Timetables => {
                "h/l ←/→ optimal timetables | Enter sessions | Tab panes | PgUp/Dn scroll"
            }
            Level::Blocks => {
                "hjkl / arrows session blocks | Enter same-time options | Esc back | Tab panes"
            }
            Level::Members => "h/l ←/→ same-time session | Enter/Esc back | Tab panes",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SessionBlock {
    choice: usize,
    day: u8,
    start: u16,
    end: u16,
}

impl AppState {
    fn session_blocks(&self) -> Vec<SessionBlock> {
        let mut blocks = Vec::new();
        if let Some(solution) = &self.solution
            && solution.status == SolveStatus::OptimalKnown
        {
            for (choice, section) in solution.choices.iter().enumerate() {
                for meeting in &section.meetings {
                    if meeting.weekday < 5
                        && meeting.start_minute < meeting.end_minute
                        && meeting.end_minute <= 1440
                    {
                        blocks.push(SessionBlock {
                            choice,
                            day: meeting.weekday,
                            start: meeting.start_minute,
                            end: meeting.end_minute,
                        });
                    }
                }
            }
        }
        blocks.sort_by_key(|b| (b.day, b.start, b.end, b.choice));
        blocks
    }

    fn select_timetable_block(&mut self, index: usize) {
        if let Some(block) = self.session_blocks().get(index) {
            self.result_cursor = block.choice;
            self.timetable_navigation.block = index;
            self.timetable_navigation.reveal = true;
        }
    }

    pub(super) fn handle_timetable_key(&mut self, key: KeyCode) -> bool {
        let level = self.timetable_navigation.level;
        match key {
            KeyCode::Enter => {
                match level {
                    Level::Timetables => {
                        if !self.session_blocks().is_empty() {
                            self.timetable_navigation.level = Level::Blocks;
                            self.select_timetable_block(0);
                        }
                    }
                    Level::Blocks => self.timetable_navigation.level = Level::Members,
                    Level::Members => self.timetable_navigation.level = Level::Blocks,
                }
                true
            }
            KeyCode::Esc if level != Level::Timetables => {
                self.timetable_navigation.level = if level == Level::Members {
                    Level::Blocks
                } else {
                    Level::Timetables
                };
                true
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'l') => {
                let delta = if matches!(key, KeyCode::Left | KeyCode::Char('h')) {
                    -1
                } else {
                    1
                };
                match level {
                    Level::Timetables => self.cycle_timetable(delta),
                    Level::Blocks => self.move_timetable_block(delta, true),
                    Level::Members => self.cycle_current_member(delta),
                }
                true
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k')
                if level != Level::Timetables =>
            {
                if level == Level::Blocks {
                    let delta = if matches!(key, KeyCode::Up | KeyCode::Char('k')) {
                        -1
                    } else {
                        1
                    };
                    self.move_timetable_block(delta, false);
                }
                true
            }
            _ => false,
        }
    }

    fn move_timetable_block(&mut self, delta: isize, horizontal: bool) {
        let blocks = self.session_blocks();
        let Some(current) = blocks.get(self.timetable_navigation.block) else {
            return;
        };
        let next = blocks
            .iter()
            .enumerate()
            .filter(|(i, b)| {
                if horizontal {
                    (i16::from(b.day) - i16::from(current.day)) * delta as i16 > 0
                } else {
                    b.day == current.day
                        && (*i as isize - self.timetable_navigation.block as isize) * delta > 0
                }
            })
            .min_by_key(|(i, b)| {
                if horizontal {
                    (
                        u32::from(b.day.abs_diff(current.day)),
                        u32::from(b.start.abs_diff(current.start)),
                        *i,
                    )
                } else {
                    (i.abs_diff(self.timetable_navigation.block) as u32, 0, *i)
                }
            })
            .map(|(i, _)| i);
        if let Some(next) = next {
            self.select_timetable_block(next);
        }
    }

    fn cycle_timetable(&mut self, delta: isize) {
        let Some(solution) = &mut self.solution else {
            return;
        };
        let count = solution.alternatives.len();
        if count <= 1 {
            return;
        }
        let index = super::moved_index(self.timetable_navigation.alternative, count, delta);
        solution.choices = solution.alternatives[index].clone();
        self.timetable_navigation.alternative = index;
        self.timetable_navigation.block = 0;
        self.result_cursor = 0;
        self.actual_members.clear();
        self.export = None;
        self.timetable_viewport.offset = 0;
        self.status = format!(
            "Equally optimal timetable {}/{}{}.",
            index + 1,
            count,
            if solution.alternatives_truncated {
                " (more exist)"
            } else {
                ""
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ManualStore;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn state() -> AppState {
        let dataset = crate::adapter::parse_catalog(
            include_str!("../../tests/fixtures/catalog.json"),
            include_str!("../../tests/fixtures/term.json"),
        )
        .unwrap();
        let selected = vec!["A".into(), "B".into()];
        let solution = app::optimize(&dataset, &selected, None).unwrap();
        let mut state = AppState::new(
            dataset,
            ManualStore::default(),
            ".".into(),
            "unused.ics".into(),
            selected,
        )
        .unwrap();
        state.install_solution(solution);
        state.focus = Focus::Timetable;
        state
    }

    fn press(app: &mut AppState, key: KeyCode) {
        app.handle_key(KeyEvent::new(key, KeyModifiers::NONE))
            .unwrap();
    }

    #[test]
    fn nested_keyboard_flow_switches_members_without_changing_time() {
        for (left, right) in [
            (KeyCode::Char('h'), KeyCode::Char('l')),
            (KeyCode::Left, KeyCode::Right),
        ] {
            let mut app = state();
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.timetable_navigation.level, Level::Blocks);
            let index = app
                .session_blocks()
                .iter()
                .position(|b| {
                    app.solution.as_ref().unwrap().choices[b.choice]
                        .members
                        .len()
                        > 1
                })
                .unwrap();
            app.select_timetable_block(index);
            let choice = app.solution.as_ref().unwrap().choices[app.result_cursor].clone();
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.timetable_navigation.level, Level::Members);
            press(&mut app, right);
            assert_eq!(
                app.actual_members[&choice.requirement_id],
                choice.members[1].id
            );
            assert_eq!(
                app.solution.as_ref().unwrap().choices[app.result_cursor].meetings,
                choice.meetings
            );
            press(&mut app, left);
            assert_eq!(
                app.actual_members[&choice.requirement_id],
                choice.members[0].id
            );
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.timetable_navigation.level, Level::Blocks);
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.timetable_navigation.level, Level::Timetables);
            assert!(!app.should_quit);
            press(&mut app, KeyCode::Tab);
            assert_ne!(app.focus, Focus::Timetable);
        }
    }

    #[test]
    fn spatial_arrows_and_vim_keys_select_and_reveal_individual_blocks() {
        let mut app = state();
        app.solution.as_mut().unwrap().choices.truncate(1);
        let choice = &mut app.solution.as_mut().unwrap().choices[0];
        let mut meeting = choice.meetings[0].clone();
        meeting.weekday = 0;
        meeting.start_minute = 480;
        meeting.end_minute = 540;
        choice.meetings = vec![meeting.clone()];
        meeting.start_minute = 1080;
        meeting.end_minute = 1140;
        choice.meetings.push(meeting.clone());
        meeting.weekday = 2;
        choice.meetings.push(meeting);
        // Renderer uses members, which must retain the same fixed meetings.
        for choice in &mut app.solution.as_mut().unwrap().choices {
            for member in &mut choice.members {
                member.meetings = choice.meetings.clone();
            }
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.session_blocks()[app.timetable_navigation.block].start,
            480
        );
        press(&mut app, KeyCode::Char('j'));
        let later = app.session_blocks()[app.timetable_navigation.block];
        assert_eq!((later.day, later.start), (0, 1080));
        let mut terminal = Terminal::new(TestBackend::new(100, 8)).unwrap();
        terminal.draw(|f| draw(f, &mut app, f.area())).unwrap();
        assert!(app.timetable_viewport.offset > 0);
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            screen.contains("18:00"),
            "selected session must be visible: {screen}"
        );
        press(&mut app, KeyCode::Up);
        assert_eq!(
            app.session_blocks()[app.timetable_navigation.block].start,
            480
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        assert!(app.session_blocks()[app.timetable_navigation.block].day > 0);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.session_blocks()[app.timetable_navigation.block].day, 0);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(
            app.session_blocks()[app.timetable_navigation.block].start,
            480
        );
    }

    #[test]
    fn timetable_alternatives_wrap_and_reset_members() {
        let mut app = state();
        let solution = app.solution.as_mut().unwrap();
        let first = solution.choices.clone();
        let mut second = first.clone();
        second.reverse();
        solution.alternatives = vec![first.clone(), second.clone()];
        app.actual_members.insert("stale".into(), "stale".into());
        press(&mut app, KeyCode::Right);
        assert_eq!(app.timetable_navigation.alternative, 1);
        assert_eq!(app.solution.as_ref().unwrap().choices[0].id, second[0].id);
        assert!(app.actual_members.is_empty());
        assert_eq!(app.focus, Focus::Timetable);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.timetable_navigation.alternative, 0);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.timetable_navigation.alternative, 1);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.timetable_navigation.alternative, 0);
        app.invalidate("changed");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.timetable_navigation.level, Level::Timetables);
        press(&mut app, KeyCode::Right);
        assert!(app.solution.is_none());
    }
}
