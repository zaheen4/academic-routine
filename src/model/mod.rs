//! Core domain data types for the academic schedule.

pub mod course;
pub mod day;
pub mod schedule;
pub mod state;
pub mod time_slot;

pub use course::{CourseEntry, TeacherContact};
pub use day::{DayOfWeek, ParseDayError};
pub use schedule::{Schedule, ScheduleConflict};
pub use state::{RoutineState, StateError};
pub use time_slot::{TimeSlot, TimeSlotError};
