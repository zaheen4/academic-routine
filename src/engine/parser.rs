//! Ingestion parser for UCAM scraped routine JSON payloads.
//!
//! Converts raw scraper payloads into strongly-typed [`Schedule`] collections,
//! validating days, time intervals, and teacher records. The strict API fails on
//! the first bad row; the reporting API skips bad rows so one malformed scrape row
//! cannot block a whole routine update.

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

    /// A required field was empty.
    #[error("Course '{course}' is missing required field '{field}'")]
    MissingField {
        /// Course code (or a placeholder when it is the missing field).
        course: String,
        /// Name of the missing field.
        field: String,
    },

    /// No usable entries were found; the existing routine must be preserved.
    #[error("No valid routine entries found ({skipped} row(s) skipped)")]
    NoValidEntries {
        /// Number of rows that were skipped.
        skipped: usize,
    },
}

/// A single row-level problem encountered while leniently parsing a routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowIssue {
    /// Zero-based index of the row in the payload.
    pub index: usize,
    /// Course code, if it could be read.
    pub course: String,
    /// Human-readable reason the row was skipped.
    pub reason: String,
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

fn is_placeholder(raw: &RawRoutineEntry) -> bool {
    raw.course_code.trim().is_empty()
        && raw.day.trim().is_empty()
        && raw.time_slot.trim().is_empty()
        && raw.course_title.trim().is_empty()
}

fn validate_row(raw: RawRoutineEntry, index: usize) -> Result<CourseEntry, RowIssue> {
    let course = raw.course_code.trim().to_string();
    let day_text = raw.day.trim().to_string();

    let issue = |reason: String| RowIssue {
        index,
        course: course.clone(),
        reason,
    };

    if course.is_empty() {
        return Err(issue("missing course code".to_string()));
    }
    if day_text.is_empty() {
        return Err(issue("missing day".to_string()));
    }
    if raw.time_slot.trim().is_empty() {
        return Err(issue("missing time slot".to_string()));
    }

    let day = DayOfWeek::from_str(&day_text)
        .map_err(|e| issue(format!("invalid day '{day_text}': {e}")))?;
    let time_slot = TimeSlot::from_str(&raw.time_slot)
        .map_err(|e| issue(format!("invalid time slot '{}': {e}", raw.time_slot)))?;

    let teacher = TeacherContact::new(
        raw.teacher.unwrap_or_default(),
        raw.teacher_phone,
        raw.teacher_email,
    );

    Ok(CourseEntry::new(
        raw.course_code,
        raw.course_title,
        raw.section,
        day,
        time_slot,
        raw.room.unwrap_or_default(),
        teacher,
    ))
}

fn parse_payload(json_content: &str) -> Result<RawRoutinePayload, IngestError> {
    serde_json::from_str(json_content).map_err(|e| IngestError::Json(e.to_string()))
}

fn semester_of(payload: &RawRoutinePayload) -> String {
    payload
        .semester
        .clone()
        .unwrap_or_else(|| "Level 4 Term 1".to_string())
}

/// Parses a raw UCAM routine JSON string into a structured [`Schedule`].
///
/// Strict: the first invalid row fails the whole parse. Use
/// [`parse_routine_json_report`] when a partially bad scrape should still be usable.
///
/// # Errors
/// Returns [`IngestError`] if JSON is malformed, a required field is missing, or
/// an entry contains an unparseable day or invalid time slot.
pub fn parse_routine_json(json_content: &str) -> Result<Schedule, IngestError> {
    let payload = parse_payload(json_content)?;
    let semester_name = semester_of(&payload);
    let mut entries = Vec::with_capacity(payload.entries.len());

    for raw in payload.entries {
        if is_placeholder(&raw) {
            continue;
        }
        let course = raw.course_code.trim().to_string();
        if course.is_empty() {
            return Err(IngestError::MissingField {
                course,
                field: "CourseCode".to_string(),
            });
        }
        let day_text = raw.day.clone();
        let day = DayOfWeek::from_str(&day_text).map_err(|e| IngestError::InvalidDay {
            course: course.clone(),
            day: day_text,
            reason: e.to_string(),
        })?;
        let time_slot =
            TimeSlot::from_str(&raw.time_slot).map_err(|e| IngestError::InvalidTimeSlot {
                course: course.clone(),
                time_slot: raw.time_slot.clone(),
                reason: e.to_string(),
            })?;
        let teacher = TeacherContact::new(
            raw.teacher.unwrap_or_default(),
            raw.teacher_phone,
            raw.teacher_email,
        );
        entries.push(CourseEntry::new(
            raw.course_code,
            raw.course_title,
            raw.section,
            day,
            time_slot,
            raw.room.unwrap_or_default(),
            teacher,
        ));
    }

    Ok(Schedule::new(semester_name, payload.generated_at, entries))
}

/// Parses a routine JSON payload, skipping bad rows and reporting them.
///
/// Empty placeholder rows are ignored silently. A file with no valid entries is
/// rejected with [`IngestError::NoValidEntries`] so an empty scrape cannot wipe
/// a known-good routine.
///
/// # Errors
/// Returns [`IngestError::Json`] for malformed JSON, [`IngestError::NoValidEntries`]
/// when nothing usable remains, or [`IngestError::Io`] at the file boundary.
pub fn parse_routine_json_report(
    json_content: &str,
) -> Result<(Schedule, Vec<RowIssue>), IngestError> {
    let payload = parse_payload(json_content)?;
    let semester_name = semester_of(&payload);
    let generated_at = payload.generated_at.clone();
    let total_rows = payload.entries.len();

    let mut entries = Vec::with_capacity(total_rows);
    let mut issues = Vec::new();

    for (index, raw) in payload.entries.into_iter().enumerate() {
        if is_placeholder(&raw) {
            continue;
        }
        match validate_row(raw, index) {
            Ok(entry) => entries.push(entry),
            Err(issue) => issues.push(issue),
        }
    }

    if entries.is_empty() {
        return Err(IngestError::NoValidEntries {
            skipped: if total_rows == 0 {
                0
            } else {
                issues.len().max(1)
            },
        });
    }

    Ok((Schedule::new(semester_name, generated_at, entries), issues))
}

/// Loads and parses a UCAM routine JSON file from the filesystem.
///
/// Uses the lenient reporting parser: bad rows are skipped and summarized on
/// stderr, while an entirely unusable file is rejected.
///
/// # Errors
/// Returns [`IngestError`] if reading fails, JSON is malformed, or no valid rows remain.
pub fn load_routine_file(path: &Path) -> Result<Schedule, IngestError> {
    let content = fs::read_to_string(path).map_err(|e| IngestError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;

    let (schedule, issues) = parse_routine_json_report(&content)?;
    if !issues.is_empty() {
        eprintln!(
            "⚠️ Routine '{}': loaded {} entries, skipped {} malformed row(s).",
            path.display(),
            schedule.entries.len(),
            issues.len()
        );
        for issue in issues.iter().take(5) {
            eprintln!(
                "   • row {} [{}]: {}",
                issue.index, issue.course, issue.reason
            );
        }
    }
    Ok(schedule)
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
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(matches!(
            parse_routine_json("{ invalid json"),
            Err(IngestError::Json(_))
        ));
    }

    #[test]
    fn test_lenient_parse_skips_bad_rows() {
        let json = r#"{
            "semester": "L4T1",
            "entries": [
                {"CourseCode":"CSE-4101","CourseTitle":"AI","Section":"B","Day":"Mon","Room":"120","TimeSlot":"9:15 - 10:30"},
                {"CourseCode":"CSE-4102","CourseTitle":"Bad Day","Section":"B","Day":"Funday","Room":"1","TimeSlot":"9:15 - 10:30"},
                {"CourseCode":"CSE-4103","CourseTitle":"Bad Time","Section":"B","Day":"Tue","Room":"1","TimeSlot":"nope"},
                {"CourseCode":"","CourseTitle":"","Section":"","Day":"","Room":"","TimeSlot":""}
            ]
        }"#;

        let (schedule, issues) = parse_routine_json_report(json).unwrap();
        assert_eq!(schedule.entries.len(), 1);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].course, "CSE-4102");
        assert_eq!(issues[1].course, "CSE-4103");

        // Strict mode still fails on the first bad row.
        assert!(parse_routine_json(json).is_err());
    }

    #[test]
    fn test_all_rows_invalid_is_rejected() {
        let json = r#"{"entries":[
            {"CourseCode":"CSE-4102","Day":"Funday","TimeSlot":"9:15 - 10:30"},
            {"CourseCode":"CSE-4103","Day":"Tue","TimeSlot":"nope"}
        ]}"#;
        assert!(matches!(
            parse_routine_json_report(json),
            Err(IngestError::NoValidEntries { .. })
        ));
    }

    #[test]
    fn test_empty_payload_is_rejected() {
        assert!(matches!(
            parse_routine_json_report(r#"{"entries":[]}"#),
            Err(IngestError::NoValidEntries { skipped: 0 })
        ));
    }
}
