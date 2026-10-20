//! Telegram command handlers and interactive assistant logic.

use crate::engine::parser::load_routine_file;
use crate::exporters::ics::export_ics_file;
use crate::model::day::DayOfWeek;
use crate::model::schedule::Schedule;
use crate::model::state::RoutineState;
use chrono::{Datelike, Local, Timelike};
use std::path::PathBuf;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InputFile, ParseMode};
use teloxide::utils::command::BotCommands;

/// Available Telegram bot commands.
#[derive(BotCommands, Clone, Debug, PartialEq, Eq)]
#[command(
    rename_rule = "lowercase",
    description = "🎓 University Routine Assistant Commands:"
)]
pub enum Command {
    /// Display help and available commands
    #[command(description = "Show help and command instructions")]
    Help,
    /// Welcome message
    #[command(description = "Start the assistant")]
    Start,
    /// View today's schedule
    #[command(description = "Show today's schedule and active session")]
    Today,
    /// Show upcoming class with countdown
    #[command(description = "Show upcoming or in-session class with countdown")]
    Next,
    /// Preview tomorrow's schedule
    #[command(description = "Preview tomorrow's schedule")]
    Tomorrow,
    /// Calculate free break periods today
    #[command(description = "Show break and study windows for today")]
    Free,
    /// Instructor contact directory
    #[command(description = "List instructors with phone numbers and emails")]
    Teachers,
    /// Drop a course (e.g. /drop CSE-4101)
    #[command(description = "Drop a course (e.g. /drop CSE-4101)")]
    Drop(String),
    /// Restore a dropped course (e.g. /restore CSE-4101)
    #[command(description = "Restore a dropped course (e.g. /restore CSE-4101)")]
    Restore(String),
    /// List all currently dropped courses
    #[command(description = "List all currently dropped/hidden courses")]
    Dropped,
    /// Send .ics calendar file for 1-tap phone importing
    #[command(description = "Send routine.ics file directly to chat for 1-tap calendar import")]
    Calendar,
    /// Refresh routine directly from UCAM portal and sync calendar
    #[command(description = "Refresh routine directly from UCAM portal and update calendar")]
    Sync,
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

    /// Regenerates routine.ics and syncs to GitHub Gist if configured.
    pub async fn regenerate_and_sync(&self) -> Result<String, String> {
        let (sched, state) = self.load_active_schedule()?;
        let ics_path = PathBuf::from("routine.ics");

        export_ics_file(&sched, &ics_path, state.semester_end_date)
            .map_err(|e| format!("Failed to export .ics: {e}"))?;

        if let Some(ref gid) = state.gist_id {
            return crate::exporters::gist::sync_gist_file(gid, &ics_path).await;
        }

        Ok("routine.ics".to_string())
    }

    /// Refreshes routine_data.json from GitHub Gist, writes it locally, and updates calendar feed.
    pub async fn refresh_from_gist(&self) -> Result<String, String> {
        let state = RoutineState::load_or_default(&self.state_path).unwrap_or_default();
        let gid = match state.gist_id.as_deref() {
            Some(id) => id.to_string(),
            None => std::env::var("GIST_ID")
                .map_err(|_| "No Gist ID configured in state or environment.".to_string())?,
        };

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

/// Dispatches an incoming Telegram command using reliable HTML formatting.
pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    ctx: Arc<BotContext>,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id;

    match cmd {
        Command::Help | Command::Start => {
            let help_text = format!(
                "🎓 <b>Welcome to your Academic Routine Assistant!</b>\n\n\
                 {}\n\n\
                 💡 <b>Quick Tips:</b>\n\
                 • Send <code>/today</code> to see your schedule for today.\n\
                 • Send <code>/next</code> to see what class is coming up next.\n\
                 • Send <code>/tomorrow</code> to preview tomorrow's classes.\n\
                 • Send <code>/free</code> to find breaks between classes.\n\
                 • Send <code>/calendar</code> to receive the .ics file on your phone!\n\
                 • Send <code>/drop &lt;CODE&gt;</code> to drop a course on the fly.",
                escape_html(&Command::descriptions().to_string())
            );
            bot.send_message(chat_id, help_text)
                .parse_mode(ParseMode::Html)
                .await?;
        }

        Command::Today => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let today = current_academic_day().unwrap_or(DayOfWeek::Sat);
            let classes = sched.for_day(today);

            if classes.is_empty() {
                bot.send_message(
                    chat_id,
                    format!(
                        "🎉 No classes scheduled for today ({})! Enjoy your free time.",
                        today.full_name()
                    ),
                )
                .await?;
                return Ok(());
            }

            let mut text = format!(
                "📅 <b>Schedule for Today ({})</b> — {} class(es)\n\n",
                today.full_name(),
                classes.len()
            );

            for c in classes {
                let lab_badge = if c.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
                text.push_str(&format!(
                    "• <b>{}</b> | Room <b>{}</b>\n  └ <code>{}</code>: <b>{}</b>{}\n",
                    c.time_slot.format_12h(),
                    c.room,
                    c.code,
                    escape_html(&c.title),
                    lab_badge
                ));
                if !c.teacher.name.is_empty() {
                    text.push_str(&format!("    👤 {}\n", escape_html(&c.teacher.name)));
                }
                text.push('\n');
            }

            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }

        Command::Next => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let today = current_academic_day().unwrap_or(DayOfWeek::Sat);
            let now = Local::now().time();
            let classes = sched.for_day(today);

            if let Some(active) = classes.iter().find(|c| c.time_slot.contains_time(now)) {
                let mins_left = (active.time_slot.end().hour() * 60
                    + active.time_slot.end().minute())
                .saturating_sub(now.hour() * 60 + now.minute());
                let msg_text = format!(
                    "🔴 <b>CURRENTLY IN SESSION</b>\n\n\
                     • Course: <b>{}</b> ({})\n\
                     • Room: <b>Room {}</b>\n\
                     • Time: <b>{}</b> (ends in {} mins)\n\
                     • Teacher: {}",
                    active.code,
                    escape_html(&active.title),
                    active.room,
                    active.time_slot.format_12h(),
                    mins_left,
                    escape_html(&active.teacher.name)
                );
                bot.send_message(chat_id, msg_text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            } else if let Some(next) = classes.iter().find(|c| c.time_slot.start() > now) {
                let mins_until = (next.time_slot.start().hour() * 60
                    + next.time_slot.start().minute())
                .saturating_sub(now.hour() * 60 + now.minute());
                let msg_text = format!(
                    "🟢 <b>NEXT UPCOMING CLASS</b>\n\n\
                     • Course: <b>{}</b> ({})\n\
                     • Room: <b>Room {}</b>\n\
                     • Time: <b>{}</b> (starts in {} mins)\n\
                     • Teacher: {}",
                    next.code,
                    escape_html(&next.title),
                    next.room,
                    next.time_slot.format_12h(),
                    mins_until,
                    escape_html(&next.teacher.name)
                );
                bot.send_message(chat_id, msg_text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    "🏁 All classes for today have concluded! Good job.",
                )
                .await?;
            }
        }

        Command::Tomorrow => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let tomorrow = next_academic_day();
            let classes = sched.for_day(tomorrow);

            if classes.is_empty() {
                bot.send_message(
                    chat_id,
                    format!(
                        "🎉 No classes scheduled for tomorrow ({})! Have a restful evening.",
                        tomorrow.full_name()
                    ),
                )
                .await?;
                return Ok(());
            }

            let mut text = format!(
                "🌅 <b>Preview for Tomorrow ({})</b> — {} class(es)\n\n",
                tomorrow.full_name(),
                classes.len()
            );

            for c in classes {
                let lab_badge = if c.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
                text.push_str(&format!(
                    "• <b>{}</b> | Room <b>{}</b>\n  └ <code>{}</code>: <b>{}</b>{}\n",
                    c.time_slot.format_12h(),
                    c.room,
                    c.code,
                    escape_html(&c.title),
                    lab_badge
                ));
            }

            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }

        Command::Free => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let today = current_academic_day().unwrap_or(DayOfWeek::Sat);
            let classes = sched.for_day(today);

            if classes.len() < 2 {
                bot.send_message(
                    chat_id,
                    "ℹ️ You have fewer than 2 classes today, so no in-between breaks exist.",
                )
                .await?;
                return Ok(());
            }

            let mut breaks = Vec::new();
            for i in 0..classes.len() - 1 {
                let cur = classes[i];
                let next = classes[i + 1];
                if let Some(gap) = cur.time_slot.gap_between(&next.time_slot) {
                    if gap > 0 {
                        let hours = gap / 60;
                        let mins = gap % 60;
                        let duration_str = if hours > 0 && mins > 0 {
                            format!("{hours}h {mins}m")
                        } else if hours > 0 {
                            format!("{hours} hour(s)")
                        } else {
                            format!("{mins} min(s)")
                        };

                        breaks.push(format!(
                            "☕ <b>{}</b> ({} to {})\n   Between <code>{}</code> and <code>{}</code>",
                            duration_str,
                            cur.time_slot.end().format("%I:%M %p"),
                            next.time_slot.start().format("%I:%M %p"),
                            cur.code,
                            next.code
                        ));
                    }
                }
            }

            if breaks.is_empty() {
                bot.send_message(
                    chat_id,
                    "ℹ️ All classes today are back-to-back with no breaks in between.",
                )
                .await?;
            } else {
                let mut text = format!(
                    "☕ <b>Break Windows for Today ({})</b>:\n\n",
                    today.full_name()
                );
                for b in breaks {
                    text.push_str(&format!("• {b}\n\n"));
                }
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }

        Command::Teachers => {
            let (sched, _) = match ctx.load_active_schedule() {
                Ok(res) => res,
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ {e}")).await?;
                    return Ok(());
                }
            };

            let dir = sched.teacher_directory();
            let mut text = format!(
                "📚 <b>Instructor Directory</b> ({} teachers):\n\n",
                dir.len()
            );
            for (name, contact) in dir {
                text.push_str(&format!("• <b>{}</b>\n", escape_html(&name)));
                if let Some(ref p) = contact.phone {
                    text.push_str(&format!("  ☎ Phone: <code>{}</code>\n", escape_html(p)));
                }
                if let Some(ref e) = contact.email {
                    text.push_str(&format!("  ✉ Email: <code>{}</code>\n", escape_html(e)));
                }
                text.push('\n');
            }

            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }

        Command::Drop(code) => {
            let clean_code = code.trim().to_uppercase();
            if clean_code.is_empty() {
                bot.send_message(
                    chat_id,
                    "⚠️ Please provide a course code to drop (e.g. <code>/drop CSE-4101</code>)",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.drop_course(&clean_code) {
                let _ = state.save(&ctx.state_path);
                let sync_msg = match ctx.regenerate_and_sync().await {
                    Ok(url) => format!(
                        "\n🔄 <i>Calendar feed updated:</i>\n<code>{}</code>",
                        escape_html(&url)
                    ),
                    Err(e) => format!("\n⚠️ <i>Could not sync Gist: {}</i>", escape_html(&e)),
                };

                let resp = format!(
                    "✅ <b>Dropped course <code>{}</code>!</b>\n\
                     It has been excluded from your schedule, Telegram alerts, and .ics feed.{}\n\n\
                     <i>To undo, send <code>/restore {}</code></i>",
                    clean_code, sync_msg, clean_code
                );
                bot.send_message(chat_id, resp)
                    .parse_mode(ParseMode::Html)
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("ℹ️ Course <code>{clean_code}</code> is already marked as dropped."),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
        }

        Command::Restore(code) => {
            let clean_code = code.trim().to_uppercase();
            if clean_code.is_empty() {
                bot.send_message(chat_id, "⚠️ Please provide a course code to restore (e.g. <code>/restore CSE-4101</code>)")
                    .parse_mode(ParseMode::Html)
                    .await?;
                return Ok(());
            }

            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.restore_course(&clean_code) {
                let _ = state.save(&ctx.state_path);
                let sync_msg = match ctx.regenerate_and_sync().await {
                    Ok(url) => format!(
                        "\n🔄 <i>Calendar feed updated:</i>\n<code>{}</code>",
                        escape_html(&url)
                    ),
                    Err(e) => format!("\n⚠️ <i>Could not sync Gist: {}</i>", escape_html(&e)),
                };

                let resp = format!(
                    "✅ <b>Restored course <code>{}</code>!</b>\n\
                     It will now appear on your schedule and calendar exports.{}",
                    clean_code, sync_msg
                );
                bot.send_message(chat_id, resp)
                    .parse_mode(ParseMode::Html)
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("ℹ️ Course <code>{clean_code}</code> was not in the dropped list."),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
        }

        Command::Dropped => {
            let state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.dropped_courses.is_empty() {
                bot.send_message(chat_id, "ℹ️ No courses are currently dropped.")
                    .await?;
            } else {
                let mut text = "🚫 <b>Currently Dropped Courses:</b>\n\n".to_string();
                for c in &state.dropped_courses {
                    text.push_str(&format!("• <code>{}</code>\n", escape_html(c)));
                }
                text.push_str(
                    "\n<i>Send <code>/restore &lt;CODE&gt;</code> to re-enable any course.</i>",
                );
                bot.send_message(chat_id, text)
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

            let ics_path = PathBuf::from("routine.ics");
            if let Err(e) = export_ics_file(&sched, &ics_path, state.semester_end_date) {
                bot.send_message(chat_id, format!("❌ Failed to generate .ics: {e}"))
                    .await?;
                return Ok(());
            }

            bot.send_document(chat_id, InputFile::file(&ics_path))
                .caption("📅 <b>Your University Class Schedule (.ics)</b>\n\nTap this file on your phone to import all classes and 15-minute alarms directly into your calendar app with 1 tap!")
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

fn current_academic_day() -> Option<DayOfWeek> {
    let now = Local::now();
    match now.weekday() {
        chrono::Weekday::Sat => Some(DayOfWeek::Sat),
        chrono::Weekday::Sun => Some(DayOfWeek::Sun),
        chrono::Weekday::Mon => Some(DayOfWeek::Mon),
        chrono::Weekday::Tue => Some(DayOfWeek::Tue),
        chrono::Weekday::Wed => Some(DayOfWeek::Wed),
        chrono::Weekday::Thu => Some(DayOfWeek::Thu),
        chrono::Weekday::Fri => Some(DayOfWeek::Fri),
    }
}

fn next_academic_day() -> DayOfWeek {
    let now = Local::now();
    match now.weekday() {
        chrono::Weekday::Fri => DayOfWeek::Sat,
        chrono::Weekday::Sat => DayOfWeek::Sun,
        chrono::Weekday::Sun => DayOfWeek::Mon,
        chrono::Weekday::Mon => DayOfWeek::Tue,
        chrono::Weekday::Tue => DayOfWeek::Wed,
        chrono::Weekday::Wed => DayOfWeek::Thu,
        chrono::Weekday::Thu => DayOfWeek::Fri,
    }
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
