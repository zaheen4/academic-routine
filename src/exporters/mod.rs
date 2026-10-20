//! Schedule format exporters: Terminal tables, iCalendar files, and Markdown.

pub mod terminal;

pub use terminal::{render_day_table, render_today_card, render_weekly_overview};
