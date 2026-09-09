//! Deterministic per-course TUI colors and shared section labels.
//!
//! Colors are assigned by the position of each course in the sorted distinct
//! course list (`nth distinct course`), so every course in a schedule of up to
//! [`PALETTE_LEN`] courses gets a different color. This replaces the old
//! `hash(course_id) % PALETTE_LEN` mapping, which could collide even with two
//! courses.
//!
//! The palette order matches `tui::week::PALETTE` by index.

use std::collections::BTreeSet;

use crate::model::ChosenSection;

/// Number of distinct course colors before the palette wraps.
pub const PALETTE_LEN: usize = 8;

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
        let ordered = (0..=PALETTE_LEN)
            .map(|index| format!("course-{index}"))
            .collect::<Vec<_>>();
        for (index, course_id) in ordered.iter().enumerate() {
            assert_eq!(course_color_index(course_id, &ordered), index % PALETTE_LEN);
        }
    }
}
