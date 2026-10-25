//! Academic time slot representation and normalization.
//!
//! Handles class timings, duration calculations, and formatting
//! for both 12-hour human display and RFC 5545 iCalendar serialization.
//!
//! Includes a resilient parser capable of normalizing unpadded or messy
//! UCAM portal time strings such as `"11:45 - 1:0"` or `"8:0 - 9:15"`.

use chrono::{NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Error returned when constructing or parsing a [`TimeSlot`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeSlotError {
    /// The start time occurs at or after the end time.
    #[error("Invalid time slot: start time ({start}) must be before end time ({end})")]
    InvalidOrder {
        /// The given start time.
        start: String,
        /// The given end time.
        end: String,
    },

    /// The time slot string is missing a hyphen delimiter separating start and end.
    #[error("Missing hyphen separator in time slot string '{0}'")]
    MissingDelimiter(String),

    /// A specific time token could not be parsed.
    #[error("Failed to parse time token '{token}': {reason}")]
    InvalidToken {
        /// The problematic token.
        token: String,
        /// Human-readable explanation.
        reason: String,
    },
}

/// A class period defined by a start time and an end time.
///
/// Ensures an invariant that `start < end`.
///
/// # Examples
/// ```
/// use academic_routine::TimeSlot;
/// use std::str::FromStr;
///
/// // Parses messy UCAM formats automatically
/// let slot = TimeSlot::from_str("11:45 - 1:0").unwrap();
/// assert_eq!(slot.format_12h(), "11:45 AM - 01:00 PM");
/// assert_eq!(slot.duration_minutes(), 75);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct TimeSlot {
    start: NaiveTime,
    end: NaiveTime,
}

#[derive(Deserialize)]
struct RawTimeSlot {
    start: NaiveTime,
    end: NaiveTime,
}

impl<'de> Deserialize<'de> for TimeSlot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawTimeSlot::deserialize(deserializer)?;
        Self::new(raw.start, raw.end).map_err(serde::de::Error::custom)
    }
}

impl TimeSlot {
    /// Creates a new [`TimeSlot`].
    ///
    /// # Errors
    /// Returns [`TimeSlotError::InvalidOrder`] if `start >= end`.
    pub fn new(start: NaiveTime, end: NaiveTime) -> Result<Self, TimeSlotError> {
        if start >= end {
            return Err(TimeSlotError::InvalidOrder {
                start: start.format("%H:%M").to_string(),
                end: end.format("%H:%M").to_string(),
            });
        }
        Ok(Self { start, end })
    }

    /// Creates a [`TimeSlot`] from raw hour and minute components.
    ///
    /// # Errors
    /// Returns [`TimeSlotError`] if the components form invalid times or if `start >= end`.
    pub fn from_hms(
        start_h: u32,
        start_m: u32,
        end_h: u32,
        end_m: u32,
    ) -> Result<Self, TimeSlotError> {
        let start = NaiveTime::from_hms_opt(start_h, start_m, 0).ok_or_else(|| {
            TimeSlotError::InvalidToken {
                token: format!("{start_h:02}:{start_m:02}"),
                reason: "Invalid hours or minutes".to_string(),
            }
        })?;
        let end = NaiveTime::from_hms_opt(end_h, end_m, 0).ok_or_else(|| {
            TimeSlotError::InvalidToken {
                token: format!("{end_h:02}:{end_m:02}"),
                reason: "Invalid hours or minutes".to_string(),
            }
        })?;
        Self::new(start, end)
    }

    /// Parses a raw UCAM time slot string (e.g. `"11:45 - 1:0"`, `"8:0 - 9:15"`).
    ///
    /// Handles unpadded minutes, trailing noise characters, and infers
    /// AM/PM based on typical university scheduling hours (7:00–11:59 AM, 12:00–6:00 PM).
    ///
    /// # Errors
    /// Returns [`TimeSlotError`] if the string cannot be parsed or if `start >= end`.
    pub fn parse_ucam(s: &str) -> Result<Self, TimeSlotError> {
        let trimmed = s.trim();
        let parts: Vec<&str> = trimmed.split('-').collect();
        if parts.len() != 2 {
            return Err(TimeSlotError::MissingDelimiter(s.to_string()));
        }

        let start = parse_single_time_token(parts[0])?;
        let end = parse_single_time_token(parts[1])?;

        Self::new(start, end)
    }

    /// Returns the start time.
    #[must_use]
    pub const fn start(&self) -> NaiveTime {
        self.start
    }

    /// Returns the end time.
    #[must_use]
    pub const fn end(&self) -> NaiveTime {
        self.end
    }

    /// Returns the total duration of the class in minutes.
    #[must_use]
    pub fn duration_minutes(&self) -> u32 {
        let start_mins = self.start.hour() * 60 + self.start.minute();
        let end_mins = self.end.hour() * 60 + self.end.minute();
        end_mins.saturating_sub(start_mins)
    }

    /// Formats the slot in standard 12-hour AM/PM notation (e.g. `"09:15 AM - 10:30 AM"`).
    #[must_use]
    pub fn format_12h(&self) -> String {
        format!(
            "{} - {}",
            self.start.format("%I:%M %p"),
            self.end.format("%I:%M %p")
        )
    }

    /// Formats the slot in 24-hour notation (e.g. `"09:15 - 10:30"`).
    #[must_use]
    pub fn format_24h(&self) -> String {
        format!(
            "{} - {}",
            self.start.format("%H:%M"),
            self.end.format("%H:%M")
        )
    }

    /// Returns the 6-digit start time string for RFC 5545 iCalendar (`"HHMMSS"`).
    #[must_use]
    pub fn ical_start_time(&self) -> String {
        self.start.format("%H%M%S").to_string()
    }

    /// Returns the 6-digit end time string for RFC 5545 iCalendar (`"HHMMSS"`).
    #[must_use]
    pub fn ical_end_time(&self) -> String {
        self.end.format("%H%M%S").to_string()
    }

    /// Returns `true` if this time slot overlaps with another time slot.
    ///
    /// Uses half-open intervals `[start, end)`. Adjacent slots sharing an exact
    /// boundary (e.g. 10:30 AM end and 10:30 AM start) do **not** overlap.
    ///
    /// # Examples
    /// ```
    /// use academic_routine::TimeSlot;
    /// use std::str::FromStr;
    ///
    /// let s1 = TimeSlot::from_str("09:15 - 10:30").unwrap();
    /// let s2 = TimeSlot::from_str("10:00 - 11:15").unwrap();
    /// let s3 = TimeSlot::from_str("10:30 - 11:45").unwrap();
    ///
    /// assert!(s1.overlaps_with(&s2)); // Partial clash
    /// assert!(!s1.overlaps_with(&s3)); // Back-to-back, no clash
    /// ```
    #[must_use]
    pub fn overlaps_with(&self, other: &TimeSlot) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Returns the break duration in minutes between this slot and another disjoint slot.
    ///
    /// If the two slots overlap, returns `None`.
    /// If the two slots are adjacent (back-to-back), returns `Some(0)`.
    ///
    /// # Examples
    /// ```
    /// use academic_routine::TimeSlot;
    /// use std::str::FromStr;
    ///
    /// let morning = TimeSlot::from_str("08:00 - 09:15").unwrap();
    /// let next = TimeSlot::from_str("10:30 - 11:45").unwrap();
    ///
    /// assert_eq!(morning.gap_between(&next), Some(75)); // 75-minute break
    /// ```
    #[must_use]
    pub fn gap_between(&self, other: &TimeSlot) -> Option<u32> {
        if self.overlaps_with(other) {
            return None;
        }

        if self.end <= other.start {
            let self_end_mins = self.end.hour() * 60 + self.end.minute();
            let other_start_mins = other.start.hour() * 60 + other.start.minute();
            Some(other_start_mins.saturating_sub(self_end_mins))
        } else if other.end <= self.start {
            let other_end_mins = other.end.hour() * 60 + other.end.minute();
            let self_start_mins = self.start.hour() * 60 + self.start.minute();
            Some(self_start_mins.saturating_sub(other_end_mins))
        } else {
            None
        }
    }

    /// Returns `true` if the given clock time falls within this slot: `start <= time < end`.
    #[must_use]
    pub fn contains_time(&self, time: NaiveTime) -> bool {
        self.start <= time && time < self.end
    }
}

impl FromStr for TimeSlot {
    type Err = TimeSlotError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_ucam(s)
    }
}

impl fmt::Display for TimeSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_12h())
    }
}

/// Parses an individual time token (e.g. `"11:45"`, `"1:0"`, `"09:15 AM"`, `"4:0 PM"`).
fn parse_single_time_token(token: &str) -> Result<NaiveTime, TimeSlotError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(TimeSlotError::InvalidToken {
            token: token.to_string(),
            reason: "Time token is empty".to_string(),
        });
    }

    // Check for explicit AM/PM period
    let upper = trimmed.to_uppercase();
    let (time_part, explicit_period) = if upper.ends_with("AM") {
        (upper.trim_end_matches("AM").trim(), Some("AM"))
    } else if upper.ends_with("PM") {
        (upper.trim_end_matches("PM").trim(), Some("PM"))
    } else {
        (trimmed, None)
    };

    let colon_pos = time_part
        .find(':')
        .ok_or_else(|| TimeSlotError::InvalidToken {
            token: token.to_string(),
            reason: "Missing colon ':' separator".to_string(),
        })?;

    let (hour_str, min_str) = time_part.split_at(colon_pos);
    let min_str = &min_str[1..]; // Skip ':'

    // Parse hour
    let mut hour: u32 = hour_str
        .trim()
        .parse()
        .map_err(|_| TimeSlotError::InvalidToken {
            token: token.to_string(),
            reason: format!("Invalid hour digits '{hour_str}'"),
        })?;

    // Parse minute, taking leading digits to gracefully ignore any trailing noise
    let min_digits: String = min_str.chars().take_while(|c| c.is_ascii_digit()).collect();
    if min_digits.is_empty() {
        return Err(TimeSlotError::InvalidToken {
            token: token.to_string(),
            reason: format!("Missing minute digits in '{min_str}'"),
        });
    }
    let minute: u32 = min_digits
        .parse()
        .map_err(|_| TimeSlotError::InvalidToken {
            token: token.to_string(),
            reason: format!("Invalid minute digits '{min_digits}'"),
        })?;

    if minute >= 60 {
        return Err(TimeSlotError::InvalidToken {
            token: token.to_string(),
            reason: format!("Minute must be 0-59, found {minute}"),
        });
    }

    // Determine AM/PM period
    let period = match explicit_period {
        Some(p) => p,
        None => {
            // University heuristic: 07:00-11:59 is AM; 1:00-6:59 & 12:00 are PM
            if (7..=11).contains(&hour) {
                "AM"
            } else {
                "PM"
            }
        }
    };

    // Convert to 24-hour time
    if period == "PM" && hour < 12 {
        hour += 12;
    } else if period == "AM" && hour == 12 {
        hour = 0;
    }

    NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(|| TimeSlotError::InvalidToken {
        token: token.to_string(),
        reason: format!("Calculated invalid 24h time {hour:02}:{minute:02}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_time_slot_creation() {
        let slot = TimeSlot::from_hms(9, 15, 10, 30).unwrap();
        assert_eq!(slot.duration_minutes(), 75);
        assert_eq!(slot.format_12h(), "09:15 AM - 10:30 AM");
        assert_eq!(slot.format_24h(), "09:15 - 10:30");
        assert_eq!(slot.ical_start_time(), "091500");
        assert_eq!(slot.ical_end_time(), "103000");
    }

    #[test]
    fn test_invalid_order_rejected() {
        let err = TimeSlot::from_hms(11, 0, 9, 30);
        assert!(matches!(err, Err(TimeSlotError::InvalidOrder { .. })));

        let err_equal = TimeSlot::from_hms(10, 0, 10, 0);
        assert!(matches!(err_equal, Err(TimeSlotError::InvalidOrder { .. })));
    }

    #[test]
    fn test_ucam_cross_meridiem_parsing() {
        let slot = TimeSlot::from_str("11:45 - 1:0").unwrap();
        assert_eq!(slot.format_12h(), "11:45 AM - 01:00 PM");
        assert_eq!(slot.duration_minutes(), 75);
        assert_eq!(slot.format_24h(), "11:45 - 13:00");
    }

    #[test]
    fn test_ucam_morning_unpadded_parsing() {
        let slot = TimeSlot::from_str("8:0 - 9:15").unwrap();
        assert_eq!(slot.format_12h(), "08:00 AM - 09:15 AM");
        assert_eq!(slot.duration_minutes(), 75);
    }

    #[test]
    fn test_ucam_afternoon_parsing() {
        let slot = TimeSlot::from_str("2:30 - 4:0").unwrap();
        assert_eq!(slot.format_12h(), "02:30 PM - 04:00 PM");
        assert_eq!(slot.duration_minutes(), 90);
        assert_eq!(slot.format_24h(), "14:30 - 16:00");
    }

    #[test]
    fn test_explicit_ampm_parsing() {
        let slot = TimeSlot::from_str("02:30 PM - 04:00 PM").unwrap();
        assert_eq!(slot.format_12h(), "02:30 PM - 04:00 PM");

        let slot_noon = TimeSlot::from_str("10:30 AM - 12:00 PM").unwrap();
        assert_eq!(slot_noon.format_12h(), "10:30 AM - 12:00 PM");
        assert_eq!(slot_noon.duration_minutes(), 90);
    }

    #[test]
    fn test_malformed_inputs() {
        assert!(matches!(
            TimeSlot::from_str("invalid"),
            Err(TimeSlotError::MissingDelimiter(_))
        ));
        assert!(matches!(
            TimeSlot::from_str("9:15 - "),
            Err(TimeSlotError::InvalidToken { .. })
        ));
        assert!(matches!(
            TimeSlot::from_str("9:99 - 10:00"),
            Err(TimeSlotError::InvalidToken { .. })
        ));
    }

    #[test]
    fn test_overlap_detection() {
        let s1 = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let s2 = TimeSlot::from_str("10:00 - 11:15").unwrap(); // partial clash
        let s3 = TimeSlot::from_str("10:30 - 11:45").unwrap(); // back-to-back adjacent
        let s4 = TimeSlot::from_str("09:30 - 10:15").unwrap(); // completely inside s1

        assert!(s1.overlaps_with(&s2));
        assert!(s2.overlaps_with(&s1));
        assert!(s1.overlaps_with(&s4));
        assert!(!s1.overlaps_with(&s3)); // exact boundary -> no clash
        assert!(!s3.overlaps_with(&s1));
    }

    #[test]
    fn test_gap_calculation() {
        let s1 = TimeSlot::from_str("08:00 - 09:15").unwrap();
        let s2 = TimeSlot::from_str("10:30 - 11:45").unwrap();
        let s_adjacent = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let s_overlap = TimeSlot::from_str("09:00 - 10:00").unwrap();

        // 75-minute break between 9:15 and 10:30
        assert_eq!(s1.gap_between(&s2), Some(75));
        assert_eq!(s2.gap_between(&s1), Some(75));

        // 0-minute break between adjacent classes
        assert_eq!(s1.gap_between(&s_adjacent), Some(0));

        // Overlapping classes have no gap
        assert_eq!(s1.gap_between(&s_overlap), None);
    }

    #[test]
    fn test_deserialize_rejects_invalid_order() {
        let valid = r#"{"start":"09:15:00","end":"10:30:00"}"#;
        let slot: TimeSlot = serde_json::from_str(valid).unwrap();
        assert_eq!(slot.format_24h(), "09:15 - 10:30");
        assert_eq!(serde_json::to_string(&slot).unwrap(), valid);

        let invalid = r#"{"start":"10:30:00","end":"09:15:00"}"#;
        assert!(serde_json::from_str::<TimeSlot>(invalid).is_err());

        let equal = r#"{"start":"10:00:00","end":"10:00:00"}"#;
        assert!(serde_json::from_str::<TimeSlot>(equal).is_err());
    }
}
