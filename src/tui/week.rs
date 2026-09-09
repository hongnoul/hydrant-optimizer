use std::collections::BTreeSet;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::model::ChosenSection;

const DAYS: [&str; 5] = ["Mon", "Tue", "Wed", "Thu", "Fri"];
const SLOT_MINUTES: u16 = 30;
const DAY_COUNT: usize = 5;
const WEEKEND_START: u8 = DAY_COUNT as u8;
const WEEK_DAYS: u8 = 7;
const TIME_WIDTH: usize = 5;
const MIN_DAY_WIDTH: usize = 5;
const SEPARATOR_COUNT: usize = DAY_COUNT + 2;
const PALETTE: [Color; 8] = [
    Color::Blue,
    Color::Green,
    Color::Magenta,
    Color::Cyan,
    Color::LightBlue,
    Color::LightGreen,
    Color::LightMagenta,
    Color::LightCyan,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ColumnPlan {
    table_width: usize,
    time_width: usize,
    day_widths: [usize; DAY_COUNT],
}

#[derive(Clone, Debug)]
struct Entry {
    day: usize,
    start: u16,
    end: u16,
    course_id: String,
    kind: String,
    section_id: String,
    selected: bool,
    color: Color,
}

#[derive(Clone, Debug, Default)]
struct EntrySet {
    weekdays: Vec<Entry>,
    weekend_count: usize,
}

pub(super) fn week_lines(
    sections: &[ChosenSection],
    width: u16,
    selected_requirement: Option<&str>,
) -> Vec<Line<'static>> {
    let width = width as usize;
    if width == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let entry_set = entries_from_sections(sections, selected_requirement);
    if entry_set.weekdays.is_empty() {
        if entry_set.weekend_count > 0 {
            lines.push(plain_line(
                "Weekday view: no Monday-Friday meetings to show.",
                width,
            ));
            lines.push(plain_line(
                &weekend_disclosure(entry_set.weekend_count),
                width,
            ));
            return lines;
        }
        lines.push(plain_line("No weekly meetings to show.", width));
        return lines;
    }
    let entries = entry_set.weekdays;

    let Some(plan) = ColumnPlan::new(width) else {
        lines.push(plain_line(
            "Width too small for the weekday table. Exact meeting times appear in Results.",
            width,
        ));
        if entry_set.weekend_count > 0 {
            lines.push(plain_line(
                &weekend_disclosure(entry_set.weekend_count),
                width,
            ));
        }
        return lines;
    };

    let min_start = entries.iter().map(|entry| entry.start).min().unwrap_or(0);
    let max_end = entries.iter().map(|entry| entry.end).max().unwrap_or(0);
    let first_slot = (min_start / SLOT_MINUTES) * SLOT_MINUTES;
    let last_slot = max_end.div_ceil(SLOT_MINUTES) * SLOT_MINUTES;

    lines.push(border_line(&plan, '┌', '┬', '┐'));
    lines.push(header_line(&plan));
    lines.push(border_line(&plan, '├', '┼', '┤'));

    let mut slot = first_slot;
    while slot < last_slot {
        lines.push(row_line(&plan, &entries, slot));
        slot = slot.saturating_add(SLOT_MINUTES);
    }
    lines.push(border_line(&plan, '└', '┴', '┘'));

    if entry_set.weekend_count > 0 {
        lines.push(plain_line(
            &weekend_disclosure(entry_set.weekend_count),
            width,
        ));
    }

    lines
}

impl ColumnPlan {
    fn new(width: usize) -> Option<Self> {
        if width < TIME_WIDTH + SEPARATOR_COUNT + DAY_COUNT * MIN_DAY_WIDTH {
            return None;
        }
        let table_width = width;
        let available = table_width - TIME_WIDTH - SEPARATOR_COUNT;
        let base = available / DAY_COUNT;
        if base < MIN_DAY_WIDTH {
            return None;
        }
        let mut day_widths = [base; DAY_COUNT];
        for width in day_widths.iter_mut().take(available % DAY_COUNT) {
            *width += 1;
        }
        Some(Self {
            table_width,
            time_width: TIME_WIDTH,
            day_widths,
        })
    }
}

fn entries_from_sections(
    sections: &[ChosenSection],
    selected_requirement: Option<&str>,
) -> EntrySet {
    let selected_requirement = selected_requirement
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut entry_set = EntrySet::default();
    for section in sections {
        let color = course_color(&section.course_id);
        let selected = selected_requirement
            .map(|requirement| section_matches_requirement(section, requirement))
            .unwrap_or(false);
        for meeting in &section.section.meetings {
            if meeting.weekday >= WEEK_DAYS
                || meeting.start_minute >= meeting.end_minute
                || meeting.end_minute > 24 * 60
            {
                continue;
            }
            if meeting.weekday >= WEEKEND_START {
                entry_set.weekend_count += 1;
                continue;
            }
            entry_set.weekdays.push(Entry {
                day: meeting.weekday as usize,
                start: meeting.start_minute,
                end: meeting.end_minute,
                course_id: section.course_id.clone(),
                kind: component_label(&section.kind),
                section_id: section.section.id.clone(),
                selected,
                color,
            });
        }
    }
    entry_set.weekdays.sort_by(|left, right| {
        (
            left.day,
            left.start,
            left.end,
            &left.course_id,
            &left.kind,
            &left.section_id,
        )
            .cmp(&(
                right.day,
                right.start,
                right.end,
                &right.course_id,
                &right.kind,
                &right.section_id,
            ))
    });
    entry_set
}

fn section_matches_requirement(section: &ChosenSection, requirement: &str) -> bool {
    requirement == format!("{}/{}", section.course_id, section.kind)
}

fn row_line(plan: &ColumnPlan, entries: &[Entry], slot_start: u16) -> Line<'static> {
    let slot_end = slot_start.saturating_add(SLOT_MINUTES).min(24 * 60);
    let mut spans = Vec::with_capacity(DAY_COUNT * 2 + 3);
    spans.push(Span::raw("│"));
    spans.push(Span::raw(pad_cell(
        &format_time(slot_start),
        plan.time_width,
    )));
    spans.push(Span::raw("│"));

    for day in 0..DAY_COUNT {
        let occupants: Vec<&Entry> = entries
            .iter()
            .filter(|entry| {
                entry.day == day && intersects(entry.start, entry.end, slot_start, slot_end)
            })
            .collect();
        let width = plan.day_widths[day];
        if occupants.is_empty() {
            spans.push(Span::raw(pad_cell("·", width)));
        } else {
            let text = occupant_text(&occupants, width);
            spans.push(Span::styled(
                pad_cell(&text, width),
                occupant_style(&occupants),
            ));
        }
        spans.push(Span::raw("│"));
    }

    debug_assert_eq!(line_width(&spans), plan.table_width);
    Line::from(spans)
}

fn header_line(plan: &ColumnPlan) -> Line<'static> {
    let mut spans = Vec::with_capacity(DAY_COUNT * 2 + 3);
    spans.push(Span::raw("│"));
    spans.push(Span::raw(pad_cell("Time", plan.time_width)));
    spans.push(Span::raw("│"));
    for (day, width) in DAYS.iter().zip(plan.day_widths) {
        spans.push(Span::raw(pad_cell(day, width)));
        spans.push(Span::raw("│"));
    }
    debug_assert_eq!(line_width(&spans), plan.table_width);
    Line::from(spans)
}

fn border_line(plan: &ColumnPlan, left: char, middle: char, right: char) -> Line<'static> {
    let mut line = String::new();
    line.push(left);
    line.push_str(&"─".repeat(plan.time_width));
    line.push(middle);
    for (index, width) in plan.day_widths.iter().enumerate() {
        line.push_str(&"─".repeat(*width));
        line.push(if index + 1 == DAY_COUNT {
            right
        } else {
            middle
        });
    }
    debug_assert_eq!(Line::from(line.as_str()).width(), plan.table_width);
    Line::from(line)
}

fn occupant_text(occupants: &[&Entry], width: usize) -> String {
    if occupants.len() == 1 {
        return fit_component_text("", &occupants[0].course_id, &occupants[0].kind, width);
    }

    let courses = occupants
        .iter()
        .map(|entry| entry.course_id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let kinds = occupants
        .iter()
        .map(|entry| entry.kind.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let prefix = format!("{}×", occupants.len());
    if width <= prefix.chars().count() {
        return fit_text(&occupants.len().to_string(), width);
    }
    let kind_text = kinds.join("/");
    if courses.len() == 1 {
        return fit_component_text(&prefix, courses[0], &kind_text, width);
    }

    let paired_text = occupants
        .iter()
        .map(|entry| format!("{} {}", entry.course_id, entry.kind))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("/");
    let paired = format!("{prefix}{paired_text}");
    if Span::raw(paired.as_str()).width() <= width {
        paired
    } else {
        fit_text(&format!("{prefix}{kind_text}"), width)
    }
}

fn component_label(kind: &str) -> String {
    let trimmed = kind.trim();
    let normalized = trimmed.to_ascii_lowercase();
    let collapsed = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    match collapsed.as_str() {
        "lecture" | "lec" => "Lec".to_string(),
        "recitation" | "rec" => "Rec".to_string(),
        "lab" | "laboratory" => "Lab".to_string(),
        "pe" | "physicaleducation" => "PE".to_string(),
        "design" => "Design".to_string(),
        "" => "Other".to_string(),
        _ => {
            let mut chars = trimmed.chars().filter(|ch| !ch.is_whitespace());
            let label = chars.by_ref().take(3).collect::<String>();
            if label.is_empty() {
                "Other".to_string()
            } else {
                titlecase_ascii(&label)
            }
        }
    }
}

fn titlecase_ascii(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    if let Some(first) = chars.next() {
        output.extend(first.to_uppercase());
    }
    for ch in chars {
        output.extend(ch.to_lowercase());
    }
    output
}

fn fit_component_text(prefix: &str, subject: &str, component: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let suffix = if component.is_empty() {
        String::new()
    } else {
        format!(" {component}")
    };
    let fixed = format!("{prefix}{suffix}");
    if Span::raw(fixed.as_str()).width() >= width {
        if prefix.is_empty() {
            return fit_text(component, width);
        }
        let prefix_width = Span::raw(prefix).width();
        if prefix_width >= width {
            return fit_text(prefix, width);
        }
        return format!("{prefix}{}", fit_text(component, width - prefix_width));
    }

    let subject_width = width - Span::raw(fixed.as_str()).width();
    let fitted_subject = fit_text(subject, subject_width);
    format!("{prefix}{fitted_subject}{suffix}")
}

fn weekend_disclosure(count: usize) -> String {
    let meeting = if count == 1 { "meeting" } else { "meetings" };
    format!("{count} weekend {meeting} hidden here. See Results. Export keeps them.")
}

fn occupant_style(occupants: &[&Entry]) -> Style {
    if occupants.iter().any(|entry| entry.selected) {
        return Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
    }
    if occupants.len() == 1 {
        return Style::default()
            .fg(readable_fg(occupants[0].color))
            .bg(occupants[0].color)
            .add_modifier(Modifier::BOLD);
    }
    Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

fn readable_fg(background: Color) -> Color {
    match background {
        Color::LightBlue | Color::LightGreen | Color::LightMagenta | Color::LightCyan => {
            Color::Black
        }
        _ => Color::White,
    }
}

fn intersects(start: u16, end: u16, slot_start: u16, slot_end: u16) -> bool {
    start < slot_end && end > slot_start
}

fn course_color(course_id: &str) -> Color {
    PALETTE[stable_hash(course_id) as usize % PALETTE.len()]
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn format_time(minutes: u16) -> String {
    if minutes >= 24 * 60 {
        "24:00".to_string()
    } else {
        format!("{:02}:{:02}", minutes / 60, minutes % 60)
    }
}

fn plain_line(text: &str, width: usize) -> Line<'static> {
    Line::from(fit_text(text, width))
}

fn pad_cell(text: &str, width: usize) -> String {
    let fitted = fit_text(text, width);
    let padding = width.saturating_sub(Span::raw(fitted.as_str()).width());
    format!("{fitted}{}", " ".repeat(padding))
}

fn fit_text(text: &str, width: usize) -> String {
    let mut output = String::new();
    for ch in text.chars() {
        let mut candidate = output.clone();
        candidate.push(ch);
        if Span::raw(candidate.as_str()).width() > width {
            break;
        }
        output = candidate;
    }
    output
}

fn line_width(spans: &[Span<'_>]) -> usize {
    Line::from(spans.to_vec()).width()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Meeting, SectionOption, Source};
    use ratatui::style::Color;

    fn section(course_id: &str, kind: &str, id: &str, meetings: Vec<Meeting>) -> ChosenSection {
        ChosenSection {
            course_id: course_id.to_string(),
            course_title: format!("{course_id} title"),
            kind: kind.to_string(),
            section: SectionOption {
                id: id.to_string(),
                label: format!("{id} label"),
                room: "room".to_string(),
                source: Source::Manual,
                meetings,
                incompatible_with: Default::default(),
                unsupported_reason: None,
            },
        }
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

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn all_text(lines: &[Line<'_>]) -> String {
        lines.iter().map(text).collect::<Vec<_>>().join("\n")
    }

    fn row_text(lines: &[Line<'_>], time: &str) -> String {
        lines
            .iter()
            .map(text)
            .find(|line| line.starts_with(&format!("│{time}")))
            .unwrap_or_else(|| panic!("missing row {time}"))
    }

    fn row_index(lines: &[Line<'_>], time: &str) -> usize {
        lines
            .iter()
            .map(text)
            .position(|line| line.starts_with(&format!("│{time}")))
            .unwrap_or_else(|| panic!("missing row {time}"))
    }

    fn cell_text(row: &str, index: usize) -> &str {
        row.split('│').nth(index + 2).unwrap()
    }

    #[test]
    fn header_includes_weekday_columns_only() {
        let lines = week_lines(
            &[section(
                "6.1200",
                "lecture",
                "L1",
                vec![meeting(0, 540, 600)],
            )],
            78,
            None,
        );
        let rendered = all_text(&lines);
        assert!(text(&lines[0]).starts_with('┌'));
        for day in DAYS {
            assert!(rendered.contains(day), "missing {day}");
        }
        assert!(!rendered.contains("Sat"));
        assert!(!rendered.contains("Sun"));
        let header = text(
            lines
                .iter()
                .find(|line| text(line).starts_with("│Time"))
                .unwrap(),
        );
        assert_eq!(header.matches('│').count(), DAY_COUNT + 2);
        for line in &lines {
            assert!(line.width() <= 78);
        }
    }

    #[test]
    fn rows_are_spaced_every_thirty_minutes() {
        let lines = week_lines(
            &[section(
                "6.1200",
                "lecture",
                "L1",
                vec![meeting(0, 555, 646)],
            )],
            78,
            None,
        );
        assert!(all_text(&lines).contains("09:00"));
        assert!(all_text(&lines).contains("09:30"));
        assert!(all_text(&lines).contains("10:00"));
        assert!(all_text(&lines).contains("10:30"));
        assert_eq!(row_index(&lines, "09:30"), row_index(&lines, "09:00") + 1);
        assert_eq!(row_index(&lines, "10:00"), row_index(&lines, "09:30") + 1);
        assert_eq!(row_index(&lines, "10:30"), row_index(&lines, "10:00") + 1);
        assert!(!all_text(&lines).contains("09:15-10:46"));
    }

    #[test]
    fn occupancy_appears_on_correct_day_and_rows() {
        let lines = week_lines(
            &[section(
                "6.1200",
                "lecture",
                "L1",
                vec![meeting(2, 600, 660)],
            )],
            78,
            None,
        );
        let row_1000 = row_text(&lines, "10:00");
        let row_1030 = row_text(&lines, "10:30");
        assert_eq!(cell_text(&row_1000, 0).trim(), "·");
        assert!(cell_text(&row_1000, 2).contains("6.1200"));
        assert!(cell_text(&row_1030, 2).contains("6.1200"));
    }

    #[test]
    fn adjacency_and_end_boundaries_do_not_create_false_overlap() {
        let lines = week_lines(
            &[
                section("6.1200", "lecture", "L1", vec![meeting(0, 540, 570)]),
                section("18.06", "lecture", "L2", vec![meeting(0, 570, 600)]),
            ],
            78,
            None,
        );
        let row_0900 = row_text(&lines, "09:00");
        let row_0930 = row_text(&lines, "09:30");
        assert!(cell_text(&row_0900, 0).contains("6.1200"));
        assert!(!cell_text(&row_0900, 0).contains('×'));
        assert!(cell_text(&row_0930, 0).contains("18.06"));
        assert!(!cell_text(&row_0930, 0).contains('×'));
    }

    #[test]
    fn exact_minute_intersection_fills_each_intersecting_slot() {
        let lines = week_lines(
            &[section("7.012", "lab", "B1", vec![meeting(1, 541, 599)])],
            78,
            None,
        );
        assert!(cell_text(&row_text(&lines, "09:00"), 1).contains("7.012"));
        assert!(cell_text(&row_text(&lines, "09:30"), 1).contains("7.012"));
        assert!(!all_text(&lines).contains("09:01"));
        assert!(!all_text(&lines).contains("09:59"));
    }

    #[test]
    fn multiple_meetings_in_one_bucket_keep_indicator_without_legend() {
        let lines = week_lines(
            &[
                section("6.1200", "lecture", "L1", vec![meeting(0, 540, 555)]),
                section("18.06", "lecture", "L2", vec![meeting(0, 555, 570)]),
            ],
            78,
            None,
        );
        let row = row_text(&lines, "09:00");
        assert!(cell_text(&row, 0).contains("2×"));
        assert!(!all_text(&lines).contains("Legend:"));
        assert!(text(lines.last().unwrap()).starts_with('└'));
    }

    #[test]
    fn grid_has_no_caption_or_legend_even_with_selected_and_shortened_components() {
        let lines = week_lines(
            &[
                section("2.00B", "design", "D1", vec![meeting(4, 540, 570)]),
                section("21W.755", "seminar", "S1", vec![meeting(0, 540, 570)]),
            ],
            78,
            Some("2.00B/design"),
        );
        assert_eq!(
            lines.len(),
            5,
            "only table borders, header, and one time row"
        );
        assert!(text(&lines[0]).starts_with('┌'));
        assert!(text(lines.last().unwrap()).starts_with('└'));
        for removed in [
            "Timetable",
            "Legend:",
            "Lec=",
            "highlighted",
            "first 3 chars",
        ] {
            assert!(!all_text(&lines).contains(removed));
        }
        assert!(lines.iter().all(|line| line.width() == 78));
    }

    #[test]
    fn component_labels_cover_known_design_and_other_kinds() {
        let lines = week_lines(
            &[
                section("6.1200", "lecture", "L1", vec![meeting(0, 540, 570)]),
                section("18.01", "recitation", "R1", vec![meeting(1, 540, 570)]),
                section("7.01", "lab", "B1", vec![meeting(2, 540, 570)]),
                section(
                    "PE.XXXX",
                    "physical education",
                    "P1",
                    vec![meeting(3, 540, 570)],
                ),
                section("2.00B", "design", "D1", vec![meeting(4, 540, 570)]),
                section("21W.755", "seminar", "S1", vec![meeting(0, 570, 600)]),
            ],
            78,
            None,
        );

        let row_0900 = row_text(&lines, "09:00");
        assert!(cell_text(&row_0900, 0).contains("6.1200 Lec"));
        assert!(cell_text(&row_0900, 1).contains("18.01 Rec"));
        assert!(cell_text(&row_0900, 2).contains("7.01 Lab"));
        assert!(cell_text(&row_0900, 3).contains("PE.XXXX PE"));
        assert!(cell_text(&row_0900, 4).contains("2.00B Design"));

        let row_0930 = row_text(&lines, "09:30");
        assert!(cell_text(&row_0930, 0).contains("21W.755 Sem"));
    }

    #[test]
    fn clipping_truncates_subject_before_component_at_inner_78() {
        let lines = week_lines(
            &[section(
                "LONG-SUBJECT-6.1200",
                "lecture",
                "L1",
                vec![meeting(0, 540, 570)],
            )],
            78,
            None,
        );
        let cell = cell_text(&row_text(&lines, "09:00"), 0).trim().to_string();
        assert!(
            cell.ends_with(" Lec"),
            "component was clipped from {cell:?}"
        );
        assert!(cell.starts_with("LONG-SUB"));
        assert!(!cell.contains("6.1200"));
    }

    #[test]
    fn shared_bucket_with_same_course_different_components_shows_multiplicity() {
        let lines = week_lines(
            &[
                section("6.1200", "lecture", "L1", vec![meeting(0, 540, 570)]),
                section("6.1200", "recitation", "R1", vec![meeting(0, 540, 570)]),
            ],
            78,
            None,
        );
        let row = row_text(&lines, "09:00");
        let cell = cell_text(&row, 0);
        assert!(cell.contains("2×"));
        assert!(cell.contains("Lec/Rec"));
        assert!(cell.contains("6.1"));
    }

    #[test]
    fn shared_bucket_keeps_different_course_component_pairs_when_they_fit() {
        let lines = week_lines(
            &[
                section("W", "lecture", "L1", vec![meeting(0, 540, 570)]),
                section("X", "lab", "B1", vec![meeting(0, 540, 570)]),
            ],
            78,
            None,
        );
        let row = row_text(&lines, "09:00");
        let cell = cell_text(&row, 0).trim();
        assert_eq!(cell, "2×W Lec/X Lab");
    }

    #[test]
    fn shared_bucket_summarizes_components_instead_of_mismatching_long_pairs() {
        let lines = week_lines(
            &[
                section("6.1200", "lecture", "L1", vec![meeting(0, 540, 570)]),
                section("18.01", "lab", "B1", vec![meeting(0, 540, 570)]),
            ],
            78,
            None,
        );
        let row = row_text(&lines, "09:00");
        let cell = cell_text(&row, 0);
        assert!(cell.contains("2×Lab/Lec"));
        assert!(!cell.contains("6.1200"));
        assert!(!cell.contains("18.01"));
    }

    #[test]
    fn weekend_meetings_are_disclosed_but_do_not_set_weekday_bounds() {
        let lines = week_lines(
            &[
                section("6.1200", "lecture", "L1", vec![meeting(0, 540, 570)]),
                section("21W.755", "seminar", "S1", vec![meeting(6, 1410, 1440)]),
            ],
            78,
            None,
        );
        let rendered = all_text(&lines);
        assert!(
            rendered.contains("1 weekend meeting hidden here. See Results. Export keeps them.")
        );
        assert!(rendered.contains("09:00"));
        assert!(!rendered.contains("23:30"));
        assert!(!rendered.contains("Sat"));
        assert!(!rendered.contains("Sun"));
    }

    #[test]
    fn only_weekend_input_shows_disclosure_without_time_rows() {
        let lines = week_lines(
            &[
                section("21W.755", "seminar", "S1", vec![meeting(5, 600, 660)]),
                section("CMS.100", "lecture", "L1", vec![meeting(6, 1410, 1440)]),
            ],
            78,
            None,
        );
        let rendered = all_text(&lines);
        assert!(rendered.contains("no Monday-Friday meetings"));
        assert!(
            rendered.contains("2 weekend meetings hidden here. See Results. Export keeps them.")
        );
        assert!(!rendered.contains("│10:00"));
        assert!(!rendered.contains("│23:30"));
        assert!(!rendered.contains("┌"));
        assert!(!rendered.contains("Lec="));
    }

    #[test]
    fn selected_requirement_is_highlighted() {
        let lines = week_lines(
            &[
                section("6.1200", "lecture", "L1", vec![meeting(0, 540, 570)]),
                section("18.06", "lecture", "L2", vec![meeting(1, 540, 570)]),
            ],
            78,
            Some("6.1200/lecture"),
        );
        let highlighted = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content.contains("6.1200"))
            .unwrap();
        assert_eq!(highlighted.style.bg, Some(Color::Yellow));
    }

    #[test]
    fn width_below_thirty_seven_shows_resize_hint_not_ambiguous_days() {
        let lines = week_lines(
            &[section(
                "6.1200",
                "lecture",
                "L1",
                vec![meeting(0, 540, 570)],
            )],
            36,
            None,
        );
        let rendered = all_text(&lines);
        assert!(rendered.contains("Width too small"));
        assert!(lines.iter().all(|line| line.width() <= 36));

        let lines = week_lines(
            &[section(
                "6.1200",
                "lecture",
                "L1",
                vec![meeting(0, 540, 570)],
            )],
            37,
            None,
        );
        let rendered = all_text(&lines);
        for day in DAYS {
            assert!(rendered.contains(day), "missing {day}");
        }
        assert!(lines.iter().all(|line| line.width() <= 37));
    }

    #[test]
    fn selected_style_and_unicode_width_are_preserved() {
        let mut item = section("界6.1200", "lecture", "L1", vec![meeting(0, 540, 570)]);
        item.section.label = "界 label".to_string();
        let lines = week_lines(&[item.clone()], 78, Some("界6.1200/lecture"));
        let highlighted = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content.contains("Lec"))
            .unwrap();
        assert_eq!(highlighted.style.bg, Some(Color::Yellow));
        assert!(highlighted.style.add_modifier.contains(Modifier::BOLD));
        assert!(highlighted.content.contains("界6.1200 Lec"));

        for width in [1, 10, 20, 26] {
            let lines = week_lines(&[item.clone()], width, None);
            assert!(!lines.is_empty());
            for line in &lines {
                assert!(line.width() <= width as usize);
            }
        }
    }

    #[test]
    fn combining_marks_emoji_and_zwj_are_measured_by_ratatui_width() {
        let item = section(
            "e\u{301}🇺🇸👨\u{200d}👩\u{200d}👧\u{200d}👦6.1200",
            "lecture",
            "L1",
            vec![meeting(0, 540, 570)],
        );
        let lines = week_lines(&[item], 30, None);
        assert!(lines.iter().all(|line| line.width() <= 30));
        assert!(
            Span::raw(fit_text(
                "e\u{301}🇺🇸👨\u{200d}👩\u{200d}👧\u{200d}👦6.1200",
                8
            ))
            .width()
                <= 8
        );
    }

    #[test]
    fn empty_input_is_graceful() {
        let lines = week_lines(&[], 78, None);
        let rendered = all_text(&lines);
        assert!(rendered.contains("No weekly meetings"));
        assert!(lines.iter().all(|line| line.width() <= 78));
    }
}
