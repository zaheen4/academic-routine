//! Routine diffing engine for detecting timetable modifications, time shifts, room changes, and new/dropped sessions.

use crate::model::course::CourseEntry;
use crate::model::day::DayOfWeek;
use crate::model::schedule::Schedule;
use crate::model::state::codes_match;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Specific type of schedule modification detected between revisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeType {
    /// A newly scheduled lecture or lab session.
    Added(CourseEntry),
    /// A cancelled or removed class session.
    Removed(CourseEntry),
    /// An existing class session with altered room, time, or teacher.
    Modified {
        /// Previous state of the class session.
        old: CourseEntry,
        /// Updated state of the class session.
        new: CourseEntry,
        /// Whether the room or lab designation was changed.
        room_changed: bool,
        /// Whether the start or end time was shifted.
        time_changed: bool,
        /// Whether the assigned faculty member changed.
        teacher_changed: bool,
    },
}

/// A single atomic change identified within the schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleDiffItem {
    /// Official course identifier.
    pub course_code: String,
    /// Human-readable title of the course.
    pub course_title: String,
    /// Section designation (e.g. `"B"`, `"B2"`).
    pub section: String,
    /// Day of the week when the class occurs.
    pub day: DayOfWeek,
    /// Exact modification applied to this session.
    pub change: ChangeType,
}

/// Comprehensive diff report between two schedule revisions.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScheduleDiff {
    /// Descriptor of the previous semester revision.
    pub old_semester: String,
    /// Descriptor of the updated semester revision.
    pub new_semester: String,
    /// All atomic changes detected between the two revisions.
    pub items: Vec<ScheduleDiffItem>,
}

impl ScheduleDiff {
    /// Compares two [`Schedule`] instances and generates a list of differences.
    #[must_use]
    pub fn between(old: &Schedule, new: &Schedule) -> Self {
        let mut items = Vec::new();

        // 1. Gather all unique (code, section, day) keys across both schedules
        let mut all_keys: BTreeSet<(String, String, DayOfWeek)> = BTreeSet::new();

        for e in &old.entries {
            all_keys.insert((e.code.to_uppercase(), e.section.to_uppercase(), e.day));
        }
        for e in &new.entries {
            all_keys.insert((e.code.to_uppercase(), e.section.to_uppercase(), e.day));
        }

        // 2. Compare sessions for each key
        for (code, sec, day) in all_keys {
            let mut old_matches: Vec<&CourseEntry> = old
                .entries
                .iter()
                .filter(|e| codes_match(&e.code, &code) && e.section.eq_ignore_ascii_case(&sec) && e.day == day)
                .collect();
            old_matches.sort_by_key(|e| e.time_slot.start());

            let mut new_matches: Vec<&CourseEntry> = new
                .entries
                .iter()
                .filter(|e| codes_match(&e.code, &code) && e.section.eq_ignore_ascii_case(&sec) && e.day == day)
                .collect();
            new_matches.sort_by_key(|e| e.time_slot.start());

            let common_len = old_matches.len().min(new_matches.len());

            // Pair existing sessions
            for i in 0..common_len {
                let o = old_matches[i];
                let n = new_matches[i];

                let room_changed = o.room.trim() != n.room.trim();
                let time_changed = o.time_slot != n.time_slot;
                let teacher_changed = o.teacher.name.trim() != n.teacher.name.trim();

                if room_changed || time_changed || teacher_changed {
                    items.push(ScheduleDiffItem {
                        course_code: n.code.clone(),
                        course_title: n.title.clone(),
                        section: n.section.clone(),
                        day,
                        change: ChangeType::Modified {
                            old: (*o).clone(),
                            new: (*n).clone(),
                            room_changed,
                            time_changed,
                            teacher_changed,
                        },
                    });
                }
            }

            // Remaining additions in new
            if new_matches.len() > common_len {
                for n in &new_matches[common_len..] {
                    items.push(ScheduleDiffItem {
                        course_code: n.code.clone(),
                        course_title: n.title.clone(),
                        section: n.section.clone(),
                        day,
                        change: ChangeType::Added((*n).clone()),
                    });
                }
            }

            // Remaining removals from old
            if old_matches.len() > common_len {
                for o in &old_matches[common_len..] {
                    items.push(ScheduleDiffItem {
                        course_code: o.code.clone(),
                        course_title: o.title.clone(),
                        section: o.section.clone(),
                        day,
                        change: ChangeType::Removed((*o).clone()),
                    });
                }
            }
        }

        Self {
            old_semester: old.semester_name.clone(),
            new_semester: new.semester_name.clone(),
            items,
        }
    }

    /// Returns `true` if there are no differences between the schedules.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the number of differences detected.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Filters the diff to only include changes relevant to a specific student section (e.g. `"B2"`).
    #[must_use]
    pub fn filter_section(&self, target_section: &str) -> Self {
        let target = target_section.trim().to_uppercase();
        if target == "ALL" || target.is_empty() {
            return self.clone();
        }

        let filtered_items = self
            .items
            .iter()
            .filter(|item| {
                let sec = item.section.to_uppercase();
                if sec == target {
                    return true;
                }
                // Theory sections (e.g. "B") apply to sub-sections (e.g. "B2")
                if target.len() > 1 && sec == target[..target.len() - 1] {
                    return true;
                }
                false
            })
            .cloned()
            .collect();

        Self {
            old_semester: self.old_semester.clone(),
            new_semester: self.new_semester.clone(),
            items: filtered_items,
        }
    }

    /// Renders a concise, formatted Telegram HTML card for change alerts.
    #[must_use]
    pub fn render_telegram_card(&self) -> String {
        if self.is_empty() {
            return "ℹ️ <b>Routine Unchanged:</b> No timetable modifications detected.".to_string();
        }

        let mut out = String::from(
            "🔔 <b>Timetable Updates Detected!</b>\n\
             ───────────────────────────\n\
             The university routine has been updated on UCAM:\n\n",
        );

        for item in &self.items {
            let day_name = item.day.as_str();
            match &item.change {
                ChangeType::Modified {
                    old,
                    new,
                    room_changed,
                    time_changed,
                    teacher_changed,
                } => {
                    out.push_str(&format!(
                        "• <b>{} ({})</b> [{}]:\n",
                        escape_html(&item.course_code),
                        escape_html(&item.section),
                        day_name
                    ));
                    if *room_changed {
                        out.push_str(&format!(
                            "  📍 Room: <code>{}</code> ➔ <b>{}</b>\n",
                            escape_html(&old.room),
                            escape_html(&new.room)
                        ));
                    }
                    if *time_changed {
                        out.push_str(&format!(
                            "  ⏰ Time: <code>{}</code> ➔ <b>{}</b>\n",
                            old.time_slot.format_12h(),
                            new.time_slot.format_12h()
                        ));
                    }
                    if *teacher_changed {
                        out.push_str(&format!(
                            "  👨‍🏫 Faculty: <code>{}</code> ➔ <b>{}</b>\n",
                            escape_html(&old.teacher.name),
                            escape_html(&new.teacher.name)
                        ));
                    }
                    out.push('\n');
                }
                ChangeType::Added(entry) => {
                    let lab_tag = if entry.is_lab() { " 🔬 [Lab]" } else { "" };
                    out.push_str(&format!(
                        "• <b>{} ({})</b> [{}]{}:\n  ➕ <i>New session:</i> <b>{}</b> (Room <b>{}</b>)\n\n",
                        escape_html(&item.course_code),
                        escape_html(&item.section),
                        day_name,
                        lab_tag,
                        entry.time_slot.format_12h(),
                        escape_html(&entry.room)
                    ));
                }
                ChangeType::Removed(entry) => {
                    out.push_str(&format!(
                        "• <b>{} ({})</b> [{}]:\n  ➖ <i>Cancelled / removed:</i> {} (Room {})\n\n",
                        escape_html(&item.course_code),
                        escape_html(&item.section),
                        day_name,
                        entry.time_slot.format_12h(),
                        escape_html(&entry.room)
                    ));
                }
            }
        }

        out.push_str(
            "───────────────────────────\n\
             <i>Your calendar feed (.ics) and daily alerts have been synchronized.</i>",
        );

        out
    }

    /// Renders a plain-text summary suitable for terminal output.
    #[must_use]
    pub fn render_terminal(&self) -> String {
        if self.is_empty() {
            return "No changes detected between schedule revisions.\n".to_string();
        }

        let mut out = format!(
            "Timetable Revisions Diff: {} changes detected\n\
             =======================================================\n",
            self.items.len()
        );

        for item in &self.items {
            let day_name = item.day.as_str();
            match &item.change {
                ChangeType::Modified {
                    old,
                    new,
                    room_changed,
                    time_changed,
                    teacher_changed,
                } => {
                    out.push_str(&format!(
                        "[MODIFIED] {} ({}) on {}\n",
                        item.course_code, item.section, day_name
                    ));
                    if *room_changed {
                        out.push_str(&format!("  Room:    {} -> {}\n", old.room, new.room));
                    }
                    if *time_changed {
                        out.push_str(&format!(
                            "  Time:    {} -> {}\n",
                            old.time_slot.format_12h(),
                            new.time_slot.format_12h()
                        ));
                    }
                    if *teacher_changed {
                        out.push_str(&format!(
                            "  Teacher: {} -> {}\n",
                            old.teacher.name, new.teacher.name
                        ));
                    }
                }
                ChangeType::Added(entry) => {
                    out.push_str(&format!(
                        "[ADDED]    {} ({}) on {}: {} in Room {}\n",
                        item.course_code,
                        item.section,
                        day_name,
                        entry.time_slot.format_12h(),
                        entry.room
                    ));
                }
                ChangeType::Removed(entry) => {
                    out.push_str(&format!(
                        "[REMOVED]  {} ({}) on {}: {} in Room {}\n",
                        item.course_code,
                        item.section,
                        day_name,
                        entry.time_slot.format_12h(),
                        entry.room
                    ));
                }
            }
        }

        out
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::course::TeacherContact;
    use crate::model::time_slot::TimeSlot;

    fn sample_course(
        code: &str,
        sec: &str,
        day: DayOfWeek,
        slot_str: &str,
        room: &str,
        teacher: &str,
    ) -> CourseEntry {
        CourseEntry::new(
            code,
            "Sample Course",
            sec,
            day,
            TimeSlot::parse_ucam(slot_str).unwrap(),
            room,
            TeacherContact::new(teacher, None, None),
        )
    }

    #[test]
    fn test_diff_empty_when_identical() {
        let e1 = sample_course("CSE-4101", "B", DayOfWeek::Sun, "08:30 - 10:00", "612", "Dr. A");
        let sched1 = Schedule::new("L4T1", None, vec![e1.clone()]);
        let sched2 = Schedule::new("L4T1", None, vec![e1]);

        let diff = ScheduleDiff::between(&sched1, &sched2);
        assert!(diff.is_empty());
        assert_eq!(diff.len(), 0);
    }

    #[test]
    fn test_diff_detects_room_and_time_changes() {
        let e_old = sample_course("CSE-4101", "B", DayOfWeek::Sun, "08:30 - 10:00", "612", "Dr. A");
        let e_new = sample_course("CSE-4101", "B", DayOfWeek::Sun, "09:00 - 10:30", "610", "Dr. A");

        let sched_old = Schedule::new("L4T1", None, vec![e_old]);
        let sched_new = Schedule::new("L4T1", None, vec![e_new]);

        let diff = ScheduleDiff::between(&sched_old, &sched_new);
        assert_eq!(diff.len(), 1);

        let item = &diff.items[0];
        assert_eq!(item.course_code, "CSE-4101");
        match &item.change {
            ChangeType::Modified {
                room_changed,
                time_changed,
                teacher_changed,
                ..
            } => {
                assert!(*room_changed);
                assert!(*time_changed);
                assert!(!*teacher_changed);
            }
            _ => panic!("Expected Modified change"),
        }
    }

    #[test]
    fn test_diff_detects_additions_and_removals() {
        let e1 = sample_course("CSE-4101", "B", DayOfWeek::Sun, "08:30 - 10:00", "612", "Dr. A");
        let e_added = sample_course("CSE-4126", "B2", DayOfWeek::Tue, "01:30 - 04:30", "Lab 4", "SS");

        let sched_old = Schedule::new("L4T1", None, vec![e1.clone()]);
        let sched_new = Schedule::new("L4T1", None, vec![e1, e_added.clone()]);

        let diff = ScheduleDiff::between(&sched_old, &sched_new);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.items[0].course_code, "CSE-4126");
        assert!(matches!(diff.items[0].change, ChangeType::Added(_)));

        // Reverse should detect removal
        let rev_diff = ScheduleDiff::between(&sched_new, &sched_old);
        assert_eq!(rev_diff.len(), 1);
        assert_eq!(rev_diff.items[0].course_code, "CSE-4126");
        assert!(matches!(rev_diff.items[0].change, ChangeType::Removed(_)));
    }

    #[test]
    fn test_diff_section_filtering() {
        let e_b1 = sample_course("CSE-4126", "B1", DayOfWeek::Mon, "01:30 - 04:30", "Lab 4", "SS");
        let e_b2 = sample_course("CSE-4126", "B2", DayOfWeek::Tue, "01:30 - 04:30", "Lab 4", "SS");
        let e_theory = sample_course("CSE-4101", "B", DayOfWeek::Sun, "08:30 - 10:00", "612", "Dr. A");

        let sched_old = Schedule::new("L4T1", None, vec![]);
        let sched_new = Schedule::new("L4T1", None, vec![e_b1, e_b2, e_theory]);

        let full_diff = ScheduleDiff::between(&sched_old, &sched_new);
        assert_eq!(full_diff.len(), 3);

        // Section B2 should see theory (B) and lab (B2), but NOT lab (B1)
        let b2_diff = full_diff.filter_section("B2");
        assert_eq!(b2_diff.len(), 2);
        assert!(b2_diff.items.iter().all(|i| i.section == "B" || i.section == "B2"));
    }
}
