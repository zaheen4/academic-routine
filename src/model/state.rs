//! Persistent state management for course dropping and semester lifecycles.

use crate::model::day::academic_now;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn default_true() -> bool {
    true
}

/// Error returned when loading or saving [`RoutineState`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// Failed to read or write the state file.
    #[error("I/O error accessing state file '{path}': {message}")]
    Io {
        /// File path that caused the error.
        path: String,
        /// Underlying error description.
        message: String,
    },

    /// Failed to parse or serialize the state JSON.
    #[error("JSON serialization error: {0}")]
    Json(String),
}

/// User state tracking dropped/hidden courses and semester parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineState {
    /// Name or descriptor of the current semester.
    pub semester_name: String,
    /// Explicit final day of the semester (sets the RFC 5545 `UNTIL` rule).
    pub semester_end_date: Option<NaiveDate>,
    /// Optional GitHub Gist ID used for automatic live calendar syncing.
    #[serde(default)]
    pub gist_id: Option<String>,
    /// Set of uppercase course codes excluded from calendars and live queries.
    pub dropped_courses: HashSet<String>,

    /// Controls whether proactive 6:00 AM morning briefings are sent.
    #[serde(default = "default_true")]
    pub morning_alerts: bool,

    /// Vacation or semester break mode: silences alerts and automated scrapes.
    #[serde(default)]
    pub vacation_mode: bool,

    /// Optional auto-expiration date for vacation mode (YYYY-MM-DD).
    #[serde(default)]
    pub vacation_until: Option<NaiveDate>,

    /// Whether UCAM sync should automatically push to GitHub Gist.
    #[serde(default = "default_true")]
    pub auto_sync_gist: bool,

    /// Whether UCAM sync should automatically push to Google Sheets.
    #[serde(default = "default_true")]
    pub auto_sync_gsheet: bool,

    /// Cached Telegram username or display handle.
    #[serde(default)]
    pub telegram_username: Option<String>,
}

impl Default for RoutineState {
    fn default() -> Self {
        Self {
            semester_name: "Level 4 Term 1".to_string(),
            semester_end_date: None,
            gist_id: None,
            dropped_courses: HashSet::new(),
            morning_alerts: true,
            vacation_mode: false,
            vacation_until: None,
            auto_sync_gist: true,
            auto_sync_gsheet: true,
            telegram_username: None,
        }
    }
}

/// Helper to check whether two course codes match, ignoring case and punctuation (e.g. '-' or ' ').
#[must_use]
pub fn codes_match(a: &str, b: &str) -> bool {
    let clean_a: String = a
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect();
    let clean_b: String = b
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect();
    !clean_a.is_empty() && clean_a == clean_b
}

impl RoutineState {
    /// Drops or marks a course as completed, hiding it from calendar and queries.
    ///
    /// Returns `true` if the course was newly added to the completed/dropped set.
    pub fn drop_course(&mut self, code: &str) -> bool {
        let clean = code.trim().to_uppercase();
        if clean.is_empty() || self.is_dropped(&clean) {
            return false;
        }
        self.dropped_courses.insert(clean)
    }

    /// Marks a course whose syllabus is done as completed. Alias for [`drop_course`].
    pub fn complete_course(&mut self, code: &str) -> bool {
        self.drop_course(code)
    }

    /// Restores or resumes an active schedule for a course.
    ///
    /// Returns `true` if the course was present and removed from the completed/dropped set.
    pub fn restore_course(&mut self, code: &str) -> bool {
        let to_remove: Vec<String> = self
            .dropped_courses
            .iter()
            .filter(|d| codes_match(d, code))
            .cloned()
            .collect();

        if to_remove.is_empty() {
            false
        } else {
            for r in to_remove {
                self.dropped_courses.remove(&r);
            }
            true
        }
    }

    /// Resumes an active schedule for a previously completed course. Alias for [`restore_course`].
    pub fn resume_course(&mut self, code: &str) -> bool {
        self.restore_course(code)
    }

    /// Checks if a course is currently marked as dropped or completed.
    #[must_use]
    pub fn is_dropped(&self, code: &str) -> bool {
        self.dropped_courses.iter().any(|d| codes_match(d, code))
    }

    /// Checks if a course is marked as completed. Alias for [`is_dropped`].
    #[must_use]
    pub fn is_completed(&self, code: &str) -> bool {
        self.is_dropped(code)
    }

    /// Returns true if vacation mode is currently active.
    ///
    /// Automatically clears and expires vacation mode if `vacation_until` is in the past.
    pub fn is_vacation_active(&mut self) -> bool {
        if !self.vacation_mode {
            return false;
        }

        if let Some(until) = self.vacation_until {
            let today = academic_now().date_naive();
            if today > until {
                self.vacation_mode = false;
                self.vacation_until = None;
                return false;
            }
        }

        true
    }

    /// Enables or disables vacation mode with an optional auto-expiration date.
    pub fn set_vacation(&mut self, enabled: bool, until: Option<NaiveDate>) {
        self.vacation_mode = enabled;
        self.vacation_until = if enabled { until } else { None };
    }

    /// Enables or disables 6:00 AM morning briefings.
    pub fn set_morning_alerts(&mut self, enabled: bool) {
        self.morning_alerts = enabled;
    }

    /// Enables or disables automatic GitHub Gist sync.
    pub fn set_sync_gist(&mut self, enabled: bool) {
        self.auto_sync_gist = enabled;
    }

    /// Enables or disables automatic Google Sheets sync.
    pub fn set_sync_gsheet(&mut self, enabled: bool) {
        self.auto_sync_gsheet = enabled;
    }

    /// Loads persisted state from a file, or returns the default state if the file does not exist.
    ///
    /// # Errors
    /// Returns [`StateError`] if the file exists but cannot be read or parsed.
    pub fn load_or_default(path: &Path) -> Result<Self, StateError> {
        let mut state = if path.exists() {
            let content = fs::read_to_string(path).map_err(|e| StateError::Io {
                path: path.display().to_string(),
                message: e.to_string(),
            })?;
            serde_json::from_str(&content).map_err(|e| StateError::Json(e.to_string()))?
        } else {
            Self::default()
        };

        if state.gist_id.is_none() {
            if let Ok(gid) = std::env::var("GIST_ID") {
                let clean = gid.trim();
                if !clean.is_empty() {
                    state.gist_id = Some(clean.to_string());
                }
            }
        }

        Ok(state)
    }

    /// Persists state as formatted JSON to the specified path.
    ///
    /// # Errors
    /// Returns [`StateError`] if writing or serializing fails.
    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| StateError::Json(e.to_string()))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| StateError::Io {
                path: parent.display().to_string(),
                message: e.to_string(),
            })?;
        }

        fs::write(path, json).map_err(|e| StateError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drop_and_restore_course() {
        let mut state = RoutineState::default();

        assert!(!state.is_dropped("CSE-4101"));
        assert!(state.drop_course("cse-4101"));
        assert!(state.is_dropped("CSE-4101"));
        assert!(state.is_dropped("cse-4101 "));
        assert!(state.is_dropped("CSE 4101")); // spaces match
        assert!(state.is_dropped("cse4101")); // no separator matches

        // Dropping again returns false
        assert!(!state.drop_course("CSE-4101"));
        assert!(!state.drop_course("CSE 4101"));

        // Restore using space format
        assert!(state.restore_course("cse 4101"));
        assert!(!state.is_dropped("CSE-4101"));
        assert!(!state.is_dropped("cse 4101"));
    }

    #[test]
    fn test_complete_and_resume_aliases() {
        let mut state = RoutineState::default();
        assert!(!state.is_completed("CSE-4125"));
        assert!(state.complete_course("CSE-4125"));
        assert!(state.is_completed("CSE-4125"));
        assert!(state.is_dropped("CSE-4125"));

        assert!(state.resume_course("CSE-4125"));
        assert!(!state.is_completed("CSE-4125"));
    }

    #[test]
    fn test_vacation_mode_lifecycle_and_auto_expiration() {
        let mut state = RoutineState::default();
        assert!(!state.is_vacation_active());

        // Enable without expiration
        state.set_vacation(true, None);
        assert!(state.is_vacation_active());

        // Enable with future expiration
        state.set_vacation(true, Some(NaiveDate::from_ymd_opt(2099, 1, 1).unwrap()));
        assert!(state.is_vacation_active());

        // Past expiration should auto-expire
        state.set_vacation(true, Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()));
        assert!(!state.is_vacation_active());
        assert!(!state.vacation_mode);
        assert_eq!(state.vacation_until, None);
    }

    #[test]
    fn test_backward_compatibility_deserialization() {
        // Old routine_state.json format without new fields
        let legacy_json = r#"{
            "semester_name": "Summer 2024",
            "semester_end_date": null,
            "gist_id": "4a7e24f1f95a405673ae900e9838f3fd",
            "dropped_courses": ["CSE-4125"]
        }"#;

        let state: RoutineState = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(state.semester_name, "Summer 2024");
        assert!(state.is_completed("CSE-4125"));
        assert!(state.morning_alerts);
        assert!(!state.vacation_mode);
        assert!(state.auto_sync_gist);
        assert!(state.auto_sync_gsheet);
        assert_eq!(state.telegram_username, None);
    }

    #[test]
    fn test_save_and_load_state() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_file = temp_dir.path().join("state.json");

        let mut original = RoutineState {
            semester_name: "2026 L4T1".to_string(),
            semester_end_date: Some(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()),
            gist_id: Some("gist123".to_string()),
            dropped_courses: HashSet::new(),
            morning_alerts: false,
            vacation_mode: true,
            vacation_until: Some(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()),
            auto_sync_gist: false,
            auto_sync_gsheet: false,
            telegram_username: Some("@testuser".to_string()),
        };
        original.complete_course("CSE-4102");

        original.save(&state_file).unwrap();

        let loaded = RoutineState::load_or_default(&state_file).unwrap();
        assert_eq!(loaded.semester_name, "2026 L4T1");
        assert_eq!(
            loaded.semester_end_date,
            Some(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap())
        );
        assert_eq!(loaded.gist_id, Some("gist123".to_string()));
        assert!(loaded.is_completed("CSE-4102"));
        assert!(!loaded.morning_alerts);
        assert!(loaded.vacation_mode);
        assert_eq!(loaded.telegram_username, Some("@testuser".to_string()));
    }
}
