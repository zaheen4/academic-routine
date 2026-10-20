//! Academic day of week representation and ordering.
//!
//! University schedules in Bangladesh strictly begin on Saturday and end on Friday.
//! This module provides [`DayOfWeek`] with explicit numeric ordering where
//! `Saturday < Sunday < Monday < Tuesday < Wednesday < Thursday < Friday`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Days of the academic week sorted in university calendar order.
///
/// Implements [`Ord`] such that `Sat` is first (1) and `Fri` is last (7).
///
/// # Examples
/// ```
/// use academic_routine::DayOfWeek;
/// use std::str::FromStr;
///
/// let sat = DayOfWeek::from_str("SAT").unwrap();
/// let mon = DayOfWeek::from_str("Monday").unwrap();
///
/// assert!(sat < mon);
/// assert_eq!(sat.as_str(), "Sat");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DayOfWeek {
    /// Saturday (first day of the university week)
    Sat = 1,
    /// Sunday
    Sun = 2,
    /// Monday
    Mon = 3,
    /// Tuesday
    Tue = 4,
    /// Wednesday
    Wed = 5,
    /// Thursday
    Thu = 6,
    /// Friday
    Fri = 7,
}

impl DayOfWeek {
    /// Returns a slice containing all days of the week in university order (Sat..=Fri).
    #[must_use]
    pub const fn all() -> &'static [DayOfWeek] {
        &[
            DayOfWeek::Sat,
            DayOfWeek::Sun,
            DayOfWeek::Mon,
            DayOfWeek::Tue,
            DayOfWeek::Wed,
            DayOfWeek::Thu,
            DayOfWeek::Fri,
        ]
    }

    /// Returns the short 3-letter abbreviation (e.g. `"Sat"`, `"Mon"`).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            DayOfWeek::Sat => "Sat",
            DayOfWeek::Sun => "Sun",
            DayOfWeek::Mon => "Mon",
            DayOfWeek::Tue => "Tue",
            DayOfWeek::Wed => "Wed",
            DayOfWeek::Thu => "Thu",
            DayOfWeek::Fri => "Fri",
        }
    }

    /// Returns the full day name (e.g. `"Saturday"`, `"Monday"`).
    #[must_use]
    pub const fn full_name(&self) -> &'static str {
        match self {
            DayOfWeek::Sat => "Saturday",
            DayOfWeek::Sun => "Sunday",
            DayOfWeek::Mon => "Monday",
            DayOfWeek::Tue => "Tuesday",
            DayOfWeek::Wed => "Wednesday",
            DayOfWeek::Thu => "Thursday",
            DayOfWeek::Fri => "Friday",
        }
    }

    /// Returns the 2-letter code used by the iCalendar RFC 5545 recurrence rule (e.g. `"SA"`, `"MO"`).
    #[must_use]
    pub const fn ical_code(&self) -> &'static str {
        match self {
            DayOfWeek::Sat => "SA",
            DayOfWeek::Sun => "SU",
            DayOfWeek::Mon => "MO",
            DayOfWeek::Tue => "TU",
            DayOfWeek::Wed => "WE",
            DayOfWeek::Thu => "TH",
            DayOfWeek::Fri => "FR",
        }
    }

    /// Returns whether this day is typically a weekend in Bangladesh (Friday or Saturday).
    #[must_use]
    pub const fn is_weekend(&self) -> bool {
        matches!(self, DayOfWeek::Fri | DayOfWeek::Sat)
    }
}

impl fmt::Display for DayOfWeek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Error returned when parsing an unrecognized day name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unrecognized day of week '{0}'. Expected Sat, Sun, Mon, Tue, Wed, Thu, or Fri.")]
pub struct ParseDayError(pub String);

impl FromStr for DayOfWeek {
    type Err = ParseDayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let cleaned = s.trim().to_uppercase();
        match cleaned.as_str() {
            "SAT" | "SATURDAY" => Ok(DayOfWeek::Sat),
            "SUN" | "SUNDAY" => Ok(DayOfWeek::Sun),
            "MON" | "MONDAY" => Ok(DayOfWeek::Mon),
            "TUE" | "TUESDAY" => Ok(DayOfWeek::Tue),
            "WED" | "WEDNESDAY" => Ok(DayOfWeek::Wed),
            "THU" | "THURSDAY" => Ok(DayOfWeek::Thu),
            "FRI" | "FRIDAY" => Ok(DayOfWeek::Fri),
            _ => Err(ParseDayError(s.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_university_day_ordering() {
        assert!(DayOfWeek::Sat < DayOfWeek::Sun);
        assert!(DayOfWeek::Sun < DayOfWeek::Mon);
        assert!(DayOfWeek::Mon < DayOfWeek::Tue);
        assert!(DayOfWeek::Tue < DayOfWeek::Wed);
        assert!(DayOfWeek::Wed < DayOfWeek::Thu);
        assert!(DayOfWeek::Thu < DayOfWeek::Fri);

        let mut days = vec![
            DayOfWeek::Fri,
            DayOfWeek::Mon,
            DayOfWeek::Sat,
            DayOfWeek::Wed,
        ];
        days.sort();
        assert_eq!(
            days,
            vec![
                DayOfWeek::Sat,
                DayOfWeek::Mon,
                DayOfWeek::Wed,
                DayOfWeek::Fri
            ]
        );
    }

    #[test]
    fn test_from_str_case_insensitivity() {
        assert_eq!("sat".parse::<DayOfWeek>().unwrap(), DayOfWeek::Sat);
        assert_eq!("Saturday".parse::<DayOfWeek>().unwrap(), DayOfWeek::Sat);
        assert_eq!("  mOnDay  ".parse::<DayOfWeek>().unwrap(), DayOfWeek::Mon);
        assert_eq!("wed".parse::<DayOfWeek>().unwrap(), DayOfWeek::Wed);
        assert_eq!("FRI".parse::<DayOfWeek>().unwrap(), DayOfWeek::Fri);
    }

    #[test]
    fn test_from_str_invalid() {
        assert!("Funday".parse::<DayOfWeek>().is_err());
        assert!("".parse::<DayOfWeek>().is_err());
        assert!("123".parse::<DayOfWeek>().is_err());
    }

    #[test]
    fn test_ical_code() {
        assert_eq!(DayOfWeek::Sat.ical_code(), "SA");
        assert_eq!(DayOfWeek::Sun.ical_code(), "SU");
        assert_eq!(DayOfWeek::Mon.ical_code(), "MO");
        assert_eq!(DayOfWeek::Fri.ical_code(), "FR");
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", DayOfWeek::Sat), "Sat");
        assert_eq!(DayOfWeek::Sat.full_name(), "Saturday");
    }

    #[test]
    fn test_serialization() {
        let serialized = serde_json::to_string(&DayOfWeek::Mon).unwrap();
        let deserialized: DayOfWeek = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, DayOfWeek::Mon);
    }
}
