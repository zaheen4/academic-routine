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
pub use cli::{resolve_routine_path, resolve_write_target, Cli, Commands};
pub use engine::{
    load_routine_file, parse_routine_json, run_gsheet_sync, run_ucam_scraper,
    run_ucam_scraper_opts, run_ucam_scraper_opts_force, IngestError, ScraperError,
};
pub use exporters::{
    export_ics_file, fetch_gist_meta_data, fetch_gist_routine_data, generate_ics, render_day_table,
    render_teacher_card, render_today_card, render_weekly_overview, sync_gist_bundle,
    sync_gist_file,
};
pub use model::{
    academic_now, codes_match, current_academic_day, next_academic_day, AttendanceRecord,
    AttendanceStatus, ChangeType, CourseEntry, DayOfWeek, ParseDayError, RoutineState, Schedule,
    ScheduleConflict, ScheduleDiff, ScheduleDiffItem, StateError, StateStore, TeacherContact,
    TimeSlot, TimeSlotError,
};
