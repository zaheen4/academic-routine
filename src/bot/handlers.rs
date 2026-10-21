//! Telegram command handlers and interactive assistant logic.

use crate::bot::keyboards::*;
use crate::engine::parser::load_routine_file;
use crate::exporters::ics::export_ics_file;
use crate::model::course::{CourseEntry, TeacherContact};
use crate::model::day::{academic_now, current_academic_day, next_academic_day, DayOfWeek};
use crate::model::schedule::Schedule;
use crate::model::state::RoutineState;
use chrono::Timelike;
use std::path::PathBuf;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, InlineKeyboardMarkup, InputFile, ParseMode};
use teloxide::utils::command::BotCommands;

/// 🎓 University Routine Assistant Commands:
#[derive(BotCommands, Clone, Debug, PartialEq, Eq)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    /// Show help and command instructions
    Help,
    /// Start the routine assistant and show overview
    Start,
    /// View today's schedule and active session
    Today,
    /// Show upcoming or in-session class with countdown
    Next,
    /// Preview tomorrow's class schedule
    Tomorrow,
    /// Show break and study windows for today
    Free,
    /// List instructor directory with phone numbers and emails
    Teachers,
    /// Search faculty directory by name, initials, or course (e.g. /teacher basar)
    Teacher(String),
    /// View complete weekly routine timetable
    Routine,
    /// Show routine status, alert settings, and vacation mode
    Status,
    /// Mark a completed course to stop alerts (e.g. /complete CSE-4125)
    Complete(String),
    /// List all courses marked as completed for the semester
    Completed,
    /// Resume routine for a completed course (e.g. /resume CSE-4125)
    Resume(String),
    /// Configure 6:00 AM daily briefing notifications (on/off/status)
    Alerts(String),
    /// Manage vacation / semester break mode (on/off/until)
    Vacation(String),
    /// Send .ics calendar file directly to chat for 1-tap phone import
    Calendar,
    /// Refresh routine directly from UCAM portal and update calendar
    Sync,
    /// Alias for /routine
    #[command(hide)]
    Week,
    /// Alias for /complete (e.g. /finish CSE-4125)
    #[command(hide)]
    Finish(String),
    /// Alias for /completed
    #[command(hide)]
    Finished,
    /// Alias for /complete (e.g. /drop CSE-4101)
    #[command(hide)]
    Drop(String),
    /// Alias for /resume (e.g. /restore CSE-4101)
    #[command(hide)]
    Restore(String),
    /// Alias for /completed
    #[command(hide)]
    Dropped,
}

/// Shared runtime context for the Telegram bot.
#[derive(Debug, Clone)]
pub struct BotContext {
    /// Path to routine JSON file.
    pub routine_path: PathBuf,
    /// Path to routine_state.json.
    pub state_path: PathBuf,
    /// Default student section (e.g. "B2").
    pub section: Option<String>,
    /// Whether to disable the HTTP health check server.
    pub no_http: bool,
}

impl BotContext {
    /// Loads the active, filtered schedule based on current state.
    pub fn load_active_schedule(&self) -> Result<(Schedule, RoutineState), String> {
        let state = RoutineState::load_or_default(&self.state_path).unwrap_or_default();
        let raw = load_routine_file(&self.routine_path)
            .map_err(|e| format!("Failed to read routine: {e}"))?;

        let mut sched = raw.filter_active(&state.dropped_courses);
        if let Some(ref sec) = self.section {
            sched = sched.filter_section(sec);
        }

        Ok((sched, state))
    }

    /// Resolves the .ics file path, prioritizing output/routine.ics if it exists or by default.
    pub fn resolve_ics_path(&self) -> PathBuf {
        let output_ics = PathBuf::from("output/routine.ics");
        if output_ics.exists() {
            return output_ics;
        }
        let local_ics = PathBuf::from("routine.ics");
        if local_ics.exists() {
            return local_ics;
        }
        output_ics
    }

    /// Regenerates routine.ics and syncs to GitHub Gist if configured.
    pub async fn regenerate_and_sync(&self) -> Result<String, String> {
        let (sched, state) = self.load_active_schedule()?;
        let ics_path = self.resolve_ics_path();
        if let Some(parent) = ics_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        export_ics_file(&sched, &ics_path, state.semester_end_date)
            .map_err(|e| format!("Failed to export .ics: {e}"))?;

        if let Some(ref gid) = state.gist_id {
            if self.routine_path.exists() {
                return crate::exporters::gist::sync_gist_bundle_with_state(
                    gid,
                    &ics_path,
                    &self.routine_path,
                    Some(&self.state_path),
                )
                .await;
            }
            return crate::exporters::gist::sync_gist_file(gid, &ics_path).await;
        }

        Ok(ics_path.display().to_string())
    }

    /// Refreshes routine_data.json and routine_state.json from GitHub Gist, writes locally, and updates calendar feed.
    pub async fn refresh_from_gist(&self) -> Result<String, String> {
        let mut state = RoutineState::load_or_default(&self.state_path).unwrap_or_default();
        let gid = match state.gist_id.as_deref() {
            Some(id) => id.to_string(),
            None => std::env::var("GIST_ID")
                .map_err(|_| "No Gist ID configured in state or environment.".to_string())?,
        };

        // If routine_state.json is present on Gist, sync state fields
        if let Ok(state_json) = crate::exporters::gist::fetch_gist_state_data(&gid).await {
            if let Ok(remote_state) = serde_json::from_str::<RoutineState>(&state_json) {
                for d in remote_state.dropped_courses {
                    state.drop_course(&d);
                }
                let _ = state.save(&self.state_path);
            }
        }

        let json_content = crate::exporters::gist::fetch_gist_routine_data(&gid).await?;

        // Validate JSON integrity
        let _parsed = crate::engine::parser::parse_routine_json(&json_content)
            .map_err(|e| format!("Invalid routine data from Gist: {e}"))?;

        // Save atomically to routine_path
        let tmp_path = self.routine_path.with_extension("tmp");
        std::fs::write(&tmp_path, &json_content)
            .map_err(|e| format!("Failed to write routine data: {e}"))?;
        std::fs::rename(&tmp_path, &self.routine_path)
            .map_err(|e| format!("Failed to commit routine data: {e}"))?;

        self.regenerate_and_sync().await
    }
}

/// Renders the Today schedule card with live badges and navigation buttons.
#[must_use]
pub fn render_today_card(
    sched: &Schedule,
    state: &RoutineState,
) -> (String, InlineKeyboardMarkup) {
    let today = current_academic_day();
    let classes = sched.for_day(today);
    let now = academic_now().time();

    let vacation_banner = if state.is_vacation_active() {
        "🏖️ <b>Vacation Mode Active (Semester Break)</b>\n<i>No official routine classes scheduled. Send /vacation off to resume normal alerts.</i>\n\n"
    } else {
        ""
    };

    if classes.is_empty() {
        let text = format!(
            "{vacation_banner}🎉 <b>No classes scheduled for today ({})!</b> Enjoy your free time.",
            today.full_name()
        );
        return (text, today_keyboard());
    }

    let mut text = format!(
        "{}📅 <b>Schedule for Today ({})</b> • {} classes\n───────────────────────────\n",
        vacation_banner,
        today.full_name(),
        classes.len()
    );

    let next_class = classes.iter().find(|c| c.time_slot.start() > now);

    for c in &classes {
        let lab_badge = if c.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
        let status_badge = if c.time_slot.contains_time(now) {
            "  [IN SESSION 🟢]".to_string()
        } else if Some(c) == next_class {
            let mins = (c.time_slot.start().hour() * 60 + c.time_slot.start().minute())
                .saturating_sub(now.hour() * 60 + now.minute());
            format!("  [NEXT • {mins}m ⏳]")
        } else if c.time_slot.end() < now {
            "  [COMPLETED ⚪]".to_string()
        } else {
            String::new()
        };

        text.push_str(&format!(
            "<code>{}</code> • <b>{}</b>{}\n<b>{}: {}</b>{}\n",
            c.time_slot.format_12h(),
            c.room,
            status_badge,
            c.code,
            escape_html(&c.title),
            lab_badge
        ));
        if !c.teacher.name.is_empty() {
            text.push_str(&format!("👤 {}\n", escape_html(&c.teacher.name)));
        }
        text.push('\n');
    }

    text.push_str("───────────────────────────\n<i>💡 30-minute reminder notifications are active</i>");
    (text, today_keyboard())
}

/// Renders the Next Class card with countdown and quick refresh buttons.
#[must_use]
pub fn render_next_card(
    sched: &Schedule,
    state: &RoutineState,
) -> (String, InlineKeyboardMarkup) {
    if state.is_vacation_active() {
        let text = "🏖️ <b>Vacation Mode Active (Semester Break)</b>\nNo classes are currently scheduled. Send <code>/vacation off</code> when classes resume.".to_string();
        return (text, next_keyboard());
    }

    let today = current_academic_day();
    let now = academic_now().time();
    let classes = sched.for_day(today);

    if let Some(active) = classes.iter().find(|c| c.time_slot.contains_time(now)) {
        let mins_left = (active.time_slot.end().hour() * 60 + active.time_slot.end().minute())
            .saturating_sub(now.hour() * 60 + now.minute());
        let lab_badge = if active.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
        let text = format!(
            "🔴 <b>CURRENTLY IN SESSION</b>\n───────────────────────────\n\
             <code>{}</code> (ends in <b>{} mins</b>)\n\n\
             📖 <b>{}: {}</b>{}\n\
             🏛️ Room: <code>{}</code>\n\
             👤 Faculty: <i>{}</i>\n───────────────────────────",
            active.time_slot.format_12h(),
            mins_left,
            active.code,
            escape_html(&active.title),
            lab_badge,
            active.room,
            escape_html(&active.teacher.name)
        );
        (text, next_keyboard())
    } else if let Some(next) = classes.iter().find(|c| c.time_slot.start() > now) {
        let mins_until = (next.time_slot.start().hour() * 60 + next.time_slot.start().minute())
            .saturating_sub(now.hour() * 60 + now.minute());
        let lab_badge = if next.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
        let text = format!(
            "⏱️ <b>Next Upcoming Class</b>\n───────────────────────────\n\
             ⏳ In <b>{} minutes</b> (starts at <code>{}</code>)\n\n\
             📖 <b>{}: {}</b>{}\n\
             🏛️ Room: <code>{}</code>\n\
             👤 Faculty: <i>{}</i>\n───────────────────────────\n\
             <i>Tap refresh to update live countdown</i>",
            mins_until,
            next.time_slot.start().format("%I:%M %p"),
            next.code,
            escape_html(&next.title),
            lab_badge,
            next.room,
            escape_html(&next.teacher.name)
        );
        (text, next_keyboard())
    } else {
        let text = "🏁 <b>All classes for today have concluded!</b>\n───────────────────────────\nGood job! Send /tomorrow or tap below to preview upcoming days.".to_string();
        (text, next_keyboard())
    }
}

/// Renders the Tomorrow preview card.
#[must_use]
pub fn render_tomorrow_card(sched: &Schedule) -> (String, InlineKeyboardMarkup) {
    let tomorrow = next_academic_day();
    let classes = sched.for_day(tomorrow);

    if classes.is_empty() {
        let text = format!(
            "🎉 <b>No classes scheduled for tomorrow ({})!</b> Have a restful evening.",
            tomorrow.full_name()
        );
        return (text, today_keyboard());
    }

    let mut text = format!(
        "🗓️ <b>Preview for Tomorrow ({})</b> • {} classes\n───────────────────────────\n",
        tomorrow.full_name(),
        classes.len()
    );

    for c in &classes {
        let lab_badge = if c.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
        text.push_str(&format!(
            "<code>{}</code> • <b>{}</b>\n<b>{}: {}</b>{}\n",
            c.time_slot.format_12h(),
            c.room,
            c.code,
            escape_html(&c.title),
            lab_badge
        ));
        if !c.teacher.name.is_empty() {
            text.push_str(&format!("👤 {}\n", escape_html(&c.teacher.name)));
        }
        text.push('\n');
    }

    text.push_str("───────────────────────────\n<i>💡 Send /routine to view the full week</i>");
    (text, today_keyboard())
}

/// Renders the compact weekly routine overview with emojiless day switcher buttons.
#[must_use]
pub fn render_routine_overview(sched: &Schedule) -> (String, InlineKeyboardMarkup) {
    let mut text = format!(
        "📋 <b>Weekly Academic Routine</b>\n\
         <i>{} | {} classes total</i>\n\
         ───────────────────────────\n",
        escape_html(&sched.semester_name),
        sched.entries.len()
    );

    for day in DayOfWeek::all() {
        let day_classes = sched.for_day(*day);
        if day_classes.is_empty() {
            continue;
        }

        text.push_str(&format!(
            "<b>{}</b> • {} classes\n",
            day.full_name(),
            day_classes.len()
        ));
        let count = day_classes.len();
        for (idx, c) in day_classes.iter().enumerate() {
            let branch = if idx == count - 1 { "└" } else { "├" };
            let lab_badge = if c.is_lab() { " 🔬" } else { "" };
            text.push_str(&format!(
                "{} <code>{}</code> <b>{}</b>: {}{}\n",
                branch,
                c.time_slot.format_12h(),
                c.room,
                c.code,
                lab_badge
            ));
        }
        text.push('\n');
    }

    text.push_str("───────────────────────────\n<i>Tap any day below to expand full instructor and room details:</i>");
    (text, routine_days_keyboard(None))
}

/// Renders a single day's routine expanded with full instructor and room details.
#[must_use]
pub fn render_routine_day(sched: &Schedule, day: DayOfWeek) -> (String, InlineKeyboardMarkup) {
    let classes = sched.for_day(day);
    if classes.is_empty() {
        let text = format!(
            "📋 <b>Schedule for {}</b>\n───────────────────────────\nNo classes scheduled on this day! Enjoy your break.",
            day.full_name()
        );
        return (text, routine_days_keyboard(Some(day)));
    }

    let mut text = format!(
        "📋 <b>Schedule for {}</b> • {} classes\n───────────────────────────\n",
        day.full_name(),
        classes.len()
    );

    for c in &classes {
        let lab_badge = if c.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
        text.push_str(&format!(
            "<code>{}</code> • Room <b>{}</b>\n<b>{}: {}</b>{}\n",
            c.time_slot.format_12h(),
            c.room,
            c.code,
            escape_html(&c.title),
            lab_badge
        ));
        if !c.teacher.name.is_empty() {
            text.push_str(&format!("👤 {}\n", escape_html(&c.teacher.name)));
        }
        text.push('\n');
    }

    text.push_str("───────────────────────────\n<i>Tap another day to switch views:</i>");
    (text, routine_days_keyboard(Some(day)))
}

/// Renders the faculty directory card.
#[must_use]
pub fn render_teachers_directory(sched: &Schedule) -> (String, InlineKeyboardMarkup) {
    let dir = sched.teacher_directory();
    let mut text = format!(
        "📚 <b>Faculty Directory</b> ({} Instructors)\n───────────────────────────\n",
        dir.len()
    );
    for (name, contact) in dir {
        text.push_str(&format!("• <b>{}</b>\n", escape_html(&name)));
        if let Some(ref p) = contact.phone {
            text.push_str(&format!("  📞 Phone: <code>{}</code>\n", escape_html(p)));
        }
        if let Some(ref e) = contact.email {
            text.push_str(&format!("  ✉️ Email: <code>{}</code>\n", escape_html(e)));
        }
        text.push('\n');
    }
    text.push_str("───────────────────────────\n<i>💡 Search any instructor with <code>/teacher &lt;name&gt;</code></i>");
    (text, today_keyboard())
}

/// Renders an individual faculty profile card with 1-tap call/email buttons.
#[must_use]
pub fn render_teacher_profile(
    name: &str,
    contact: &TeacherContact,
    courses: &[&CourseEntry],
) -> (String, InlineKeyboardMarkup) {
    let mut text = format!(
        "👤 <b>Faculty Profile: {}</b>\n───────────────────────────\n",
        escape_html(name)
    );
    if let Some(ref p) = contact.phone {
        text.push_str(&format!("📞 <b>Phone:</b> <code>{}</code>\n", escape_html(p)));
    }
    if let Some(ref e) = contact.email {
        text.push_str(&format!("✉️ <b>Email:</b> <code>{}</code>\n", escape_html(e)));
    }

    if !courses.is_empty() {
        text.push_str("\n📖 <b>Courses Taught:</b>\n");
        for c in courses {
            let lab_badge = if c.is_lab() { " 🔬" } else { "" };
            text.push_str(&format!(
                "• <code>{}</code>: {}{}\n  └ {} {} (Room {})\n",
                c.code,
                escape_html(&c.title),
                lab_badge,
                c.day.full_name(),
                c.time_slot.format_12h(),
                c.room
            ));
        }
    }
    text.push_str("───────────────────────────");

    let markup = teacher_contact_keyboard(contact.phone.as_deref(), contact.email.as_deref());
    (text, markup)
}

/// Renders today's break and study windows card.
#[must_use]
pub fn render_free_card(
    sched: &Schedule,
    state: &RoutineState,
) -> (String, InlineKeyboardMarkup) {
    let today = current_academic_day();
    let classes = sched.for_day(today);

    let vacation_banner = if state.is_vacation_active() {
        "🏖️ <b>Vacation Mode Active (Semester Break)</b>\n<i>All hours are currently free time!</i>\n\n"
    } else {
        ""
    };

    if classes.len() < 2 {
        let text = format!(
            "{vacation_banner}🎉 <b>No mid-day gaps today ({})!</b>\n\
             You have either a completely free day or only a single continuous class block.",
            today.full_name()
        );
        return (text, today_keyboard());
    }

    let mut breaks = Vec::new();
    for i in 0..classes.len() - 1 {
        let current = &classes[i];
        let next = &classes[i + 1];
        if let Some(gap_mins) = current.time_slot.gap_between(&next.time_slot) {
            if gap_mins > 0 {
                let note = if gap_mins >= 60 {
                    "🥪 Lunch / Major Study Break"
                } else if gap_mins >= 30 {
                    "☕ Break Window"
                } else {
                    "🚶 Class Transition Window"
                };
                breaks.push((
                    current.time_slot.end().format("%I:%M %p").to_string(),
                    next.time_slot.start().format("%I:%M %p").to_string(),
                    gap_mins,
                    note,
                ));
            }
        }
    }

    if breaks.is_empty() {
        let text = format!(
            "☕ <b>No breaks scheduled between classes today ({})</b>\nClasses run back-to-back.",
            today.full_name()
        );
        return (text, today_keyboard());
    }

    let mut text = format!(
        "{}☕ <b>Free Breaks Today ({})</b>\n───────────────────────────\n",
        vacation_banner,
        today.full_name()
    );

    for (start, end, mins, note) in &breaks {
        text.push_str(&format!(
            "• <code>{} – {}</code> (<b>{} mins</b>)\n  {}\n\n",
            start, end, mins, note
        ));
    }

    text.push_str(&format!(
        "───────────────────────────\n<i>💡 {} study break(s) available between your classes</i>",
        breaks.len()
    ));

    (text, today_keyboard())
}

/// Renders the subsystem status card.
#[must_use]
pub fn render_status_card(
    sched: &Schedule,
    state: &RoutineState,
    chat_id: ChatId,
) -> (String, InlineKeyboardMarkup) {
    let user_display = state
        .telegram_username
        .as_deref()
        .map(|u| format!("{u} [Chat {chat_id}]"))
        .unwrap_or_else(|| format!("Chat {chat_id}"));

    let vacation_str = if state.is_vacation_active() {
        if let Some(ref until) = state.vacation_until {
            format!("🏖️ <b>ACTIVE</b> (until {until})")
        } else {
            "🏖️ <b>ACTIVE</b> (Semester Break)".to_string()
        }
    } else {
        "<b>INACTIVE</b> (Normal Semester)".to_string()
    };

    let alerts_str = if state.morning_alerts {
        format!("🔔 <b>ENABLED</b> (06:00 AM to {user_display})")
    } else {
        "🔕 <b>PAUSED</b> (/alerts on to resume)".to_string()
    };

    let completed_count = state.dropped_courses.len();
    let gist_status = if let Some(ref gid) = state.gist_id {
        let short = &gid[..8.min(gid.len())];
        format!("Linked (<code>{short}...</code>)")
    } else {
        "Not linked".to_string()
    };

    let sheet_name = std::env::var("SPREADSHEET_NAME")
        .unwrap_or_else(|_| "CSE-03_B_ClassRoutine".to_string());

    let text = format!(
        "🎓 <b>Academic Routine Assistant Status</b>\n───────────────────────────\n\
         📁 <b>Semester:</b> <code>{}</code>\n\
         🛡️ <b>Courses:</b> {} active ({} completed)\n\
         🏖️ <b>Vacation:</b> {}\n\
         🔔 <b>Morning Alerts:</b> {}\n───────────────────────────\n\
         🔗 <b>Gist Feed:</b> {}\n\
         📊 <b>Google Sheets:</b> <code>{}</code>\n\
         🤖 <b>Bot Daemon:</b> Online &amp; Listening\n───────────────────────────\n\
         <i>💡 Tap a button below or send /alerts, /vacation to adjust.</i>",
        escape_html(&sched.semester_name),
        sched.entries.len(),
        completed_count,
        vacation_str,
        alerts_str,
        gist_status,
        escape_html(&sheet_name),
    );

    (text, main_menu_keyboard())
}

/// Dispatches an incoming Telegram command using reliable HTML formatting.
pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    ctx: Arc<BotContext>,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id;

    // Auto-cache user handle/name in routine_state.json if available
    if let Some(user) = &msg.from {
        let handle = user
            .username
            .as_ref()
            .map(|u| format!("@{u}"))
            .unwrap_or_else(|| user.first_name.clone());
        let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
        if state.telegram_username.as_deref() != Some(&handle) {
            state.telegram_username = Some(handle);
            let _ = state.save(&ctx.state_path);
        }
    }

    match cmd {
        Command::Help | Command::Start => {
            let help_text = "🎓 <b>Academic Routine Assistant</b>\n\
                 <i>Autonomous timetable, reminder &amp; calendar assistant</i>\n\
                 ───────────────────────────\n\
                 <b>Quick Navigation:</b>\n\
                 Tap any button below or send a command:\n\n\
                 • <code>/today</code> — Today's classes &amp; live status\n\
                 • <code>/next</code> — Upcoming class with countdown\n\
                 • <code>/routine</code> — Full weekly timetable &amp; day switcher\n\
                 • <code>/teacher &lt;name&gt;</code> — Search faculty &amp; 1-tap call/email\n\
                 • <code>/free</code> — Study &amp; break windows\n\
                 • <code>/status</code> — System health &amp; alert status\n\
                 • <code>/calendar</code> — 1-tap mobile calendar import\n\
                 ───────────────────────────\n\
                 <i>💡 30-minute pre-class reminders are active in the background.</i>";

            bot.send_message(chat_id, help_text)
                .parse_mode(ParseMode::Html)
                .reply_markup(main_menu_keyboard())
                .await?;
        }

        Command::Status => {
            let (sched, state) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let (text, markup) = render_status_card(&sched, &state, chat_id);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await?;
        }

        Command::Today => {
            let (sched, state) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let (text, markup) = render_today_card(&sched, &state);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await?;
        }

        Command::Next => {
            let (sched, state) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let (text, markup) = render_next_card(&sched, &state);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await?;
        }

        Command::Tomorrow => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let (text, markup) = render_tomorrow_card(&sched);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await?;
        }

        Command::Free => {
            let (sched, state) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let (text, markup) = render_free_card(&sched, &state);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await?;
        }

        Command::Teachers => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let (text, markup) = render_teachers_directory(&sched);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await?;
        }

        Command::Teacher(ref query) => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let q = query.trim();
            if q.is_empty() {
                let help_text = "🔍 <b>Faculty Directory Search</b>\n\
                    ───────────────────────────\n\
                    Search any instructor by name, initials, or course code:\n\n\
                    • <code>/teacher basar</code>\n\
                    • <code>/teacher SS</code>\n\
                    • <code>/teacher CSE-4125</code>\n\
                    ───────────────────────────\n\
                    <i>Tap below to view the entire faculty directory:</i>";
                bot.send_message(chat_id, help_text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(teacher_contact_keyboard(None, None))
                    .await?;
                return Ok(());
            }

            let matches = sched.find_teachers(q);
            if matches.is_empty() {
                let text = format!(
                    "🔍 No faculty found matching \"<b>{}</b>\".\n\n\
                     <i>Tip: Check spelling or search by course code (e.g. <code>/teacher 4125</code>).</i>",
                    escape_html(q)
                );
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(teacher_contact_keyboard(None, None))
                    .await?;
                return Ok(());
            }

            if matches.len() == 1 {
                let (name, contact, courses) = &matches[0];
                let (text, markup) = render_teacher_profile(name, contact, courses);
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(markup)
                    .await?;
            } else {
                let mut text = format!(
                    "🔍 Found <b>{}</b> faculty matching \"<b>{}</b>\":\n───────────────────────────\n",
                    matches.len(),
                    escape_html(q)
                );
                for (name, contact, courses) in &matches {
                    let phone_str = contact.phone.as_deref().unwrap_or("No phone");
                    let email_str = contact.email.as_deref().unwrap_or("No email");
                    text.push_str(&format!(
                        "• <b>{}</b> ({} classes)\n  📞 <code>{}</code> • ✉️ <code>{}</code>\n",
                        escape_html(name),
                        courses.len(),
                        phone_str,
                        email_str
                    ));
                }
                text.push_str("───────────────────────────\n<i>Tip: Refine your search to open direct call/email action buttons.</i>");
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(teacher_contact_keyboard(None, None))
                    .await?;
            }
        }

        Command::Complete(code) | Command::Finish(code) | Command::Drop(code) => {
            let clean_code = code.trim().to_uppercase();
            if clean_code.is_empty() {
                bot.send_message(
                    chat_id,
                    "⚠️ Please provide a course code to complete (e.g. <code>/complete CSE-4125</code>)",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.complete_course(&clean_code) {
                let _ = state.save(&ctx.state_path);
                let sync_msg = match ctx.regenerate_and_sync().await {
                    Ok(url) => format!(
                        "\n🔄 <i>Calendar feed updated:</i>\n<code>{}</code>",
                        escape_html(&url)
                    ),
                    Err(e) => format!("\n⚠️ <i>Could not sync Gist: {}</i>", escape_html(&e)),
                };

                let resp = format!(
                    "🏁 <b>Marked course <code>{}</code> as completed!</b>\n\
                     Its classes are now hidden from your schedule, 6:00 AM daily briefings, and .ics calendar feed.{}\n\n\
                     <i>To resume, send <code>/resume {}</code></i>",
                    clean_code, sync_msg, clean_code
                );
                bot.send_message(chat_id, resp)
                    .parse_mode(ParseMode::Html)
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("ℹ️ Course <code>{clean_code}</code> is already marked as completed."),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
        }

        Command::Resume(code) | Command::Restore(code) => {
            let clean_code = code.trim().to_uppercase();
            if clean_code.is_empty() {
                bot.send_message(
                    chat_id,
                    "⚠️ Please provide a course code to resume (e.g. <code>/resume CSE-4125</code>)",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.resume_course(&clean_code) {
                let _ = state.save(&ctx.state_path);
                let sync_msg = match ctx.regenerate_and_sync().await {
                    Ok(url) => format!(
                        "\n🔄 <i>Calendar feed updated:</i>\n<code>{}</code>",
                        escape_html(&url)
                    ),
                    Err(e) => format!("\n⚠️ <i>Could not sync Gist: {}</i>", escape_html(&e)),
                };

                let resp = format!(
                    "✅ <b>Resumed course <code>{}</code>!</b>\n\
                     It will now appear on your active routine, morning briefings, and calendar exports.{}",
                    clean_code, sync_msg
                );
                bot.send_message(chat_id, resp)
                    .parse_mode(ParseMode::Html)
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("ℹ️ Course <code>{clean_code}</code> was not in the completed list."),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
        }

        Command::Completed | Command::Finished | Command::Dropped => {
            let state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.dropped_courses.is_empty() {
                bot.send_message(chat_id, "ℹ️ No courses are currently marked as completed.")
                    .await?;
            } else {
                let raw_sched = load_routine_file(&ctx.routine_path).ok();
                let mut text = "🎓 <b>Currently Completed Courses:</b>\n\n".to_string();
                for c in &state.dropped_courses {
                    let title = raw_sched
                        .as_ref()
                        .and_then(|s| {
                            s.entries
                                .iter()
                                .find(|e| crate::model::state::codes_match(&e.code, c))
                        })
                        .map(|e| e.title.as_str());

                    if let Some(t) = title {
                        text.push_str(&format!(
                            "• <code>{}</code>: <b>{}</b>\n",
                            escape_html(c),
                            escape_html(t)
                        ));
                    } else {
                        text.push_str(&format!("• <code>{}</code>\n", escape_html(c)));
                    }
                }
                text.push_str(
                    "\n<i>Send <code>/resume &lt;CODE&gt;</code> to re-activate any course.</i>",
                );
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }

        Command::Routine | Command::Week => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let (text, markup) = render_routine_overview(&sched);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(markup)
                .await?;
        }

        Command::Alerts(action) => {
            let clean = action.trim().to_lowercase();
            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();

            match clean.as_str() {
                "off" | "pause" | "mute" | "stop" => {
                    state.set_morning_alerts(false);
                    let _ = state.save(&ctx.state_path);
                    let _ = ctx.regenerate_and_sync().await;
                    bot.send_message(
                        chat_id,
                        "🔕 <b>Proactive Morning Briefings Paused!</b>\n\
                         You will not receive 6:00 AM daily alerts.\n\n\
                         <i>Send <code>/alerts on</code> to resume at any time.</i>",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                "on" | "resume" | "unmute" | "start" => {
                    state.set_morning_alerts(true);
                    let _ = state.save(&ctx.state_path);
                    let _ = ctx.regenerate_and_sync().await;
                    bot.send_message(
                        chat_id,
                        "🔔 <b>Proactive Morning Briefings Resumed!</b>\n\
                         You will receive daily briefings at 06:00 AM on class days.",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                _ => {
                    let status = if state.morning_alerts {
                        "🔔 <b>ACTIVE</b> (06:00 AM on class days)"
                    } else {
                        "🔕 <b>PAUSED</b> (no alerts)"
                    };
                    bot.send_message(
                        chat_id,
                        format!(
                            "⏰ <b>Morning Briefing Settings</b>\n\n\
                             Status: {}\n\n\
                             • Send <code>/alerts off</code> to pause.\n\
                             • Send <code>/alerts on</code> to resume.",
                            status
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
            }
        }

        Command::Vacation(param) => {
            let clean = param.trim();
            let lower = clean.to_lowercase();
            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();

            if lower.starts_with("off") || lower.starts_with("resume") || lower.starts_with("stop") {
                state.set_vacation(false, None);
                let _ = state.save(&ctx.state_path);
                let _ = ctx.regenerate_and_sync().await;
                bot.send_message(
                    chat_id,
                    "🎒 <b>Welcome Back! Vacation Mode Deactivated.</b>\n\
                     Normal class queries, countdowns, and 6:00 AM morning briefings are now active.",
                )
                .parse_mode(ParseMode::Html)
                .await?;
            } else if lower.starts_with("on") || lower.starts_with("start") {
                state.set_vacation(true, None);
                let _ = state.save(&ctx.state_path);
                let _ = ctx.regenerate_and_sync().await;
                bot.send_message(
                    chat_id,
                    "🏖️ <b>Vacation / Semester Break Mode Activated!</b>\n\
                     6:00 AM morning briefings and class alerts are paused.\n\n\
                     <i>Send <code>/vacation off</code> when classes resume.</i>",
                )
                .parse_mode(ParseMode::Html)
                .await?;
            } else if lower.starts_with("until") {
                let date_str = clean.trim_start_matches("until").trim();
                match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    Ok(until) => {
                        state.set_vacation(true, Some(until));
                        let _ = state.save(&ctx.state_path);
                        let _ = ctx.regenerate_and_sync().await;
                        bot.send_message(
                            chat_id,
                            format!(
                                "🏖️ <b>Vacation Mode Activated until {}!</b>\n\
                                 Morning alerts and class reminders will automatically resume after this date.\n\n\
                                 <i>Send <code>/vacation off</code> anytime to cancel early.</i>",
                                until
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                    }
                    Err(_) => {
                        bot.send_message(
                            chat_id,
                            "⚠️ Invalid date format. Use <code>YYYY-MM-DD</code> (e.g. <code>/vacation until 2026-11-15</code>).",
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                    }
                }
            } else {
                let status = if state.is_vacation_active() {
                    if let Some(until) = state.vacation_until {
                        format!("🏖️ <b>ACTIVE</b> (until {until})")
                    } else {
                        "🏖️ <b>ACTIVE</b> (Semester Break)".to_string()
                    }
                } else {
                    "🎒 <b>INACTIVE</b> (Normal Semester)".to_string()
                };

                bot.send_message(
                    chat_id,
                    format!(
                        "🏖️ <b>Vacation / Semester Break Mode</b>\n\n\
                         Status: {}\n\n\
                         • Send <code>/vacation on</code> to pause all class alerts.\n\
                         • Send <code>/vacation until YYYY-MM-DD</code> to pause until a date.\n\
                         • Send <code>/vacation off</code> to resume normal classes.",
                        status
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
        }

        Command::Calendar => {
            let (sched, state) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let ics_path = ctx.resolve_ics_path();
            if let Some(parent) = ics_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = export_ics_file(&sched, &ics_path, state.semester_end_date) {
                bot.send_message(chat_id, format!("❌ Failed to generate .ics: {e}"))
                    .await?;
                return Ok(());
            }

            bot.send_document(chat_id, InputFile::file(&ics_path))
                .caption("📅 <b>Your University Class Schedule (.ics)</b>\n\nTap this file on your phone to import all classes and 30-minute alarms directly into your calendar app with 1 tap!")
                .parse_mode(ParseMode::Html)
                .await?;
        }

        Command::Sync => {
            // Check if Gist sync is available (fast, 100% reliable, zero Cloudflare blocks)
            let state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            let has_gist = state.gist_id.is_some() || std::env::var("GIST_ID").is_ok();

            if has_gist {
                bot.send_message(
                    chat_id,
                    "🔄 <b>Synchronizing with Cloud Feed...</b>\nFetching latest timetable from your linked GitHub Gist.",
                )
                .parse_mode(ParseMode::Html)
                .await?;

                match ctx.refresh_from_gist().await {
                    Ok(url) => {
                        let resp = format!(
                            "✅ <b>Routine Successfully Synchronized with Cloud Feed!</b>\n\
                             All class schedules, room assignments, and faculty contacts are up to date.\n\n\
                             🔄 <i>Calendar feed:</i>\n<code>{}</code>\n\n\
                             <i>Send /today or /calendar to view your updated routine.</i>",
                            escape_html(&url)
                        );
                        bot.send_message(chat_id, resp)
                            .parse_mode(ParseMode::Html)
                            .await?;
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Gist refresh notice: {e}. Attempting direct UCAM scrape fallback...");
                    }
                }
            }

            bot.send_message(
                chat_id,
                "🔄 <b>Connecting to UCAM portal...</b>\nPulling the latest class routine, teacher contacts, and room assignments. This takes ~25 seconds.",
            )
            .parse_mode(ParseMode::Html)
            .await?;

            let routine_target = ctx.routine_path.clone();
            let scrape_res = tokio::task::spawn_blocking(move || {
                crate::engine::scraper::run_ucam_scraper(&routine_target)
            })
            .await;

            match scrape_res {
                Ok(Ok(_path)) => {
                    let sync_msg = match ctx.regenerate_and_sync().await {
                        Ok(url) => format!(
                            "\n🔄 <i>Calendar feed updated:</i>\n<code>{}</code>",
                            escape_html(&url)
                        ),
                        Err(e) => format!("\n⚠️ <i>Gist sync notice: {}</i>", escape_html(&e)),
                    };

                    let resp = format!(
                        "✅ <b>UCAM Routine Successfully Synchronized!</b>\n\
                         All class times, room numbers, and instructor contacts are up to date.{}\n\n\
                         <i>Send /today or /calendar to view your updated routine.</i>",
                        sync_msg
                    );
                    bot.send_message(chat_id, resp)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Ok(Err(e)) => {
                    bot.send_message(
                        chat_id,
                        format!(
                            "❌ <b>UCAM Direct Sync Notice:</b>\n<code>{}</code>\n\n\
                             💡 <b>Clean Sync Recommendation:</b>\n\
                             Run <code>cargo run -- sync-ucam</code> on your local PC (clean residential IP) to update your Gist, then tap /sync here to pull updates instantly!",
                            escape_html(&e.to_string())
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ <b>Internal task error:</b>\n{e}"))
                        .await?;
                }
            }
        }
    }

    Ok(())
}

/// Handles interactive inline keyboard button callbacks.
pub async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    ctx: Arc<BotContext>,
) -> ResponseResult<()> {
    let data = match q.data.as_deref() {
        Some(d) => d,
        None => return Ok(()),
    };

    let (sched, state) = match ctx.load_active_schedule() {
        Ok(res) => res,
        Err(e) => {
            let _ = bot
                .answer_callback_query(q.id.clone())
                .text(format!("❌ Error: {e}"))
                .show_alert(true)
                .await;
            return Ok(());
        }
    };

    // Calendar download sends the file directly to the chat
    if data == "nav_calendar" {
        if let Some(msg) = &q.message {
            let chat_id = msg.chat().id;
            let ics_path = ctx.resolve_ics_path();
            if let Some(parent) = ics_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = export_ics_file(&sched, &ics_path, state.semester_end_date) {
                let _ = bot
                    .answer_callback_query(q.id.clone())
                    .text(format!("❌ Failed: {e}"))
                    .show_alert(true)
                    .await;
                return Ok(());
            }

            let _ = bot
                .answer_callback_query(q.id.clone())
                .text("📥 Sending calendar file...")
                .await;

            let _ = bot
                .send_document(chat_id, InputFile::file(&ics_path))
                .caption("📅 <b>Your University Class Schedule (.ics)</b>\n\nTap to import all classes and 30-minute alarms into Apple Calendar or Google Calendar.")
                .parse_mode(ParseMode::Html)
                .await;
            return Ok(());
        }
    }

    let card = match data {
        "nav_today" => render_today_card(&sched, &state),
        "nav_next" => render_next_card(&sched, &state),
        "nav_tomorrow" => render_tomorrow_card(&sched),
        "nav_routine" | "day_all" => render_routine_overview(&sched),
        "nav_teachers" => render_teachers_directory(&sched),
        "nav_free" => render_free_card(&sched, &state),
        "day_sun" => render_routine_day(&sched, DayOfWeek::Sun),
        "day_mon" => render_routine_day(&sched, DayOfWeek::Mon),
        "day_tue" => render_routine_day(&sched, DayOfWeek::Tue),
        "day_wed" => render_routine_day(&sched, DayOfWeek::Wed),
        "day_thu" => render_routine_day(&sched, DayOfWeek::Thu),
        _ => return Ok(()),
    };

    let (text, markup) = card;

    if let Some(msg) = &q.message {
        let chat_id = msg.chat().id;
        let message_id = msg.id();

        let edit_res = bot
            .edit_message_text(chat_id, message_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(markup)
            .await;

        if let Err(ref e) = edit_res {
            let err_str = e.to_string();
            // If the content is identical (e.g. rapid refresh), ignore the error
            if !err_str.contains("message is not modified") {
                eprintln!("⚠️ Failed to update message in-place: {err_str}");
            }
        }
    }

    let _ = bot.answer_callback_query(q.id).await;
    Ok(())
}

/// Escapes HTML special characters for Telegram HTML mode.
#[must_use]
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("Hello & World"), "Hello &amp; World");
        assert_eq!(
            escape_html("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
        assert_eq!(escape_html("Room 612 (Lab)"), "Room 612 (Lab)");
        assert_eq!(escape_html("CSE-4101"), "CSE-4101");
    }

    #[test]
    fn test_command_parsing() {
        assert_eq!(Command::parse("/start", "").unwrap(), Command::Start);
        assert_eq!(Command::parse("/help", "").unwrap(), Command::Help);
        assert_eq!(Command::parse("/today", "").unwrap(), Command::Today);
        assert_eq!(Command::parse("/next", "").unwrap(), Command::Next);
        assert_eq!(Command::parse("/tomorrow", "").unwrap(), Command::Tomorrow);
        assert_eq!(Command::parse("/free", "").unwrap(), Command::Free);
        assert_eq!(Command::parse("/teachers", "").unwrap(), Command::Teachers);
        assert_eq!(Command::parse("/dropped", "").unwrap(), Command::Dropped);
        assert_eq!(Command::parse("/calendar", "").unwrap(), Command::Calendar);
        assert_eq!(Command::parse("/sync", "").unwrap(), Command::Sync);
        assert_eq!(
            Command::parse("/drop CSE-4125", "").unwrap(),
            Command::Drop("CSE-4125".to_string())
        );
        assert_eq!(
            Command::parse("/restore CSE-4125", "").unwrap(),
            Command::Restore("CSE-4125".to_string())
        );
    }

    #[test]
    fn test_bot_context_load_active_schedule() {
        let scraper_path = PathBuf::from(
            "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
        );
        if !scraper_path.exists() {
            return;
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let state_path = temp_dir.path().join("state.json");

        let ctx = BotContext {
            routine_path: scraper_path,
            state_path: state_path.clone(),
            section: Some("B2".to_string()),
            no_http: true,
        };

        // 1. Initial load with section B2
        let (sched, state) = ctx.load_active_schedule().expect("Failed to load schedule");
        assert!(!sched.entries.is_empty());
        assert!(state.dropped_courses.is_empty());
        let initial_count = sched.entries.len();

        // 2. Drop a course (which has 2 weekly class sessions)
        let mut state_to_save = state;
        assert!(state_to_save.drop_course("CSE-4101"));
        state_to_save.save(&state_path).unwrap();

        // 3. Reload schedule and verify course is excluded
        let (filtered_sched, _) = ctx.load_active_schedule().unwrap();
        assert!(!filtered_sched.all_course_codes().contains("CSE-4101"));
        assert!(filtered_sched.entries.len() < initial_count);

        // 4. Restore the course using space-separated format
        let mut state_to_restore = RoutineState::load_or_default(&state_path).unwrap();
        assert!(state_to_restore.restore_course("cse 4101"));
        state_to_restore.save(&state_path).unwrap();

        // 5. Reload and verify restored
        let (restored_sched, _) = ctx.load_active_schedule().unwrap();
        assert_eq!(restored_sched.entries.len(), initial_count);
        assert!(restored_sched.all_course_codes().contains("CSE-4101"));
    }
}
