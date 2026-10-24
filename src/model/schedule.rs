//! Schedule collection and day filtering methods.

use crate::model::course::CourseEntry;
use crate::model::day::DayOfWeek;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Represents a conflict between two scheduled course sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleConflict {
    /// The day on which the conflict occurs.
    pub day: DayOfWeek,
    /// First course session involved in the conflict.
    pub first: CourseEntry,
    /// Second course session involved in the conflict.
    pub second: CourseEntry,
    /// Specific reason for the conflict (e.g. time clash, room double-booking).
    pub reason: String,
}

/// Complete university schedule across all days of the week.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Schedule {
    /// Name or descriptor of the semester (e.g. `"Level 4 Term 1"`).
    pub semester_name: String,
    /// Timestamp when this schedule was generated/scraped.
    pub generated_at: Option<String>,
    /// All scheduled class sessions.
    pub entries: Vec<CourseEntry>,
}

impl Schedule {
    /// Creates a new [`Schedule`].
    #[must_use]
    pub fn new(
        semester_name: impl Into<String>,
        generated_at: Option<String>,
        entries: Vec<CourseEntry>,
    ) -> Self {
        Self {
            semester_name: semester_name.into(),
            generated_at,
            entries,
        }
    }

    /// Returns all class entries occurring on a specific day, sorted chronologically.
    #[must_use]
    pub fn for_day(&self, day: DayOfWeek) -> Vec<&CourseEntry> {
        let mut day_entries: Vec<&CourseEntry> =
            self.entries.iter().filter(|e| e.day == day).collect();
        day_entries.sort_by_key(|e| e.time_slot.start());
        day_entries
    }

    /// Returns a set of all unique course codes in this schedule (e.g. `["CSE-4101", "CSE-4103"]`).
    #[must_use]
    pub fn all_course_codes(&self) -> BTreeSet<String> {
        self.entries.iter().map(|e| e.code.clone()).collect()
    }

    /// Returns a set of all unique section labels found in the schedule (e.g. `["B", "B1", "B2"]`).
    #[must_use]
    pub fn all_sections(&self) -> BTreeSet<String> {
        self.entries.iter().map(|e| e.section.clone()).collect()
    }

    /// Produces a filtered [`Schedule`] for a specific student section (e.g. `"B2"`).
    ///
    /// Keeps exact matching sections, and preserves broad theory sections
    /// (e.g. section `"B"` theory courses are kept for student section `"B2"`).
    #[must_use]
    pub fn filter_section(&self, target_section: &str) -> Self {
        let target = target_section.trim().to_uppercase();
        let filtered = self
            .entries
            .iter()
            .filter(|e| {
                let sec = e.section.to_uppercase();
                if sec == target {
                    return true;
                }
                // If target is "B2" and course is theory "B", keep it
                if target.starts_with(&sec) && !e.is_lab() {
                    return true;
                }
                false
            })
            .cloned()
            .collect();

        Self {
            semester_name: self.semester_name.clone(),
            generated_at: self.generated_at.clone(),
            entries: filtered,
        }
    }

    /// Produces a filtered [`Schedule`] excluding any course codes marked as dropped.
    #[must_use]
    pub fn filter_active(&self, dropped_codes: &HashSet<String>) -> Self {
        let filtered = self
            .entries
            .iter()
            .filter(|e| {
                !dropped_codes
                    .iter()
                    .any(|d| crate::model::state::codes_match(d, &e.code))
            })
            .cloned()
            .collect();

        Self {
            semester_name: self.semester_name.clone(),
            generated_at: self.generated_at.clone(),
            entries: filtered,
        }
    }

    /// Detects any overlapping time clashes or room double-bookings within the schedule.
    #[must_use]
    pub fn find_conflicts(&self) -> Vec<ScheduleConflict> {
        let mut conflicts = Vec::new();

        for day in DayOfWeek::all() {
            let day_entries = self.for_day(*day);
            for i in 0..day_entries.len() {
                for j in (i + 1)..day_entries.len() {
                    let a = day_entries[i];
                    let b = day_entries[j];

                    if a.time_slot.overlaps_with(&b.time_slot) {
                        let room_info = if a.room == b.room && !a.room.is_empty() {
                            format!(" [Same Room: {}]", a.room)
                        } else {
                            format!(" [Rooms: {} vs {}]", a.room, b.room)
                        };

                        let reason = format!(
                            "Time overlap at {}: [{}] {} ({}) clashes with [{}] {} ({}){}",
                            a.time_slot.format_12h(),
                            a.section,
                            a.code,
                            a.title,
                            b.section,
                            b.code,
                            b.title,
                            room_info
                        );

                        conflicts.push(ScheduleConflict {
                            day: *day,
                            first: a.clone(),
                            second: b.clone(),
                            reason,
                        });
                    }
                }
            }
        }

        conflicts
    }

    /// Returns a map of instructors associated with their contact details.
    #[must_use]
    pub fn teacher_directory(&self) -> BTreeMap<String, crate::model::course::TeacherContact> {
        let mut dir = BTreeMap::new();
        for entry in &self.entries {
            if !entry.teacher.name.is_empty() {
                dir.insert(entry.teacher.name.clone(), entry.teacher.clone());
            }
        }
        dir
    }

    /// Searches for instructors matching a query string across their name, initials, or courses taught.
    /// Returns a list of tuples containing (Instructor Name, Contact Details, Courses Taught).
    #[must_use]
    pub fn find_teachers(
        &self,
        query: &str,
    ) -> Vec<(String, crate::model::course::TeacherContact, Vec<&CourseEntry>)> {
        let q = query.trim().to_lowercase();
        let mut results = Vec::new();
        let directory = self.teacher_directory();

        for (name, contact) in directory {
            let name_lower = name.to_lowercase();

            // Check initials (e.g. "Md. Abul Basar" -> "mab", "ab")
            let initials: String = name
                .split_whitespace()
                .filter_map(|w| w.chars().next())
                .collect::<String>()
                .to_lowercase();

            let taught_courses: Vec<&CourseEntry> = self
                .entries
                .iter()
                .filter(|e| e.teacher.name == name)
                .collect();

            let course_matches = taught_courses.iter().any(|c| {
                c.code.to_lowercase().contains(&q) || c.title.to_lowercase().contains(&q)
            });

            if q.is_empty()
                || name_lower.contains(&q)
                || initials.contains(&q)
                || course_matches
            {
                results.push((name, contact, taught_courses));
            }
        }

        results
    }

    /// Compares this schedule against an updated revision to identify modifications, additions, and removals.
    #[must_use]
    pub fn diff(&self, updated: &Schedule) -> crate::model::diff::ScheduleDiff {
        crate::model::diff::ScheduleDiff::between(self, updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::course::TeacherContact;
    use crate::model::time_slot::TimeSlot;
    use std::str::FromStr;

    #[test]
    fn test_schedule_for_day_sorted() {
        let t = TeacherContact::new("Test", None, None);
        let slot_late = TimeSlot::from_str("11:45 - 1:0").unwrap();
        let slot_early = TimeSlot::from_str("08:00 - 09:15").unwrap();

        let e1 = CourseEntry::new(
            "CSE-4101",
            "AI",
            "B",
            DayOfWeek::Mon,
            slot_late,
            "120",
            t.clone(),
        );
        let e2 = CourseEntry::new(
            "CSE-4103",
            "Compiler",
            "B",
            DayOfWeek::Mon,
            slot_early,
            "121",
            t,
        );

        let sched = Schedule::new("L4T1", None, vec![e1, e2]);
        let mon = sched.for_day(DayOfWeek::Mon);

        assert_eq!(mon.len(), 2);
        assert_eq!(mon[0].code, "CSE-4103"); // 08:00 before 11:45
        assert_eq!(mon[1].code, "CSE-4101");
    }

    #[test]
    fn test_schedule_filter_section() {
        let t = TeacherContact::new("Test", None, None);
        let slot = TimeSlot::from_str("09:15 - 10:30").unwrap();

        let theory_b = CourseEntry::new(
            "CSE-4101",
            "AI",
            "B",
            DayOfWeek::Mon,
            slot,
            "120",
            t.clone(),
        );
        let lab_b1 = CourseEntry::new(
            "CSE-4111",
            "AI Lab",
            "B1",
            DayOfWeek::Mon,
            slot,
            "AC-217",
            t.clone(),
        );
        let lab_b2 = CourseEntry::new(
            "CSE-4111",
            "AI Lab",
            "B2",
            DayOfWeek::Mon,
            slot,
            "AC-131",
            t,
        );

        let sched = Schedule::new("L4T1", None, vec![theory_b, lab_b1, lab_b2]);

        // Filter for B2 student
        let b2_sched = sched.filter_section("B2");
        assert_eq!(b2_sched.entries.len(), 2);
        assert!(b2_sched.entries.iter().any(|e| e.section == "B"));
        assert!(b2_sched.entries.iter().any(|e| e.section == "B2"));
        assert!(!b2_sched.entries.iter().any(|e| e.section == "B1"));
    }

    #[test]
    fn test_conflict_detection() {
        let t = TeacherContact::new("Test", None, None);
        let slot_a = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let slot_b = TimeSlot::from_str("10:00 - 11:15").unwrap(); // Clashes with slot_a

        let e1 = CourseEntry::new(
            "CSE-4101",
            "AI",
            "B1",
            DayOfWeek::Mon,
            slot_a,
            "120",
            t.clone(),
        );
        let e2 = CourseEntry::new(
            "CSE-4103",
            "Compiler",
            "B2",
            DayOfWeek::Mon,
            slot_b,
            "120",
            t,
        );

        let sched = Schedule::new("L4T1", None, vec![e1, e2]);
        let conflicts = sched.find_conflicts();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].day, DayOfWeek::Mon);
        assert!(conflicts[0].reason.contains("[B1] CSE-4101"));
        assert!(conflicts[0].reason.contains("[B2] CSE-4103"));
        assert!(conflicts[0].reason.contains("Same Room: 120"));
    }

    #[test]
    fn test_find_teachers_search() {
        let t1 = TeacherContact::new(
            "Md. Abul Basar",
            Some("01758443472".into()),
            Some("basar@niter.edu.bd".into()),
        );
        let t2 = TeacherContact::new("Dr. Muhammad Asif", None, None);
        let slot = TimeSlot::from_str("10:30 - 13:00").unwrap();

        let e1 = CourseEntry::new(
            "CSE-4125",
            "Machine Learning Lab",
            "B2",
            DayOfWeek::Sun,
            slot,
            "AC-202",
            t1,
        );
        let e2 = CourseEntry::new(
            "CSE-4101",
            "Artificial Intelligence",
            "B",
            DayOfWeek::Sun,
            slot,
            "AC-131",
            t2,
        );

        let sched = Schedule::new("L4T1", None, vec![e1, e2]);

        // Search by name substring
        let res_name = sched.find_teachers("basar");
        assert_eq!(res_name.len(), 1);
        assert_eq!(res_name[0].0, "Md. Abul Basar");
        assert_eq!(res_name[0].2.len(), 1);

        // Search by course code
        let res_course = sched.find_teachers("4125");
        assert_eq!(res_course.len(), 1);
        assert_eq!(res_course[0].0, "Md. Abul Basar");

        // Search by initials
        let res_initials = sched.find_teachers("mab");
        assert_eq!(res_initials.len(), 1);
        assert_eq!(res_initials[0].0, "Md. Abul Basar");

        // Empty query returns all
        let res_all = sched.find_teachers("");
        assert_eq!(res_all.len(), 2);
    }
}
