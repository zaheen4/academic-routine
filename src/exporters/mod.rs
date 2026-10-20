//! Schedule format exporters: Terminal tables, iCalendar files, and Markdown.

pub mod ics;
pub mod terminal;

pub use ics::{export_ics_file, generate_ics};
pub use terminal::{render_day_table, render_today_card, render_weekly_overview};
