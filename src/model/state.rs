//! Persistent state management for course dropping, attendance, and semester lifecycles.
//!
//! State is private to the host. It is never uploaded to the public Gist bundle.
//! Writes are atomic (tmp file + fsync + rename) and the previous version is kept
//! as a `.bak` file so a corrupt state file can be recovered instead of silently
//! resetting every user's data.

use crate::model::day::academic_now;
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Current on-disk state schema version.
pub const STATE_SCHEMA_VERSION: u32 = 2;

fn default_true() -> bool {
    true
}

fn default_schema_version() -> u32 {
    1
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

    /// The file exists but could not be parsed, and recovery failed.
    #[error("State file '{path}' is corrupt: {detail}")]
    Corrupt {
        /// Path of the corrupt file.
        path: String,
        /// Optional quarantine path, if the file could be moved aside.
        quarantine: Option<String>,
        /// Parser or recovery detail.
        detail: String,
    },

    /// The state file was written by a newer version of the application.
    #[error("State file schema version {found} is newer than supported version {supported}")]
    UnsupportedVersion {
        /// Version found in the file.
        found: u32,
        /// Highest version this binary understands.
        supported: u32,
    },
}

/// Sanitized metadata that is safe to publish in the public Gist bundle.
///
/// This deliberately contains no chat IDs, usernames, attendance, mutes,
/// reminder sets, or Gist credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicRoutineMeta {
    /// On-disk/routine schema version.
    pub schema_version: u32,
    /// Name or descriptor of the current semester.
    pub semester_name: String,
    /// Explicit final day of the semester (sets the RFC 5545 `UNTIL` rule).
    pub semester_end_date: Option<NaiveDate>,
    /// Owner/global dropped course codes, sorted for stable diffs.
    pub owner_dropped_courses: Vec<String>,
    /// RFC 3339 timestamp of when this projection was produced.
    pub updated_at: String,
}

/// User state tracking dropped/hidden courses, attendance, and notification preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutineState {
    /// Version of this persisted schema.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Name or descriptor of the current semester.
    pub semester_name: String,
    /// Explicit final day of the semester (sets the RFC 5545 `UNTIL` rule).
    pub semester_end_date: Option<NaiveDate>,
    /// Optional GitHub Gist ID used for automatic live calendar syncing.
    #[serde(default)]
    pub gist_id: Option<String>,
    /// Owner/global set of uppercase course codes excluded from calendars and CLI queries.
    ///
    /// Bot users additionally have per-chat drops in [`Self::user_dropped_courses`].
    #[serde(default)]
    pub dropped_courses: BTreeSet<String>,

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

    /// Deprecated global Telegram handle. Migrated into [`Self::owner_username`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_username: Option<String>,

    /// Display name/handle of the owner (CLI status only; never shown to other users).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_username: Option<String>,

    /// Map of Telegram Chat ID -> preferred section ("B1", "B2", or "ALL").
    #[serde(default)]
    pub user_sections: BTreeMap<i64, String>,

    /// Map of Telegram Chat ID -> (Course Code -> AttendanceRecord).
    #[serde(default)]
    pub user_attendance: BTreeMap<i64, BTreeMap<String, AttendanceRecord>>,

    /// Map of Telegram Chat ID -> additional courses hidden only for that chat.
    #[serde(default)]
    pub user_dropped_courses: BTreeMap<i64, BTreeSet<String>>,

    /// Map of Telegram Chat ID -> cached display name/handle.
    #[serde(default)]
    pub user_names: BTreeMap<i64, String>,

    /// Every chat that has interacted with the bot, including `/start`-only users.
    #[serde(default)]
    pub registered_chats: BTreeSet<i64>,

    /// Chats that have explicitly muted 6:00 AM daily morning briefings.
    #[serde(default)]
    pub muted_morning_alerts: BTreeSet<i64>,

    /// Chats that have explicitly opted into 30-minute pre-class reminders.
    #[serde(default)]
    pub user_preclass_reminders: BTreeSet<i64>,

    /// Chat ID -> set of reminder keys already delivered.
    ///
    /// Key format: `YYYY-MM-DD|CODE|HHMM`. Persisted so a restart does not
    /// duplicate a reminder and a slow loop does not miss one.
    #[serde(default)]
    pub reminders_sent: BTreeMap<i64, BTreeSet<String>>,

    /// Date of the last successfully delivered 06:00 morning briefing.
    #[serde(default)]
    pub last_morning_briefing: Option<NaiveDate>,
}

impl Default for RoutineState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            semester_name: "Level 4 Term 1".to_string(),
            semester_end_date: None,
            gist_id: None,
            dropped_courses: BTreeSet::new(),
            morning_alerts: true,
            vacation_mode: false,
            vacation_until: None,
            auto_sync_gist: true,
            auto_sync_gsheet: true,
            telegram_username: None,
            owner_username: None,
            user_sections: BTreeMap::new(),
            user_attendance: BTreeMap::new(),
            user_dropped_courses: BTreeMap::new(),
            user_names: BTreeMap::new(),
            registered_chats: BTreeSet::new(),
            muted_morning_alerts: BTreeSet::new(),
            user_preclass_reminders: BTreeSet::new(),
            reminders_sent: BTreeMap::new(),
            last_morning_briefing: None,
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
    /// Drops or marks a course as completed for the owner/global schedule.
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

    /// Restores or resumes an active schedule for a globally dropped course.
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

    /// Drops a course only for one chat.
    pub fn drop_course_for(&mut self, chat_id: i64, code: &str) -> bool {
        let clean = code.trim().to_uppercase();
        if clean.is_empty() {
            return false;
        }
        let set = self.user_dropped_courses.entry(chat_id).or_default();
        if set.iter().any(|d| codes_match(d, &clean)) {
            return false;
        }
        set.insert(clean)
    }

    /// Resumes a course only for one chat.
    pub fn resume_course_for(&mut self, chat_id: i64, code: &str) -> bool {
        let Some(set) = self.user_dropped_courses.get_mut(&chat_id) else {
            return false;
        };
        let to_remove: Vec<String> = set
            .iter()
            .filter(|d| codes_match(d, code))
            .cloned()
            .collect();
        if to_remove.is_empty() {
            return false;
        }
        for r in to_remove {
            set.remove(&r);
        }
        if set.is_empty() {
            self.user_dropped_courses.remove(&chat_id);
        }
        true
    }

    /// Checks if a course is currently marked as dropped or completed globally.
    #[must_use]
    pub fn is_dropped(&self, code: &str) -> bool {
        self.dropped_courses.iter().any(|d| codes_match(d, code))
    }

    /// Checks if a course is marked as completed globally. Alias for [`is_dropped`].
    #[must_use]
    pub fn is_completed(&self, code: &str) -> bool {
        self.is_dropped(code)
    }

    /// Checks if a course is dropped for a specific chat (global or per-chat).
    #[must_use]
    pub fn is_dropped_for(&self, chat_id: i64, code: &str) -> bool {
        self.is_dropped(code)
            || self
                .user_dropped_courses
                .get(&chat_id)
                .is_some_and(|set| set.iter().any(|d| codes_match(d, code)))
    }

    /// Returns the effective dropped-course set for a chat (global union per-chat).
    #[must_use]
    pub fn dropped_for(&self, chat_id: i64) -> BTreeSet<String> {
        let mut set = self.dropped_courses.clone();
        if let Some(user_set) = self.user_dropped_courses.get(&chat_id) {
            set.extend(user_set.iter().cloned());
        }
        set
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
        self.registered_chats.insert(chat_id);
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
        self.registered_chats.insert(chat_id);
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
        self.registered_chats.insert(chat_id);
        let user_map = self.user_attendance.entry(chat_id).or_default();
        let record = AttendanceRecord { attended, missed };
        user_map.insert(clean, record.clone());
        record
    }

    /// Resets attendance records for a specific course in a user's chat.
    pub fn reset_attendance(&mut self, chat_id: i64, code: &str) -> bool {
        if let Some(user_map) = self.user_attendance.get_mut(&chat_id) {
            let key = user_map.keys().find(|k| codes_match(k, code)).cloned();
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

    /// Records that a user has interacted with the bot.
    pub fn touch_user(&mut self, chat_id: i64, display_name: Option<&str>) {
        self.registered_chats.insert(chat_id);
        if let Some(name) = display_name.map(str::trim).filter(|n| !n.is_empty()) {
            self.user_names.insert(chat_id, name.to_string());
        }
    }

    /// Returns the cached display name for a chat.
    #[must_use]
    pub fn user_name(&self, chat_id: i64) -> Option<&str> {
        self.user_names.get(&chat_id).map(|s| s.as_str())
    }

    /// Returns true if morning briefings (06:00 AM) are enabled for a given chat.
    #[must_use]
    pub fn is_morning_alerts_enabled(&self, chat_id: i64) -> bool {
        self.morning_alerts && !self.muted_morning_alerts.contains(&chat_id)
    }

    /// Sets morning briefing preference for a given chat.
    pub fn set_user_morning_alerts(&mut self, chat_id: i64, enabled: bool) {
        self.registered_chats.insert(chat_id);
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
        self.registered_chats.insert(chat_id);
        if enabled {
            self.user_preclass_reminders.insert(chat_id);
        } else {
            self.user_preclass_reminders.remove(&chat_id);
        }
    }

    /// Returns a set of all unique chat IDs that have registered or interacted with the bot.
    #[must_use]
    pub fn all_registered_chats(&self) -> BTreeSet<i64> {
        let mut set = self.registered_chats.clone();
        set.extend(self.user_sections.keys().copied());
        set.extend(self.user_attendance.keys().copied());
        set.extend(self.user_dropped_courses.keys().copied());
        set.extend(self.user_names.keys().copied());
        set.extend(self.muted_morning_alerts.iter().copied());
        set.extend(self.user_preclass_reminders.iter().copied());
        set
    }

    /// Returns true if a reminder key was already delivered to this chat.
    #[must_use]
    pub fn reminder_was_sent(&self, chat_id: i64, key: &str) -> bool {
        self.reminders_sent
            .get(&chat_id)
            .is_some_and(|set| set.contains(key))
    }

    /// Marks a reminder key as delivered. Returns true if it was newly inserted.
    pub fn mark_reminder_sent(&mut self, chat_id: i64, key: impl Into<String>) -> bool {
        self.reminders_sent
            .entry(chat_id)
            .or_default()
            .insert(key.into())
    }

    /// Removes reminder keys older than `today` (ISO `YYYY-MM-DD` prefix).
    /// Returns true if anything was pruned.
    pub fn prune_reminders_sent(&mut self, today: NaiveDate) -> bool {
        let today_str = today.format("%Y-%m-%d").to_string();
        let mut changed = false;
        self.reminders_sent.retain(|_, set| {
            let before = set.len();
            set.retain(|key| key.as_str() >= today_str.as_str());
            changed |= set.len() != before;
            !set.is_empty()
        });
        changed
    }

    /// Returns true if the 06:00 morning briefing was already delivered on `date`.
    #[must_use]
    pub fn morning_briefing_was_sent(&self, date: NaiveDate) -> bool {
        self.last_morning_briefing == Some(date)
    }

    /// Records a successfully delivered 06:00 morning briefing.
    pub fn mark_morning_briefing_sent(&mut self, date: NaiveDate) {
        self.last_morning_briefing = Some(date);
    }

    /// Imports attendance records from a UCAM attendance JSON file for a given chat_id.
    pub fn import_ucam_attendance_file(
        &mut self,
        chat_id: i64,
        path: &Path,
    ) -> Result<usize, String> {
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

        let parsed: UcamAttendanceFile =
            serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let count = parsed.records.len();
        for item in parsed.records {
            self.set_attendance(chat_id, &item.course_code, item.attended, item.missed);
        }
        Ok(count)
    }

    /// Gets an individual course attendance record for a user.
    #[must_use]
    pub fn get_attendance(&self, chat_id: i64, code: &str) -> Option<&AttendanceRecord> {
        self.user_attendance
            .get(&chat_id)
            .and_then(|m| m.iter().find(|(k, _)| codes_match(k, code)).map(|(_, v)| v))
    }

    /// Gets all course attendance records for a user.
    #[must_use]
    pub fn get_all_attendance(&self, chat_id: i64) -> Option<&BTreeMap<String, AttendanceRecord>> {
        self.user_attendance.get(&chat_id)
    }

    /// Builds the sanitized public projection for the Gist bundle.
    #[must_use]
    pub fn public_meta(&self) -> PublicRoutineMeta {
        PublicRoutineMeta {
            schema_version: STATE_SCHEMA_VERSION,
            semester_name: self.semester_name.clone(),
            semester_end_date: self.semester_end_date,
            owner_dropped_courses: self.dropped_courses.iter().cloned().collect(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    /// Applies owner/global fields from the public Gist metadata.
    ///
    /// Per-chat state (sections, attendance, personal drops, mutes, reminders,
    /// names) is never touched by a remote refresh.
    pub fn apply_public_meta(&mut self, meta: &PublicRoutineMeta) {
        self.semester_name = meta.semester_name.clone();
        self.semester_end_date = meta.semester_end_date;
        self.dropped_courses = meta.owner_dropped_courses.iter().cloned().collect();
    }

    /// Loads persisted state from a file. Returns `None` if the file does not exist.
    ///
    /// This is strict: a corrupt file is an error, not a silent default.
    ///
    /// # Errors
    /// Returns [`StateError`] if the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Option<Self>, StateError> {
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path).map_err(|e| StateError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        let mut state: Self =
            serde_json::from_str(&content).map_err(|e| StateError::Json(e.to_string()))?;
        state.migrate()?;
        state.apply_env_defaults();
        let _ = state.clean_expired_vacation();
        Ok(Some(state))
    }

    /// Opens the state, recovering a corrupt file from `.bak` when possible.
    ///
    /// A corrupt file is renamed to `routine_state.json.corrupt-<timestamp>` so
    /// its bytes are preserved. Returns a human-readable recovery note, if any.
    ///
    /// # Errors
    /// Returns [`StateError`] if the corrupt file cannot be moved aside, or if a
    /// newer schema version is encountered.
    pub fn open_or_recover(path: &Path) -> Result<(Self, Option<String>), StateError> {
        match Self::load(path) {
            Ok(Some(state)) => Ok((state, None)),
            Ok(None) => {
                // No main file: try the backup from an interrupted save.
                let backup = backup_path(path);
                if backup.exists() {
                    if let Ok(Some(state)) = Self::load(&backup) {
                        return Ok((
                            state,
                            Some(format!(
                                "State file was missing; restored from backup '{}'.",
                                backup.display()
                            )),
                        ));
                    }
                }
                let mut state = Self::default();
                state.apply_env_defaults();
                Ok((state, None))
            }
            Err(error) => {
                let quarantine = quarantine_path(path);
                if let Err(rename_error) = fs::rename(path, &quarantine) {
                    return Err(StateError::Corrupt {
                        path: path.display().to_string(),
                        quarantine: None,
                        detail: format!(
                            "{error}; additionally failed to quarantine the file: {rename_error}"
                        ),
                    });
                }

                let backup = backup_path(path);
                if backup.exists() {
                    if let Ok(Some(state)) = Self::load(&backup) {
                        return Ok((
                            state,
                            Some(format!(
                                "State file was corrupt ({error}); quarantined at '{}' and restored from '{}'.",
                                quarantine.display(),
                                backup.display()
                            )),
                        ));
                    }
                }

                let mut state = Self::default();
                state.apply_env_defaults();
                Ok((
                    state,
                    Some(format!(
                        "State file was corrupt ({error}); quarantined at '{}' and started from defaults. Previous state is preserved there.",
                        quarantine.display()
                    )),
                ))
            }
        }
    }

    /// Persists state as formatted JSON atomically.
    ///
    /// Writes a mode-0600 temp file, fsyncs it, moves the previous version to
    /// `.bak`, then renames the temp file into place.
    ///
    /// # Errors
    /// Returns [`StateError`] if writing, syncing, or renaming fails.
    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let json =
            serde_json::to_string_pretty(self).map_err(|e| StateError::Json(e.to_string()))?;

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| StateError::Io {
            path: parent.display().to_string(),
            message: e.to_string(),
        })?;

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("routine_state.json");
        let nonce = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp_path = parent.join(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()));
        let backup = backup_path(path);

        {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp_path).map_err(|e| StateError::Io {
                path: tmp_path.display().to_string(),
                message: e.to_string(),
            })?;
            file.write_all(json.as_bytes())
                .map_err(|e| StateError::Io {
                    path: tmp_path.display().to_string(),
                    message: e.to_string(),
                })?;
            file.sync_all().map_err(|e| StateError::Io {
                path: tmp_path.display().to_string(),
                message: e.to_string(),
            })?;
        }

        if path.exists() {
            // Best-effort backup; tolerate ENOENT if removed concurrently.
            let _ = fs::copy(path, &backup);
        }

        if let Err(error) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(StateError::Io {
                path: path.display().to_string(),
                message: error.to_string(),
            });
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    fn apply_env_defaults(&mut self) {
        if self.gist_id.is_none() {
            if let Ok(gid) = std::env::var("GIST_ID") {
                let clean = gid.trim();
                if !clean.is_empty() {
                    self.gist_id = Some(clean.to_string());
                }
            }
        }
    }

    fn migrate(&mut self) -> Result<(), StateError> {
        if self.schema_version > STATE_SCHEMA_VERSION {
            return Err(StateError::UnsupportedVersion {
                found: self.schema_version,
                supported: STATE_SCHEMA_VERSION,
            });
        }

        if self.schema_version < STATE_SCHEMA_VERSION {
            self.schema_version = STATE_SCHEMA_VERSION;
        }

        // Migrate the old single global handle to an owner-only field.
        if let Some(name) = self.telegram_username.take() {
            if self.owner_username.is_none() {
                self.owner_username = Some(name);
            }
        }

        // Backfill registrations from all legacy per-chat maps.
        self.registered_chats
            .extend(self.user_sections.keys().copied());
        self.registered_chats
            .extend(self.user_attendance.keys().copied());
        self.registered_chats
            .extend(self.user_dropped_courses.keys().copied());
        self.registered_chats
            .extend(self.user_names.keys().copied());
        self.registered_chats
            .extend(self.muted_morning_alerts.iter().copied());
        self.registered_chats
            .extend(self.user_preclass_reminders.iter().copied());

        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("routine_state.json");
    path.with_file_name(format!("{file_name}.bak"))
}

fn quarantine_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("routine_state.json");
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    path.with_file_name(format!("{file_name}.corrupt-{timestamp}"))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FileStamp {
    mtime: Option<std::time::SystemTime>,
    len: u64,
}

impl FileStamp {
    fn from_path(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            mtime: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

#[derive(Debug)]
struct StoreInner {
    state: RoutineState,
    stamp: Option<FileStamp>,
}

impl StoreInner {
    fn refresh(&mut self, path: &Path) {
        let current_stamp = FileStamp::from_path(path);
        if current_stamp.is_some() && current_stamp == self.stamp {
            return;
        }

        match RoutineState::load(path) {
            Ok(Some(fresh)) => {
                self.state = fresh;
                self.stamp = current_stamp;
            }
            Ok(None) => {
                // File was removed externally; keep in-memory copy, clear stamp
                self.stamp = None;
            }
            Err(e) => {
                eprintln!("⚠️ state refresh skipped: {e}");
            }
        }
    }
}

/// Thread-safe owner of the in-memory state and its on-disk path.
///
/// Handlers mutate through [`StateStore::update`], which holds the lock across
/// the mutation and the atomic save, eliminating lost updates. Never call back
/// into the store from inside an `update` closure.
#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
    inner: Arc<Mutex<StoreInner>>,
    recovery_note: Arc<Mutex<Option<String>>>,
}

impl StateStore {
    /// Opens the state file, recovering from a corrupt file if needed.
    ///
    /// # Errors
    /// Returns [`StateError`] if the file is unreadable, has a newer schema, or
    /// cannot be quarantined.
    pub fn open(path: PathBuf) -> Result<(Self, Option<String>), StateError> {
        let existed = path.exists();
        let (mut state, note) = RoutineState::open_or_recover(&path)?;
        let cleaned = state.clean_expired_vacation();
        let initial_stamp = FileStamp::from_path(&path);
        let store = Self {
            path,
            inner: Arc::new(Mutex::new(StoreInner {
                state,
                stamp: initial_stamp,
            })),
            recovery_note: Arc::new(Mutex::new(note.clone())),
        };
        if existed || cleaned {
            // Persist the migration/vacation cleanup so the next process sees it.
            if let Err(error) = store.save_current() {
                eprintln!("⚠️ Could not persist migrated state: {error}");
            }
            // Set the fingerprint from the file AFTER that save
            if let Ok(mut guard) = store.inner.lock() {
                guard.stamp = FileStamp::from_path(&store.path);
            }
        }
        Ok((store, note))
    }

    fn save_current(&self) -> Result<(), StateError> {
        let mut guard = self.lock();
        guard.state.save(&self.path)?;
        guard.stamp = FileStamp::from_path(&self.path);
        Ok(())
    }

    /// Returns a snapshot of the current state.
    #[must_use]
    pub fn read(&self) -> RoutineState {
        let mut guard = self.lock();
        guard.refresh(&self.path);
        guard.state.clone()
    }

    /// Applies a mutation and persists the result atomically.
    ///
    /// # Errors
    /// Returns [`StateError`] if the save fails. The in-memory mutation remains
    /// applied, so callers should report the error to the user/admin.
    pub fn update<R>(&self, mutate: impl FnOnce(&mut RoutineState) -> R) -> Result<R, StateError> {
        let mut guard = self.lock();
        guard.refresh(&self.path);
        let result = mutate(&mut guard.state);
        guard.state.save(&self.path)?;
        guard.stamp = FileStamp::from_path(&self.path);
        Ok(result)
    }

    /// Applies a mutation without saving (for read-only projections).
    pub fn with_state<R>(&self, inspect: impl FnOnce(&RoutineState) -> R) -> R {
        let mut guard = self.lock();
        guard.refresh(&self.path);
        inspect(&guard.state)
    }

    /// Replaces the entire state and persists it.
    ///
    /// # Errors
    /// Returns [`StateError`] if the save fails.
    pub fn replace(&self, state: RoutineState) -> Result<(), StateError> {
        self.update(|s| *s = state)
    }

    /// Returns the state file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the recovery note from startup, if any.
    #[must_use]
    pub fn recovery_note(&self) -> Option<String> {
        self.recovery_note
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn lock(&self) -> MutexGuard<'_, StoreInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Returns the next occurrence of `hh:mm` in the academic timezone.
///
/// Kept here so scheduler code can share a single, testable clock helper.
#[must_use]
pub fn next_daily_time(hour: u32, minute: u32) -> chrono::DateTime<chrono::FixedOffset> {
    let now = academic_now();
    let target_time = chrono::NaiveTime::from_hms_opt(hour, minute, 0)
        .expect("scheduler time is a valid clock time");
    let target_date = if now.time() < target_time {
        now.date_naive()
    } else {
        now.date_naive() + chrono::Duration::days(1)
    };
    let naive = target_date.and_time(target_time);
    let offset = *now.offset();
    naive
        .and_local_timezone(offset)
        .single()
        .unwrap_or_else(|| now + chrono::Duration::days(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("routine_state.json")
    }

    #[test]
    fn test_drop_and_restore_course() {
        let mut state = RoutineState::default();

        assert!(!state.is_dropped("CSE-4101"));
        assert!(state.drop_course("cse-4101"));
        assert!(state.is_dropped("CSE-4101"));
        assert!(state.is_dropped("cse-4101 "));
        assert!(state.is_dropped("CSE 4101"));
        assert!(state.is_dropped("cse4101"));

        assert!(!state.drop_course("CSE-4101"));
        assert!(!state.drop_course("CSE 4101"));

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
    fn test_per_chat_drops_are_isolated() {
        let mut state = RoutineState::default();
        let a = 1001;
        let b = 1002;

        assert!(state.drop_course_for(a, "CSE-4101"));
        assert!(state.is_dropped_for(a, "CSE-4101"));
        assert!(!state.is_dropped_for(b, "CSE-4101"));
        assert!(!state.is_dropped("CSE-4101"));

        assert!(state.drop_course("CSE-4103"));
        assert!(state.is_dropped_for(a, "CSE-4103"));
        assert!(state.is_dropped_for(b, "CSE-4103"));

        assert!(state.resume_course_for(a, "CSE-4101"));
        assert!(!state.is_dropped_for(a, "CSE-4101"));
        assert!(!state.resume_course_for(a, "CSE-4101"));
    }

    #[test]
    fn test_vacation_mode_lifecycle_and_auto_expiration() {
        let mut state = RoutineState::default();
        assert!(!state.is_vacation_active());

        state.set_vacation(true, None);
        assert!(state.is_vacation_active());

        state.set_vacation(true, Some(NaiveDate::from_ymd_opt(2099, 1, 1).unwrap()));
        assert!(state.is_vacation_active());

        state.set_vacation(true, Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()));
        assert!(!state.is_vacation_active());
        assert!(state.clean_expired_vacation());
        assert!(!state.vacation_mode);
        assert_eq!(state.vacation_until, None);
    }

    #[test]
    fn test_backward_compatibility_deserialization_and_migration() {
        let legacy_json = r#"{
            "semester_name": "Summer 2024",
            "semester_end_date": null,
            "gist_id": "4a7e24f1f95a405673ae900e9838f3fd",
            "dropped_courses": ["CSE-4125"],
            "telegram_username": "@legacy",
            "user_sections": {"123": "B2"}
        }"#;

        let mut state: RoutineState = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(state.schema_version, 1);
        state.migrate().unwrap();

        assert_eq!(state.schema_version, STATE_SCHEMA_VERSION);
        assert_eq!(state.semester_name, "Summer 2024");
        assert!(state.is_completed("CSE-4125"));
        assert!(state.morning_alerts);
        assert!(!state.vacation_mode);
        assert!(state.auto_sync_gist);
        assert!(state.auto_sync_gsheet);
        assert_eq!(state.telegram_username, None);
        assert_eq!(state.owner_username.as_deref(), Some("@legacy"));
        assert_eq!(
            state.user_sections.get(&123).map(String::as_str),
            Some("B2")
        );
        assert!(state.registered_chats.contains(&123));
    }

    #[test]
    fn test_save_and_load_state() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_file = temp_state_path(&temp_dir);

        let mut original = RoutineState {
            semester_name: "2026 L4T1".to_string(),
            semester_end_date: Some(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()),
            gist_id: Some("gist123".to_string()),
            morning_alerts: false,
            vacation_mode: true,
            vacation_until: Some(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()),
            auto_sync_gist: false,
            auto_sync_gsheet: false,
            owner_username: Some("@testuser".to_string()),
            ..RoutineState::default()
        };
        original.complete_course("CSE-4102");
        original.touch_user(42, Some("@alice"));
        original.drop_course_for(42, "CSE-4101");
        original.record_attendance(42, "CSE-4101", true);

        original.save(&state_file).unwrap();

        let loaded = RoutineState::load(&state_file).unwrap().unwrap();
        assert_eq!(loaded.schema_version, STATE_SCHEMA_VERSION);
        assert_eq!(loaded.semester_name, "2026 L4T1");
        assert_eq!(
            loaded.semester_end_date,
            Some(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap())
        );
        assert_eq!(loaded.gist_id, Some("gist123".to_string()));
        assert!(loaded.is_completed("CSE-4102"));
        assert!(!loaded.morning_alerts);
        assert!(loaded.vacation_mode);
        assert_eq!(loaded.owner_username.as_deref(), Some("@testuser"));
        assert!(loaded.is_dropped_for(42, "CSE-4101"));
        assert_eq!(
            loaded.get_attendance(42, "CSE-4101").map(|r| r.attended),
            Some(1)
        );
    }

    #[test]
    fn test_atomic_save_backup_and_permissions() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_file = temp_state_path(&temp_dir);

        let mut state = RoutineState {
            semester_name: "first".to_string(),
            ..Default::default()
        };
        state.save(&state_file).unwrap();
        state.semester_name = "second".to_string();
        state.save(&state_file).unwrap();

        assert!(backup_path(&state_file).exists());
        let loaded = RoutineState::load(&state_file).unwrap().unwrap();
        assert_eq!(loaded.semester_name, "second");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&state_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn test_corrupt_state_is_quarantined_and_backup_recovered() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state_file = temp_state_path(&temp_dir);

        let mut state = RoutineState {
            semester_name: "known-good".to_string(),
            ..Default::default()
        };
        state.save(&state_file).unwrap();
        // Second save creates the `.bak` of the first version.
        state.semester_name = "current".to_string();
        state.save(&state_file).unwrap();

        fs::write(&state_file, "{ truncated").unwrap();
        let (recovered, note) = RoutineState::open_or_recover(&state_file).unwrap();
        assert!(note.is_some());
        assert_eq!(recovered.semester_name, "known-good");

        let quarantined: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1);
        assert!(fs::read_to_string(quarantined[0].path())
            .unwrap()
            .contains("truncated"));
    }

    #[test]
    fn test_deterministic_serialization() {
        let mut state = RoutineState::default();
        state.complete_course("CSE-4101");
        state.complete_course("CSE-4103");
        state.touch_user(2, Some("b"));
        state.touch_user(1, Some("a"));

        let first = serde_json::to_string(&state).unwrap();
        let second = serde_json::to_string(&state).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn test_reminder_idempotence_and_pruning() {
        let mut state = RoutineState::default();
        let chat = 7;
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let old = "2026-10-24|CSE-4101|0915";
        let current = "2026-10-25|CSE-4101|0915";

        state.mark_reminder_sent(chat, old);
        assert!(state.reminder_was_sent(chat, old));
        assert!(!state.mark_reminder_sent(chat, old));

        assert!(state.mark_reminder_sent(chat, current));
        assert!(state.prune_reminders_sent(today));
        assert!(!state.reminder_was_sent(chat, old));
        assert!(state.reminder_was_sent(chat, current));
    }

    #[test]
    fn test_morning_briefing_idempotence() {
        let mut state = RoutineState::default();
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        assert!(!state.morning_briefing_was_sent(today));
        state.mark_morning_briefing_sent(today);
        assert!(state.morning_briefing_was_sent(today));
    }

    #[test]
    fn test_public_meta_contains_no_pii() {
        let mut state = RoutineState::default();
        state.touch_user(123456789, Some("@alice"));
        state.record_attendance(123456789, "CSE-4101", true);
        state.drop_course_for(123456789, "CSE-4103");
        state.set_user_preclass_reminders(123456789, true);
        state.complete_course("CSE-4101");

        let meta = state.public_meta();
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("CSE-4101"));
        for forbidden in [
            "123456789",
            "@alice",
            "user_attendance",
            "user_sections",
            "muted_morning_alerts",
            "user_preclass_reminders",
            "reminders_sent",
        ] {
            assert!(
                !json.contains(forbidden),
                "public meta leaked forbidden field/value: {forbidden}"
            );
        }
    }

    #[test]
    fn test_touch_user_registers_start_only_chat() {
        let mut state = RoutineState::default();
        state.touch_user(55, Some("@bob"));
        assert!(state.all_registered_chats().contains(&55));
        assert_eq!(state.user_name(55), Some("@bob"));
    }

    #[test]
    fn test_attendance_status_and_bunk_math() {
        let empty = AttendanceRecord::default();
        assert_eq!(empty.total(), 0);
        assert_eq!(empty.percentage(), 100.0);
        assert_eq!(empty.status(), AttendanceStatus::Collegiate);

        let rec1 = AttendanceRecord {
            attended: 15,
            missed: 2,
        };
        assert_eq!(rec1.status(), AttendanceStatus::Collegiate);
        assert_eq!(rec1.safe_bunks(), 3);
        assert_eq!(rec1.needed_for_collegiate(), 0);

        let rec2 = AttendanceRecord {
            attended: 15,
            missed: 5,
        };
        assert_eq!(rec2.status(), AttendanceStatus::Collegiate);
        assert_eq!(rec2.safe_bunks(), 0);
        assert_eq!(rec2.needed_for_collegiate(), 0);

        let rec3 = AttendanceRecord {
            attended: 7,
            missed: 3,
        };
        assert_eq!(rec3.status(), AttendanceStatus::NonCollegiate);
        assert_eq!(rec3.safe_bunks(), 0);
        assert_eq!(rec3.needed_for_collegiate(), 2);

        let rec4 = AttendanceRecord {
            attended: 2,
            missed: 2,
        };
        assert_eq!(rec4.status(), AttendanceStatus::DisCollegiate);
        assert_eq!(rec4.safe_bunks(), 0);
        assert_eq!(rec4.needed_for_collegiate(), 4);
        assert_eq!(rec4.needed_for_non_collegiate(), 1);
    }

    #[test]
    fn test_user_section_and_attendance_state_lifecycle() {
        let mut state = RoutineState::default();
        let user_a = 12345;
        let user_b = 67890;

        state.set_user_section(user_a, "b1");
        state.set_user_section(user_b, "B2");
        assert_eq!(state.get_user_section(user_a), Some("B1"));
        assert_eq!(state.get_user_section(user_b), Some("B2"));

        state.record_attendance(user_a, "CSE-4101", true);
        state.record_attendance(user_a, "CSE-4101", true);
        state.record_attendance(user_a, "CSE-4101", false);
        let rec_a = state.get_attendance(user_a, "cse-4101").unwrap();
        assert_eq!(rec_a.attended, 2);
        assert_eq!(rec_a.missed, 1);

        assert!(state.get_attendance(user_b, "CSE-4101").is_none());

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

        assert!(state.is_morning_alerts_enabled(user_a));
        assert!(state.is_morning_alerts_enabled(user_b));
        assert!(!state.is_preclass_reminders_enabled(user_a));
        assert!(!state.is_preclass_reminders_enabled(user_b));

        state.set_user_preclass_reminders(user_a, true);
        assert!(state.is_preclass_reminders_enabled(user_a));
        assert!(!state.is_preclass_reminders_enabled(user_b));

        state.set_user_morning_alerts(user_b, false);
        assert!(state.is_morning_alerts_enabled(user_a));
        assert!(!state.is_morning_alerts_enabled(user_b));

        let chats = state.all_registered_chats();
        assert!(chats.contains(&user_a));
        assert!(chats.contains(&user_b));
    }

    #[test]
    fn test_state_store_serializes_updates() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_state_path(&temp_dir);
        let (store, note) = StateStore::open(path.clone()).unwrap();
        assert!(note.is_none());

        for _ in 0..25 {
            store
                .update(|state| {
                    state.record_attendance(1, "CSE-4101", true);
                })
                .unwrap();
        }

        let loaded = RoutineState::load(&path).unwrap().unwrap();
        assert_eq!(
            loaded.get_attendance(1, "CSE-4101").map(|r| r.attended),
            Some(25)
        );
    }

    #[test]
    fn test_external_write_survives_later_update() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_state_path(&temp_dir);
        let (store_a, _) = StateStore::open(path.clone()).unwrap();
        let (store_b, _) = StateStore::open(path.clone()).unwrap();

        store_a
            .update(|s| s.set_attendance(100, "CSE-4101", 5, 1))
            .unwrap();
        store_b.update(|s| s.set_vacation(true, None)).unwrap();
        store_a.update(|s| s.set_morning_alerts(false)).unwrap();

        let final_state = store_b.read();
        assert_eq!(
            final_state
                .get_attendance(100, "CSE-4101")
                .map(|r| r.attended),
            Some(5)
        );
        assert!(final_state.is_vacation_active());
        assert!(!final_state.morning_alerts);
    }

    #[test]
    fn test_read_reflects_external_write() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_state_path(&temp_dir);
        let (store_a, _) = StateStore::open(path.clone()).unwrap();
        let (store_b, _) = StateStore::open(path.clone()).unwrap();

        store_a.update(|s| s.drop_course("CSE-4101")).unwrap();
        assert!(store_b.read().is_dropped("CSE-4101"));
    }

    #[test]
    fn test_refresh_error_keeps_memory_and_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_state_path(&temp_dir);
        let (store, _) = StateStore::open(path.clone()).unwrap();
        store.update(|s| s.drop_course("CSE-4101")).unwrap();

        // Overwrite file with corrupt content externally
        fs::write(&path, "{ corrupt json").unwrap();

        // store.read() should log warning and keep in-memory state
        assert!(store.read().is_dropped("CSE-4101"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ corrupt json");
    }

    #[test]
    fn test_concurrent_writers_never_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_state_path(&temp_dir);
        let (store_a, _) = StateStore::open(path.clone()).unwrap();
        let (store_b, _) = StateStore::open(path.clone()).unwrap();

        let handle_a = std::thread::spawn(move || {
            for _ in 0..50 {
                store_a
                    .update(|s| s.record_attendance(1, "CSE-4101", true))
                    .unwrap();
            }
        });

        let handle_b = std::thread::spawn(move || {
            for _ in 0..50 {
                store_b
                    .update(|s| s.record_attendance(2, "CSE-4102", true))
                    .unwrap();
            }
        });

        handle_a.join().unwrap();
        handle_b.join().unwrap();

        let loaded = RoutineState::load(&path).unwrap();
        assert!(loaded.is_some(), "final file must parse successfully");
    }
}
