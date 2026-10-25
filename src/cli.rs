//! Command-line argument parsing and subcommand definitions.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// High-performance academic schedule engine and assistant.
#[derive(Debug, Parser)]
#[command(name = "academic-routine")]
#[command(author = "Mir Zaheen Waseet <zwaseet@gmail.com>")]
#[command(version)]
#[command(
    about = "A fast academic schedule engine, .ics calendar synchronizer, and Telegram assistant",
    long_about = None
)]
pub struct Cli {
    /// Explicit path to routine JSON file (defaults to scraper output or local file)
    #[arg(short, long, global = true)]
    pub file: Option<PathBuf>,

    /// Student section filter (e.g. 'B2' keeps theory 'B' and lab 'B2', filtering other labs)
    #[arg(short, long, global = true)]
    pub section: Option<String>,

    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available CLI subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// View the complete weekly schedule or a specific day
    View {
        /// Specific day to display (e.g. 'Mon', 'Sunday')
        #[arg(short, long)]
        day: Option<String>,
    },

    /// View today's schedule and current session status
    Today,

    /// Show the immediate next upcoming class or active session
    Next,

    /// Search faculty directory by name, initials, or course code
    Teacher {
        /// Instructor name, initials, or course code to query
        query: Option<String>,
    },

    /// List all faculty members in the instructor directory
    Teachers,

    /// Scan schedule for time overlaps and room double-bookings
    Conflicts,

    /// Compare previous baseline routine and current routine to display timetable modifications
    Diff {
        /// Optional path to previous baseline routine file (defaults to output/routine_data.prev.json)
        prev: Option<PathBuf>,

        /// Optional path to current routine file (defaults to output/routine_data.json)
        current: Option<PathBuf>,
    },

    /// Mark a course whose syllabus is completed as finished (hides from alerts and calendar)
    Complete {
        /// Course code to complete (e.g. 'CSE-4125')
        code: String,
    },

    /// Drop a course, excluding it from schedules and calendar exports (alias for complete)
    Drop {
        /// Course code to drop (e.g. 'CSE-4101')
        code: String,
    },

    /// Restore or resume an active schedule for a previously completed course
    Resume {
        /// Course code to resume
        code: String,
    },

    /// Restore a previously dropped course (alias for resume)
    Restore {
        /// Course code to restore
        code: String,
    },

    /// List all courses currently marked as completed for the semester
    Completed,

    /// List all currently dropped/hidden courses (alias for completed)
    Dropped,

    /// Display complete subsystem status, active routine, vacation mode, and integration health
    Status,

    /// Manage vacation / semester break mode (e.g. 'vacation on', 'vacation off', 'vacation until 2026-11-15')
    Vacation {
        /// Action to perform: 'on', 'off', 'status', or 'until'
        action: Option<String>,

        /// Auto-expiration date if action is 'until' (format: YYYY-MM-DD)
        #[arg(long)]
        until: Option<String>,
    },

    /// Manage proactive 6:00 AM morning briefings ('on', 'off', 'status')
    Alerts {
        /// Action to perform: 'on', 'off', or 'status'
        action: Option<String>,
    },

    /// Pause a subsystem: 'alerts', 'gist', 'gsheet', or 'all'
    Pause {
        /// Subsystem to pause
        target: Option<String>,
    },

    /// Unpause / resume a subsystem: 'alerts', 'gist', 'gsheet', or 'all'
    Unpause {
        /// Subsystem to unpause
        target: Option<String>,
    },

    /// Manage automated weekly background sync timer ('status', 'pause', 'resume', 'run')
    Timer {
        /// Action to perform: 'status', 'pause', 'resume', or 'run'
        action: Option<String>,
    },

    /// Export schedule to calendar or file formats
    Export {
        /// Export to iCalendar (.ics) format file path (defaults to 'output/routine.ics')
        #[arg(long, default_value = "output/routine.ics")]
        ics: PathBuf,

        /// Explicit semester end date for recurrence UNTIL rule (format: YYYY-MM-DD)
        #[arg(long)]
        until: Option<String>,

        /// Automatically sync exported .ics to configured GitHub Gist
        #[arg(long)]
        sync_gist: bool,

        /// Export all courses including completed ones
        #[arg(long)]
        include_all: bool,
    },

    /// Link a GitHub Gist ID for automatic live calendar sync
    SetGist {
        /// Gist ID (e.g. '4a7e24f1f95a405673ae900e9838f3fd')
        id: String,
    },

    /// Sync the current routine.ics to your linked GitHub Gist immediately
    SyncGist,

    /// Run the interactive Telegram Assistant Bot and 6:00 AM alert daemon
    Bot {
        /// Disable lightweight HTTP health-check server (useful for pure local runs)
        #[arg(long)]
        no_http: bool,
    },

    /// Synchronize routine data directly to Google Spreadsheet (CSE-03_B_ClassRoutine)
    SyncGsheet {
        /// Explicit path to routine JSON file (defaults to output/routine_data.json)
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Scrape the latest routine directly from UCAM portal, update calendar, and sync Gist
    SyncUcam {
        /// Force re-scrape even if routine data is fresh
        #[arg(long)]
        force: bool,

        /// Run browser visibly for debugging (default: false / headless offscreen virtual display)
        #[arg(long)]
        headed: bool,

        /// Only update local routine_data.json and routine.ics (skip Gist and Google Sheets)
        #[arg(long)]
        local_only: bool,

        /// Skip syncing calendar and routine to GitHub Gist
        #[arg(long)]
        no_gist: bool,

        /// Skip syncing routine to Google Spreadsheet
        #[arg(long)]
        no_gsheet: bool,
    },
}

/// Resolves the administrator's Telegram Chat ID from the environment.
///
/// Prioritizes `ADMIN_CHAT_ID`, falling back to `TELEGRAM_CHAT_ID`.
#[must_use]
pub fn resolve_admin_chat_id() -> Option<i64> {
    std::env::var("ADMIN_CHAT_ID")
        .or_else(|_| std::env::var("TELEGRAM_CHAT_ID"))
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
}

/// Resolves a write target path, defaulting to `output/routine_data.json` if not specified.
/// Does not require the path to exist on disk.
#[must_use]
pub fn resolve_write_target(explicit: Option<&PathBuf>) -> PathBuf {
    explicit
        .cloned()
        .unwrap_or_else(|| PathBuf::from("output/routine_data.json"))
}

/// Resolves the routine data JSON path from an explicit CLI argument or fallback locations.
///
/// # Errors
/// Returns an error message if an explicit path was supplied but does not exist on disk.
pub fn resolve_routine_path(explicit: Option<&PathBuf>) -> Result<Option<PathBuf>, String> {
    if let Some(p) = explicit {
        if p.exists() {
            return Ok(Some(p.clone()));
        }
        return Err(format!(
            "Specified routine file does not exist: {}",
            p.display()
        ));
    }

    let output_data = PathBuf::from("output/routine_data.json");
    if output_data.exists() {
        return Ok(Some(output_data));
    }

    let local_data = PathBuf::from("routine_data.json");
    if local_data.exists() {
        return Ok(Some(local_data));
    }

    let output_final = PathBuf::from("output/final_combined_routine.json");
    if output_final.exists() {
        return Ok(Some(output_final));
    }

    let local_path = PathBuf::from("final_combined_routine.json");
    if local_path.exists() {
        return Ok(Some(local_path));
    }

    Ok(None)
}
