//! Independent edge-to-edge timetable. Results scrolling never owns this viewport.
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
                Ok(sections) => week::week_lines(
                    &sections,
                    width,
                    solution
                        .choices
                        .get(app.result_cursor)
                        .map(|choice| choice.requirement_id.as_str()),
                ),
                Err(error) => vec![Line::styled(
                    format!("Cannot render timetable: {error:#}"),
                    Style::default().fg(Color::Red),
                )],
            };
        }
        return vec![Line::from(format!(
            "No feasible timetable: {:?}.",
            solution.status
        ))];
    }
    vec![Line::from(if app.optimize_running {
        "Optimizing. The timetable will appear here."
    } else {
        "No timetable yet. Select classes and press o."
    })]
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
    let viewport = &mut app.timetable_viewport;
    viewport.height = grid.height;
    viewport.max_offset = (paragraph.line_count(grid.width).min(u16::MAX as usize) as u16)
        .saturating_sub(grid.height);
    viewport.offset = viewport.offset.min(viewport.max_offset);
    let title = if active { "> Timetable" } else { "Timetable" };
    frame.render_widget(
        Line::styled(
            format!(
                "{title} · 30-minute rows · t focus · PgUp/Dn | {}/{}",
                viewport.offset, viewport.max_offset
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
