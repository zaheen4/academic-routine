//! Academic time slot representation and normalization.
//!
//! Handles class timings, duration calculations, and formatting
//! for both 12-hour human display and RFC 5545 iCalendar serialization.

use chrono::{NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Error returned when constructing an invalid [`TimeSlot`].
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
}

/// A class period defined by a start time and an end time.
///
/// Ensures an invariant that `start < end`.
///
/// # Examples
/// ```
/// use academic_routine::model::time_slot::TimeSlot;
/// use chrono::NaiveTime;
///
/// let start = NaiveTime::from_hms_opt(9, 15, 0).unwrap();
/// let end = NaiveTime::from_hms_opt(10, 30, 0).unwrap();
/// let slot = TimeSlot::new(start, end).unwrap();
///
/// assert_eq!(slot.duration_minutes(), 75);
/// assert_eq!(slot.format_12h(), "09:15 AM - 10:30 AM");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimeSlot {
    start: NaiveTime,
    end: NaiveTime,
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
            TimeSlotError::InvalidOrder {
                start: format!("{start_h:02}:{start_m:02}"),
                end: format!("{end_h:02}:{end_m:02}"),
            }
        })?;
        let end = NaiveTime::from_hms_opt(end_h, end_m, 0).ok_or_else(|| {
            TimeSlotError::InvalidOrder {
                start: format!("{start_h:02}:{start_m:02}"),
                end: format!("{end_h:02}:{end_m:02}"),
            }
        })?;
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
}

impl fmt::Display for TimeSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_12h())
    }
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
        // Start after end
        let err = TimeSlot::from_hms(11, 0, 9, 30);
        assert!(matches!(err, Err(TimeSlotError::InvalidOrder { .. })));

        // Start equal to end
        let err_equal = TimeSlot::from_hms(10, 0, 10, 0);
        assert!(matches!(err_equal, Err(TimeSlotError::InvalidOrder { .. })));
    }

    #[test]
    fn test_afternoon_slot_formatting() {
        let slot = TimeSlot::from_hms(14, 30, 16, 0).unwrap();
        assert_eq!(slot.duration_minutes(), 90);
        assert_eq!(slot.format_12h(), "02:30 PM - 04:00 PM");
        assert_eq!(slot.format_24h(), "14:30 - 16:00");
    }

    #[test]
    fn test_slot_ordering() {
        let slot_morning = TimeSlot::from_hms(8, 0, 9, 15).unwrap();
        let slot_noon = TimeSlot::from_hms(11, 45, 13, 0).unwrap();
        assert!(slot_morning < slot_noon);
    }
}
