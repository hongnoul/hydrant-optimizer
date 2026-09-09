//! Deterministic per-course color assignment shared by the TUI and ICS export.
//!
//! Colors are assigned by the position of each course in the sorted distinct
//! course list (`nth distinct course`), so every course in a schedule of up to
//! [`PALETTE_LEN`] courses gets a different color. This replaces the old
//! `hash(course_id) % PALETTE_LEN` mapping, which could collide even with two
//! courses.
//!
//! The palette order matches `tui::week::PALETTE` by index. Each entry also
//! carries the closest Google Calendar event color (hex + `colorId`) so the
//! ICS exporter can embed data that makes manual recoloring after import a
//! one-click choice. Google Calendar ignores per-event colors on ICS import,
//! so the exporter also repeats the assignment in `CATEGORIES`, `DESCRIPTION`,
//! and `X-HYDRANT-*` properties; see `calendar::export_ics`.

use std::collections::BTreeSet;

use crate::model::ChosenSection;

/// Number of distinct course colors before the palette wraps.
pub const PALETTE_LEN: usize = 8;

/// Google Calendar event color name for each palette index.
pub const GCAL_NAMES: [&str; PALETTE_LEN] = [
    "Blueberry",
    "Basil",
    "Grape",
    "Peacock",
    "Lavender",
    "Sage",
    "Flamingo",
    "Banana",
];

/// Hex background for each palette index (closest Google Calendar event color).
pub const GCAL_HEXES: [&str; PALETTE_LEN] = [
    "#3F51B5", "#0B8043", "#8E24AA", "#039BE5", "#7986CB", "#33B679", "#E67C73", "#F6BF26",
];

/// Google Calendar event `colorId` for each palette index.
pub const GCAL_IDS: [&str; PALETTE_LEN] = ["9", "10", "3", "7", "1", "2", "4", "5"];

/// Sorted distinct course IDs in a chosen-section list.
pub fn sorted_course_ids(sections: &[ChosenSection]) -> Vec<String> {
    sections
        .iter()
        .map(|section| section.course_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Palette index for `course_id` within a precomputed `sorted_course_ids` order.
///
/// Falls back to position 0 for courses missing from the order (should not
/// happen when the order was built from the same section list).
pub fn course_color_index(course_id: &str, ordered_course_ids: &[String]) -> usize {
    ordered_course_ids
        .iter()
        .position(|id| id == course_id)
        .unwrap_or(0)
        % PALETTE_LEN
}

/// Full color record for one palette index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CourseColor {
    pub index: usize,
    pub gcal_name: &'static str,
    pub hex: &'static str,
    pub gcal_id: &'static str,
}

/// Short timetable legend for a section-kind string, e.g. `Lec`, `Rec`, `Lab`.
/// Shared by the TUI grid and ICS event titles so both read like Hydrant.
pub fn component_label(kind: &str) -> String {
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

impl CourseColor {
    pub fn by_index(index: usize) -> Self {
        let index = index % PALETTE_LEN;
        Self {
            index,
            gcal_name: GCAL_NAMES[index],
            hex: GCAL_HEXES[index],
            gcal_id: GCAL_IDS[index],
        }
    }

    pub fn for_course(course_id: &str, ordered_course_ids: &[String]) -> Self {
        Self::by_index(course_color_index(course_id, ordered_course_ids))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(course_id: &str) -> ChosenSection {
        ChosenSection {
            course_id: course_id.to_string(),
            course_title: format!("{course_id} title"),
            kind: "lecture".to_string(),
            section: crate::model::SectionOption {
                id: format!("{course_id}-L1"),
                label: "L1".to_string(),
                room: String::new(),
                source: crate::model::Source::Manual,
                meetings: Vec::new(),
                incompatible_with: Default::default(),
                unsupported_reason: None,
            },
        }
    }

    #[test]
    fn nth_distinct_order_is_sorted_and_collision_free() {
        let sections = vec![section("C"), section("A"), section("B"), section("A")];
        let ordered = sorted_course_ids(&sections);
        assert_eq!(ordered, vec!["A", "B", "C"]);
        let indexes = ordered
            .iter()
            .map(|id| course_color_index(id, &ordered))
            .collect::<Vec<_>>();
        assert_eq!(indexes, vec![0, 1, 2]);
    }

    #[test]
    fn component_labels_match_hydrant_legends() {
        assert_eq!(component_label("lecture"), "Lec");
        assert_eq!(component_label("recitation"), "Rec");
        assert_eq!(component_label("lab"), "Lab");
        assert_eq!(component_label("pe"), "PE");
        assert_eq!(component_label("design"), "Design");
        assert_eq!(component_label("seminar"), "Sem");
    }

    #[test]
    fn palette_wraps_only_past_capacity() {
        assert_eq!(CourseColor::by_index(8).index, 0);
        assert_eq!(CourseColor::by_index(2).hex, "#8E24AA");
        assert_eq!(CourseColor::by_index(2).gcal_id, "3");
    }
}
