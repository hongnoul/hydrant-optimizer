//! Independent full-width timetable pane. Results scrolling never owns this viewport.
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        })
        .title(if active { "> Timetable" } else { "Timetable" });
    let inner = block.inner(area);
    let paragraph = Paragraph::new(lines(app, inner.width)).wrap(Wrap { trim: false });
    let viewport = &mut app.timetable_viewport;
    viewport.height = inner.height;
    viewport.max_offset = (paragraph.line_count(inner.width).min(u16::MAX as usize) as u16)
        .saturating_sub(inner.height);
    viewport.offset = viewport.offset.min(viewport.max_offset);
    let block = block.title_bottom(format!(
        " t focus · PgUp/Dn | {}/{} ",
        viewport.offset, viewport.max_offset
    ));
    frame.render_widget(paragraph.block(block).scroll((viewport.offset, 0)), area);
}
