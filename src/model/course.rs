//! Course entry and teacher contact information models.

use crate::model::day::DayOfWeek;
use crate::model::time_slot::TimeSlot;
use serde::{Deserialize, Serialize};

/// Contact details for a course instructor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct TeacherContact {
    /// Full name or initials of the teacher.
    pub name: String,
    /// Phone or mobile number, if available.
    pub phone: Option<String>,
    /// Institutional or personal email address, if available.
    pub email: Option<String>,
}

impl TeacherContact {
    /// Creates a new [`TeacherContact`].
    pub fn new(name: impl Into<String>, phone: Option<String>, email: Option<String>) -> Self {
        Self {
            name: name.into().trim().to_string(),
            phone: phone
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            email: email
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        }
    }

    /// Formats contact information for calendar descriptions or CLI displays.
    #[must_use]
    pub fn formatted_summary(&self) -> String {
        let mut parts = vec![format!("Teacher: {}", self.name)];
        if let Some(ref p) = self.phone {
            parts.push(format!("Phone: {p}"));
        }
        if let Some(ref e) = self.email {
            parts.push(format!("Email: {e}"));
        }
        parts.join(" | ")
    }
}

/// A scheduled course lecture or lab session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CourseEntry {
    /// Official course identifier (e.g. `"CSE-4101"`).
    pub code: String,
    /// Full title of the course (e.g. `"Artificial Intelligence"`).
    pub title: String,
    /// Class or lab section label (e.g. `"B"`, `"B1"`, `"B2"`).
    pub section: String,
    /// Day of the week on which this session occurs.
    pub day: DayOfWeek,
    /// Assigned time period for this session.
    pub time_slot: TimeSlot,
    /// Assigned room or laboratory identifier (e.g. `"120"`, `"Lab 4"`).
    pub room: String,
    /// Assigned instructor details.
    pub teacher: TeacherContact,
}

impl CourseEntry {
    /// Creates a new [`CourseEntry`].
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        title: impl Into<String>,
        section: impl Into<String>,
        day: DayOfWeek,
        time_slot: TimeSlot,
        room: impl Into<String>,
        teacher: TeacherContact,
    ) -> Self {
        Self {
            code: code.into().trim().to_uppercase(),
            title: title.into().trim().to_string(),
            section: section.into().trim().to_uppercase(),
            day,
            time_slot,
            room: room.into().trim().to_string(),
            teacher,
        }
    }

    /// Determines whether this entry represents a laboratory/sessional course.
    #[must_use]
    pub fn is_lab(&self) -> bool {
        let title_upper = self.title.to_uppercase();
        title_upper.contains("LAB")
            || title_upper.contains("SESSIONAL")
            || self.section.chars().any(|c| c.is_ascii_digit())
    }

    /// Generates a deterministic, stable RFC 5545 iCalendar `UID`.
    ///
    /// Formatted as `<clean_code>-<day>-<hhmm>@academic-routine`.
    #[must_use]
    pub fn ical_uid(&self) -> String {
        let clean_code = self.code.to_lowercase().replace([' ', '-', '_', '.'], "");
        let day_str = self.day.as_str().to_lowercase();
        let time_str = self.time_slot.ical_start_time();
        format!("{clean_code}-{day_str}-{time_str}@academic-routine")
    }

    /// Formats a concise one-line summary for notifications or CLI listings.
    #[must_use]
    pub fn one_line_summary(&self) -> String {
        format!(
            "[{}] {} ({}) - Room {} ({})",
            self.code, self.title, self.section, self.room, self.time_slot
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_course_entry_creation_and_lab_detection() {
        let slot = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let teacher = TeacherContact::new(
            "Utpol Kanti Das",
            Some("01859222633".into()),
            Some("ukdas@niter.edu.bd".into()),
        );

        let theory = CourseEntry::new(
            "CSE-4101",
            "Artificial Intelligence",
            "B",
            DayOfWeek::Mon,
            slot,
            "120",
            teacher.clone(),
        );

        assert!(!theory.is_lab());
        assert_eq!(theory.code, "CSE-4101");
        assert_eq!(theory.ical_uid(), "cse4101-mon-091500@academic-routine");
        assert_eq!(
            theory.teacher.formatted_summary(),
            "Teacher: Utpol Kanti Das | Phone: 01859222633 | Email: ukdas@niter.edu.bd"
        );

        let lab = CourseEntry::new(
            "CSE-4102",
            "Artificial Intelligence Sessional",
            "B1",
            DayOfWeek::Wed,
            slot,
            "Lab 2",
            teacher,
        );
        assert!(lab.is_lab());
    }
}
