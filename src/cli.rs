//! Command-line argument parsing and subcommand definitions.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// High-performance academic schedule engine and assistant.
#[derive(Debug, Parser)]
#[command(name = "academic-routine")]
#[command(author = "Mir Zaheen Waseet <zwaseet@gmail.com>")]
#[command(version = "0.1.0")]
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

    /// Scan schedule for time overlaps and room double-bookings
    Conflicts,

    /// Drop a course, excluding it from schedules and calendar exports
    Drop {
        /// Course code to drop (e.g. 'CSE-4101')
        code: String,
    },

    /// Restore a previously dropped course
    Restore {
        /// Course code to restore
        code: String,
    },

    /// List all currently dropped/hidden courses
    Dropped,

    /// Export schedule to calendar or file formats
    Export {
        /// Export to iCalendar (.ics) format file path (defaults to 'routine.ics')
        #[arg(long, default_value = "routine.ics")]
        ics: PathBuf,

        /// Explicit semester end date for recurrence UNTIL rule (format: YYYY-MM-DD)
        #[arg(long)]
        until: Option<String>,
    },

    /// Display teacher directory with phone numbers and emails
    Teachers,
}
