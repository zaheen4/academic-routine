//! Persistent state management for course dropping and semester lifecycles.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

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
}

impl Default for RoutineState {
    fn default() -> Self {
        Self {
            semester_name: "Level 4 Term 1".to_string(),
            semester_end_date: None,
            gist_id: None,
            dropped_courses: HashSet::new(),
        }
    }
}

impl RoutineState {
    /// Drops a course by its code, marking it to be hidden from calendar and queries.
    ///
    /// Returns `true` if the course was newly added to the dropped set.
    pub fn drop_course(&mut self, code: &str) -> bool {
        self.dropped_courses.insert(code.trim().to_uppercase())
    }

    /// Restores a previously dropped course.
    ///
    /// Returns `true` if the course was present and removed from the dropped set.
    pub fn restore_course(&mut self, code: &str) -> bool {
        self.dropped_courses.remove(&code.trim().to_uppercase())
    }

    /// Checks if a course is currently marked as dropped.
    #[must_use]
    pub fn is_dropped(&self, code: &str) -> bool {
        self.dropped_courses.contains(&code.trim().to_uppercase())
    }

    /// Loads persisted state from a file, or returns the default state if the file does not exist.
    ///
    /// # Errors
    /// Returns [`StateError`] if the file exists but cannot be read or parsed.
    pub fn load_or_default(path: &Path) -> Result<Self, StateError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| StateError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;

        serde_json::from_str(&content).map_err(|e| StateError::Json(e.to_string()))
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

        // Dropping again returns false
        assert!(!state.drop_course("CSE-4101"));

        // Restore
        assert!(state.restore_course("cse-4101"));
        assert!(!state.is_dropped("CSE-4101"));
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
        };
        original.drop_course("CSE-4102");

        original.save(&state_file).unwrap();

        let loaded = RoutineState::load_or_default(&state_file).unwrap();
        assert_eq!(loaded.semester_name, "2026 L4T1");
        assert_eq!(
            loaded.semester_end_date,
            Some(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap())
        );
        assert_eq!(loaded.gist_id, Some("gist123".to_string()));
        assert!(loaded.is_dropped("CSE-4102"));
    }
}
