//! # Academic Routine (`academic-routine`)
//!
//! A high-performance academic schedule engine, timetable organizer,
//! calendar synchronizer, and notification assistant built in Rust.
//!
//! This crate separates core scheduling business logic (the library)
//! from terminal interfaces and bot daemons (the application).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod engine;
pub mod exporters;
pub mod model;

pub use engine::{load_routine_file, parse_routine_json, IngestError};
pub use exporters::{render_day_table, render_today_card, render_weekly_overview};
pub use model::{
    CourseEntry, DayOfWeek, ParseDayError, RoutineState, Schedule, ScheduleConflict, StateError,
    TeacherContact, TimeSlot, TimeSlotError,
};
