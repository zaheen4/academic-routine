//! Core domain data types for the academic schedule.

pub mod day;
pub mod time_slot;

pub use day::{DayOfWeek, ParseDayError};
pub use time_slot::{TimeSlot, TimeSlotError};
