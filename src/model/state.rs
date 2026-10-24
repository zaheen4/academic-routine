//! Persistent state management for course dropping and semester lifecycles.

use crate::model::day::academic_now;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

fn default_true() -> bool {
    true
}

/// Official university attendance eligibility status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttendanceStatus {
    /// Attendance >= 75%: Eligible for semester final exams without fine.
    Collegiate,
    /// 60% <= Attendance < 75%: Eligible with fine/penalty.
    NonCollegiate,
    /// Attendance < 60%: Disqualified from sitting for finals.
    DisCollegiate,
}

impl std::fmt::Display for AttendanceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Collegiate => write!(f, "Collegiate"),
            Self::NonCollegiate => write!(f, "Non-Collegiate"),
            Self::DisCollegiate => write!(f, "Dis-Collegiate"),
        }
    }
}

/// Class attendance record for an individual course.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AttendanceRecord {
    /// Number of classes attended.
    pub attended: u32,
    /// Number of classes missed.
    pub missed: u32,
}

impl AttendanceRecord {
    /// Total number of classes conducted.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.attended + self.missed
    }

    /// Current attendance percentage (0.0% to 100.0%).
    #[must_use]
    pub fn percentage(&self) -> f64 {
        let tot = self.total();
        if tot == 0 {
            100.0
        } else {
            (self.attended as f64 / tot as f64) * 100.0
        }
    }

    /// Evaluates current academic collegiate status.
    #[must_use]
    pub fn status(&self) -> AttendanceStatus {
        let pct = self.percentage();
        if pct >= 75.0 {
            AttendanceStatus::Collegiate
        } else if pct >= 60.0 {
            AttendanceStatus::NonCollegiate
        } else {
            AttendanceStatus::DisCollegiate
        }
    }

    /// Number of consecutive future classes the student can safely miss while remaining Collegiate (>= 75%).
    #[must_use]
    pub fn safe_bunks(&self) -> u32 {
        let a = self.attended as i64;
        let m = self.missed as i64;
        let diff = a - 3 * m;
        if diff <= 0 {
            0
        } else {
            (diff / 3) as u32
        }
    }

    /// Number of consecutive future classes needed to recover to Collegiate (>= 75%).
    #[must_use]
    pub fn needed_for_collegiate(&self) -> u32 {
        let a = self.attended as i64;
        let m = self.missed as i64;
        let diff = 3 * m - a;
        if diff <= 0 {
            0
        } else {
            diff as u32
        }
    }

    /// Number of consecutive future classes needed to recover to Non-Collegiate (>= 60%).
    #[must_use]
    pub fn needed_for_non_collegiate(&self) -> u32 {
        let a = self.attended as i64;
        let m = self.missed as i64;
        let diff = 3 * m - 2 * a;
        if diff <= 0 {
            0
        } else {
            ((diff + 1) / 2) as u32
        }
    }
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

    /// Map of Telegram Chat ID -> preferred section ("B1", "B2", or "ALL").
    #[serde(default)]
    pub user_sections: HashMap<i64, String>,

    /// Map of Telegram Chat ID -> (Course Code -> AttendanceRecord).
    #[serde(default)]
    pub user_attendance: HashMap<i64, HashMap<String, AttendanceRecord>>,

    /// Chats that have explicitly muted 6:00 AM daily morning briefings.
    #[serde(default)]
    pub muted_morning_alerts: HashSet<i64>,

    /// Chats that have explicitly opted into 30-minute pre-class reminders.
    #[serde(default)]
    pub user_preclass_reminders: HashSet<i64>,
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
            user_sections: HashMap::new(),
            user_attendance: HashMap::new(),
            muted_morning_alerts: HashSet::new(),
            user_preclass_reminders: HashSet::new(),
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
    #[must_use]
    pub fn is_vacation_active(&self) -> bool {
        if !self.vacation_mode {
            return false;
        }

        if let Some(until) = self.vacation_until {
            let today = academic_now().date_naive();
            if today > until {
                return false;
            }
        }

        true
    }

    /// Automatically clears and expires vacation mode if `vacation_until` is in the past.
    /// Returns true if the state was mutated.
    pub fn clean_expired_vacation(&mut self) -> bool {
        if self.vacation_mode {
            if let Some(until) = self.vacation_until {
                let today = academic_now().date_naive();
                if today > until {
                    self.vacation_mode = false;
                    self.vacation_until = None;
                    return true;
                }
            }
        }
        false
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

    /// Retrieves user's section preference for a given chat, if set.
    #[must_use]
    pub fn get_user_section(&self, chat_id: i64) -> Option<&str> {
        self.user_sections.get(&chat_id).map(|s| s.as_str())
    }

    /// Sets a user's section preference ("B1", "B2", or "ALL").
    pub fn set_user_section(&mut self, chat_id: i64, section: &str) {
        self.user_sections
            .insert(chat_id, section.trim().to_uppercase());
    }

    /// Clears a user's section preference, reverting to default.
    pub fn clear_user_section(&mut self, chat_id: i64) -> bool {
        self.user_sections.remove(&chat_id).is_some()
    }

    /// Records attendance (attended or missed) for a course in a user's chat.
    pub fn record_attendance(
        &mut self,
        chat_id: i64,
        code: &str,
        attended: bool,
    ) -> AttendanceRecord {
        let clean = code.trim().to_uppercase();
        let user_map = self.user_attendance.entry(chat_id).or_default();
        let record = user_map.entry(clean).or_default();
        if attended {
            record.attended += 1;
        } else {
            record.missed += 1;
        }
        record.clone()
    }

    /// Manually sets or calibrates attendance numbers for a course in a user's chat.
    pub fn set_attendance(
        &mut self,
        chat_id: i64,
        code: &str,
        attended: u32,
        missed: u32,
    ) -> AttendanceRecord {
        let clean = code.trim().to_uppercase();
        let user_map = self.user_attendance.entry(chat_id).or_default();
        let record = AttendanceRecord { attended, missed };
        user_map.insert(clean, record.clone());
        record
    }

    /// Resets attendance records for a specific course in a user's chat.
    pub fn reset_attendance(&mut self, chat_id: i64, code: &str) -> bool {
        if let Some(user_map) = self.user_attendance.get_mut(&chat_id) {
            let key = user_map
                .keys()
                .find(|k| codes_match(k, code))
                .cloned();
            if let Some(k) = key {
                return user_map.remove(&k).is_some();
            }
        }
        false
    }

    /// Resets all course attendance records for a user.
    pub fn reset_all_attendance(&mut self, chat_id: i64) {
        self.user_attendance.remove(&chat_id);
    }

    /// Returns true if morning briefings (06:00 AM) are enabled for a given chat.
    #[must_use]
    pub fn is_morning_alerts_enabled(&self, chat_id: i64) -> bool {
        self.morning_alerts && !self.muted_morning_alerts.contains(&chat_id)
    }

    /// Sets morning briefing preference for a given chat.
    pub fn set_user_morning_alerts(&mut self, chat_id: i64, enabled: bool) {
        if enabled {
            self.muted_morning_alerts.remove(&chat_id);
        } else {
            self.muted_morning_alerts.insert(chat_id);
        }
    }

    /// Returns true if 30-minute pre-class reminders are enabled for a given chat (opt-in).
    #[must_use]
    pub fn is_preclass_reminders_enabled(&self, chat_id: i64) -> bool {
        self.morning_alerts && self.user_preclass_reminders.contains(&chat_id)
    }

    /// Sets 30-minute pre-class reminder preference for a given chat.
    pub fn set_user_preclass_reminders(&mut self, chat_id: i64, enabled: bool) {
        if enabled {
            self.user_preclass_reminders.insert(chat_id);
        } else {
            self.user_preclass_reminders.remove(&chat_id);
        }
    }

    /// Returns a set of all unique chat IDs that have registered or interacted with the bot.
    #[must_use]
    pub fn all_registered_chats(&self) -> HashSet<i64> {
        let mut set: HashSet<i64> = self.user_sections.keys().copied().collect();
        for k in self.user_attendance.keys() {
            set.insert(*k);
        }
        for k in &self.user_preclass_reminders {
            set.insert(*k);
        }
        set
    }

    /// Imports attendance records from a UCAM attendance JSON file for a given chat_id.
    pub fn import_ucam_attendance_file(&mut self, chat_id: i64, path: &Path) -> Result<usize, String> {
        if !path.exists() {
            return Ok(0);
        }
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        #[derive(Deserialize)]
        struct UcamAttendanceFile {
            #[serde(default)]
            records: Vec<UcamAttendanceItem>,
        }
        #[derive(Deserialize)]
        struct UcamAttendanceItem {
            course_code: String,
            #[serde(default)]
            attended: u32,
            #[serde(default)]
            missed: u32,
        }

        let parsed: UcamAttendanceFile = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let count = parsed.records.len();
        for item in parsed.records {
            self.set_attendance(chat_id, &item.course_code, item.attended, item.missed);
        }
        Ok(count)
    }

    /// Gets an individual course attendance record for a user.
    #[must_use]
    pub fn get_attendance(&self, chat_id: i64, code: &str) -> Option<&AttendanceRecord> {
        self.user_attendance.get(&chat_id).and_then(|m| {
            m.iter()
                .find(|(k, _)| codes_match(k, code))
                .map(|(_, v)| v)
        })
    }

    /// Gets all course attendance records for a user.
    #[must_use]
    pub fn get_all_attendance(&self, chat_id: i64) -> Option<&HashMap<String, AttendanceRecord>> {
        self.user_attendance.get(&chat_id)
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

        let _ = state.clean_expired_vacation();

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

        // Past expiration should evaluate as inactive and clear when cleaned
        state.set_vacation(true, Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()));
        assert!(!state.is_vacation_active());
        assert!(state.clean_expired_vacation());
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
            user_sections: HashMap::new(),
            user_attendance: HashMap::new(),
            muted_morning_alerts: HashSet::new(),
            user_preclass_reminders: HashSet::new(),
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

    #[test]
    fn test_attendance_status_and_bunk_math() {
        // 1. Initial 0 classes: 100%, Collegiate
        let empty = AttendanceRecord::default();
        assert_eq!(empty.total(), 0);
        assert_eq!(empty.percentage(), 100.0);
        assert_eq!(empty.status(), AttendanceStatus::Collegiate);

        // 2. Collegiate case: 15 attended, 2 missed (Total 17, 88.2%)
        let rec1 = AttendanceRecord { attended: 15, missed: 2 };
        assert_eq!(rec1.status(), AttendanceStatus::Collegiate);
        // B = floor((15 - 6) / 3) = 3 safe bunks
        assert_eq!(rec1.safe_bunks(), 3);
        assert_eq!(rec1.needed_for_collegiate(), 0);

        // 3. Exactly 75% case: 15 attended, 5 missed (Total 20, 75.0%)
        let rec2 = AttendanceRecord { attended: 15, missed: 5 };
        assert_eq!(rec2.status(), AttendanceStatus::Collegiate);
        // B = floor((15 - 15) / 3) = 0 safe bunks
        assert_eq!(rec2.safe_bunks(), 0);
        assert_eq!(rec2.needed_for_collegiate(), 0);

        // 4. Non-Collegiate case: 7 attended, 3 missed (Total 10, 70.0%)
        let rec3 = AttendanceRecord { attended: 7, missed: 3 };
        assert_eq!(rec3.status(), AttendanceStatus::NonCollegiate);
        assert_eq!(rec3.safe_bunks(), 0);
        // Recovery: ceil(3*3 - 7) = 2 classes needed
        assert_eq!(rec3.needed_for_collegiate(), 2);

        // 5. Dis-Collegiate case: 2 attended, 2 missed (Total 4, 50.0%)
        let rec4 = AttendanceRecord { attended: 2, missed: 2 };
        assert_eq!(rec4.status(), AttendanceStatus::DisCollegiate);
        assert_eq!(rec4.safe_bunks(), 0);
        // Recovery to Collegiate: ceil(3*2 - 2) = 4 classes needed
        assert_eq!(rec4.needed_for_collegiate(), 4);
        // Recovery to Non-Collegiate: ceil((3*2 - 2*2)/2) = 1 class needed
        assert_eq!(rec4.needed_for_non_collegiate(), 1);
    }

    #[test]
    fn test_user_section_and_attendance_state_lifecycle() {
        let mut state = RoutineState::default();
        let user_a = 12345;
        let user_b = 67890;

        // User A picks B1, User B picks B2
        state.set_user_section(user_a, "b1");
        state.set_user_section(user_b, "B2");
        assert_eq!(state.get_user_section(user_a), Some("B1"));
        assert_eq!(state.get_user_section(user_b), Some("B2"));

        // Attendance logging per user
        state.record_attendance(user_a, "CSE-4101", true);
        state.record_attendance(user_a, "CSE-4101", true);
        state.record_attendance(user_a, "CSE-4101", false);
        let rec_a = state.get_attendance(user_a, "cse-4101").unwrap();
        assert_eq!(rec_a.attended, 2);
        assert_eq!(rec_a.missed, 1);

        // User B has no records for CSE-4101 yet
        assert!(state.get_attendance(user_b, "CSE-4101").is_none());

        // Calibrate User B directly
        state.set_attendance(user_b, "CSE-4101", 10, 1);
        let rec_b = state.get_attendance(user_b, "CSE-4101").unwrap();
        assert_eq!(rec_b.attended, 10);
        assert_eq!(rec_b.missed, 1);
        assert_eq!(rec_b.safe_bunks(), 2);
    }

    #[test]
    fn test_multi_user_alerts_and_reminders_preferences() {
        let mut state = RoutineState::default();
        let user_a: i64 = 1001;
        let user_b: i64 = 1002;

        state.set_user_section(user_a, "B1");
        state.set_user_section(user_b, "B2");

        // 1. Morning alerts enabled by default
        assert!(state.is_morning_alerts_enabled(user_a));
        assert!(state.is_morning_alerts_enabled(user_b));

        // 2. Pre-class 30-min reminders disabled by default
        assert!(!state.is_preclass_reminders_enabled(user_a));
        assert!(!state.is_preclass_reminders_enabled(user_b));

        // 3. User A opts into 30-min reminders; User B does not
        state.set_user_preclass_reminders(user_a, true);
        assert!(state.is_preclass_reminders_enabled(user_a));
        assert!(!state.is_preclass_reminders_enabled(user_b));

        // 4. User B mutes morning alerts; User A keeps them
        state.set_user_morning_alerts(user_b, false);
        assert!(state.is_morning_alerts_enabled(user_a));
        assert!(!state.is_morning_alerts_enabled(user_b));

        // 5. Check all registered chats
        let chats = state.all_registered_chats();
        assert!(chats.contains(&user_a));
        assert!(chats.contains(&user_b));
    }
}
