//! # Academic Routine (`academic-routine`)
//!
//! A high-performance academic schedule engine, timetable organizer,
//! calendar synchronizer, and notification assistant built in Rust.
//!
//! This crate separates core scheduling business logic (the library)
//! from terminal interfaces and bot daemons (the application).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bot;
pub mod cli;
pub mod engine;
pub mod exporters;
pub mod model;

pub use bot::{run_bot_daemon, BotContext, Command};
pub use cli::{Cli, Commands};
pub use engine::{
    load_routine_file, parse_routine_json, run_gsheet_sync, run_ucam_scraper, run_ucam_scraper_opts,
    IngestError, ScraperError,
};
pub use exporters::{
    export_ics_file, fetch_gist_routine_data, fetch_gist_state_data, generate_ics, render_day_table,
    render_today_card, render_weekly_overview, sync_gist_bundle, sync_gist_bundle_with_state,
    sync_gist_file,
};
pub use model::{
    academic_now, current_academic_day, next_academic_day, CourseEntry, DayOfWeek, ParseDayError,
    RoutineState, Schedule, ScheduleConflict, StateError, TeacherContact, TimeSlot, TimeSlotError,
};
