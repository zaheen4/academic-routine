//! Ingestion parser for UCAM scraped routine JSON payloads.
//!
//! Converts raw scraper payloads (e.g. `final_combined_routine.json`) into strongly-typed
//! [`Schedule`] collections, validating days, time intervals, and teacher records.

use crate::model::course::{CourseEntry, TeacherContact};
use crate::model::day::DayOfWeek;
use crate::model::schedule::Schedule;
use crate::model::time_slot::TimeSlot;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::str::FromStr;

/// Error returned when parsing or reading a routine JSON payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IngestError {
    /// File read error.
    #[error("Failed to read routine file '{path}': {message}")]
    Io {
        /// The file path.
        path: String,
        /// The underlying I/O error message.
        message: String,
    },

    /// JSON syntax error.
    #[error("Failed to parse routine JSON: {0}")]
    Json(String),

    /// Missing or invalid day of week in an entry.
    #[error("Course '{course}' has invalid day '{day}': {reason}")]
    InvalidDay {
        /// Course code.
        course: String,
        /// Given day string.
        day: String,
        /// Error description.
        reason: String,
    },

    /// Missing or invalid time slot in an entry.
    #[error("Course '{course}' has invalid time slot '{time_slot}': {reason}")]
    InvalidTimeSlot {
        /// Course code.
        course: String,
        /// Given time slot string.
        time_slot: String,
        /// Error description.
        reason: String,
    },
}

#[derive(Debug, Deserialize)]
struct RawRoutinePayload {
    #[serde(default)]
    generated_at: Option<String>,
    #[serde(default)]
    semester: Option<String>,
    #[serde(default)]
    entries: Vec<RawRoutineEntry>,
}

#[derive(Debug, Deserialize)]
struct RawRoutineEntry {
    #[serde(rename = "CourseCode", default)]
    course_code: String,
    #[serde(rename = "CourseTitle", default)]
    course_title: String,
    #[serde(rename = "Section", default)]
    section: String,
    #[serde(rename = "Day", default)]
    day: String,
    #[serde(rename = "Room", default)]
    room: Option<String>,
    #[serde(rename = "TimeSlot", default)]
    time_slot: String,
    #[serde(rename = "Teacher", default)]
    teacher: Option<String>,
    #[serde(rename = "TeacherPhone", default)]
    teacher_phone: Option<String>,
    #[serde(rename = "TeacherEmail", default)]
    teacher_email: Option<String>,
}

/// Parses a raw UCAM routine JSON string into a structured [`Schedule`].
///
/// # Errors
/// Returns [`IngestError`] if JSON syntax is malformed, or if an entry contains
/// an unparseable day name or invalid time slot.
pub fn parse_routine_json(json_content: &str) -> Result<Schedule, IngestError> {
    let payload: RawRoutinePayload =
        serde_json::from_str(json_content).map_err(|e| IngestError::Json(e.to_string()))?;

    let semester_name = payload
        .semester
        .unwrap_or_else(|| "Level 4 Term 1".to_string());
    let mut entries = Vec::with_capacity(payload.entries.len());

    for raw in payload.entries {
        // Skip completely empty placeholders
        if raw.course_code.trim().is_empty() && raw.day.trim().is_empty() {
            continue;
        }

        let day = DayOfWeek::from_str(&raw.day).map_err(|e| IngestError::InvalidDay {
            course: raw.course_code.clone(),
            day: raw.day.clone(),
            reason: e.to_string(),
        })?;

        let time_slot =
            TimeSlot::from_str(&raw.time_slot).map_err(|e| IngestError::InvalidTimeSlot {
                course: raw.course_code.clone(),
                time_slot: raw.time_slot.clone(),
                reason: e.to_string(),
            })?;

        let teacher = TeacherContact::new(
            raw.teacher.unwrap_or_default(),
            raw.teacher_phone,
            raw.teacher_email,
        );

        let entry = CourseEntry::new(
            raw.course_code,
            raw.course_title,
            raw.section,
            day,
            time_slot,
            raw.room.unwrap_or_default(),
            teacher,
        );

        entries.push(entry);
    }

    Ok(Schedule::new(semester_name, payload.generated_at, entries))
}

/// Loads and parses a UCAM routine JSON file from the filesystem.
///
/// # Errors
/// Returns [`IngestError`] if reading the file fails or if parsing fails.
pub fn load_routine_file(path: &Path) -> Result<Schedule, IngestError> {
    let content = fs::read_to_string(path).map_err(|e| IngestError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;

    parse_routine_json(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_UCAM_JSON: &str = r#"{
        "generated_at": "2026-10-17T13:03:50Z",
        "semester": "2024-2025 Level 4 Term 1 January-July 2026",
        "entries": [
            {
                "CourseCode": "CSE-4101",
                "CourseTitle": "Artificial Intelligence",
                "Section": "B",
                "Day": "Mon",
                "Room": "120",
                "TimeSlot": "9:15 - 10:30",
                "Teacher": "Utpol Kanti Das",
                "TeacherPhone": "01859222633",
                "TeacherEmail": "ukdas@niter.edu.bd"
            },
            {
                "CourseCode": "CSE-4101",
                "CourseTitle": "Artificial Intelligence",
                "Section": "B",
                "Day": "Wed",
                "Room": "120",
                "TimeSlot": "8:0 - 9:15",
                "Teacher": "Utpol Kanti Das",
                "TeacherPhone": "01859222633",
                "TeacherEmail": "ukdas@niter.edu.bd"
            }
        ]
    }"#;

    #[test]
    fn test_parse_sample_ucam_json() {
        let schedule = parse_routine_json(SAMPLE_UCAM_JSON).unwrap();

        assert_eq!(schedule.entries.len(), 2);
        assert_eq!(
            schedule.semester_name,
            "2024-2025 Level 4 Term 1 January-July 2026"
        );

        let mon_entries = schedule.for_day(DayOfWeek::Mon);
        assert_eq!(mon_entries.len(), 1);
        assert_eq!(mon_entries[0].code, "CSE-4101");
        assert_eq!(mon_entries[0].time_slot.format_12h(), "09:15 AM - 10:30 AM");

        let wed_entries = schedule.for_day(DayOfWeek::Wed);
        assert_eq!(wed_entries.len(), 1);
        assert_eq!(wed_entries[0].code, "CSE-4101");
        assert_eq!(wed_entries[0].time_slot.format_12h(), "08:00 AM - 09:15 AM");
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(matches!(
            parse_routine_json("{ invalid json"),
            Err(IngestError::Json(_))
        ));
    }
}
