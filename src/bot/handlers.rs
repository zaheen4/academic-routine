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
use teloxide::types::InputFile;
use teloxide::utils::command::BotCommands;

/// Available Telegram bot commands.
#[derive(BotCommands, Clone, Debug)]
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
    pub fn regenerate_and_sync(&self) -> Result<String, String> {
        let (sched, state) = self.load_active_schedule()?;
        let ics_path = PathBuf::from("routine.ics");

        export_ics_file(&sched, &ics_path, state.semester_end_date)
            .map_err(|e| format!("Failed to export .ics: {e}"))?;

        if let Some(ref gid) = state.gist_id {
            let output = std::process::Command::new("gh")
                .args(["gist", "edit", gid, "routine.ics"])
                .output();

            if let Ok(out) = output {
                if out.status.success() {
                    return Ok(format!(
                        "https://gist.githubusercontent.com/raw/{gid}/routine.ics"
                    ));
                }
            }
        }

        Ok("routine.ics".to_string())
    }
}

/// Dispatches an incoming Telegram command.
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
                "🎓 *Welcome to your Academic Routine Assistant!*\n\n\
                 {}\n\n\
                 💡 *Tips:*\n\
                 • Send `/today` to see your schedule for today.\n\
                 • Send `/next` to see what class is coming up next.\n\
                 • Send `/calendar` to receive the `.ics` file directly on your phone!\n\
                 • Send `/drop <CODE>` to drop a course on the fly.",
                Command::descriptions()
            );
            bot.send_message(chat_id, help_text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
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
                    format!("🎉 No classes scheduled for today ({today})! Enjoy your free time."),
                )
                .await?;
                return Ok(());
            }

            let mut text = format!(
                "📅 *Schedule for Today ({})* — {} classes\n\n",
                today.full_name(),
                classes.len()
            );
            for c in classes {
                let lab_badge = if c.is_lab() { " 🔬 _[LAB]_" } else { "" };
                text.push_str(&format!(
                    "• *{}* \\| Room *{}*\n  └ `{}`: *{}*{}\n",
                    escape_md(&c.time_slot.format_12h()),
                    escape_md(&c.room),
                    escape_md(&c.code),
                    escape_md(&c.title),
                    lab_badge
                ));
                if !c.teacher.name.is_empty() {
                    text.push_str(&format!("    👤 {}\n", escape_md(&c.teacher.name)));
                }
                text.push('\n');
            }

            bot.send_message(chat_id, text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
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
                    "🔴 *CURRENTLY IN SESSION*\n\n\
                     • Course: *{}* ({})\n\
                     • Room: *Room {}*\n\
                     • Time: *{}* \\(ends in {} mins\\)\n\
                     • Teacher: {}",
                    escape_md(&active.code),
                    escape_md(&active.title),
                    escape_md(&active.room),
                    escape_md(&active.time_slot.format_12h()),
                    mins_left,
                    escape_md(&active.teacher.name)
                );
                bot.send_message(chat_id, msg_text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .await?;
            } else if let Some(next) = classes.iter().find(|c| c.time_slot.start() > now) {
                let mins_until = (next.time_slot.start().hour() * 60
                    + next.time_slot.start().minute())
                .saturating_sub(now.hour() * 60 + now.minute());
                let msg_text = format!(
                    "🟢 *NEXT UPCOMING CLASS*\n\n\
                     • Course: *{}* ({})\n\
                     • Room: *Room {}*\n\
                     • Time: *{}* \\(starts in {} mins\\)\n\
                     • Teacher: {}",
                    escape_md(&next.code),
                    escape_md(&next.title),
                    escape_md(&next.room),
                    escape_md(&next.time_slot.format_12h()),
                    mins_until,
                    escape_md(&next.teacher.name)
                );
                bot.send_message(chat_id, msg_text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
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
                "🌅 *Preview for Tomorrow ({})* — {} classes\n\n",
                tomorrow.full_name(),
                classes.len()
            );
            for c in classes {
                let lab_badge = if c.is_lab() { " 🔬 _[LAB]_" } else { "" };
                text.push_str(&format!(
                    "• *{}* \\| Room *{}*\n  └ `{}`: *{}*{}\n",
                    escape_md(&c.time_slot.format_12h()),
                    escape_md(&c.room),
                    escape_md(&c.code),
                    escape_md(&c.title),
                    lab_badge
                ));
            }

            bot.send_message(chat_id, text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
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
                            "☕ *{}* \\({} to {}\\)\n   Between `{}` and `{}`",
                            escape_md(&duration_str),
                            escape_md(&cur.time_slot.end().format("%I:%M %p").to_string()),
                            escape_md(&next.time_slot.start().format("%I:%M %p").to_string()),
                            escape_md(&cur.code),
                            escape_md(&next.code)
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
                let mut text = format!("☕ *Break Windows for Today ({})*:\n\n", today.full_name());
                for b in breaks {
                    text.push_str(&format!("• {b}\n\n"));
                }
                bot.send_message(chat_id, text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
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
                "📚 *Instructor Directory* \\({} teachers\\):\n\n",
                dir.len()
            );
            for (name, contact) in dir {
                text.push_str(&format!("• *{}*\n", escape_md(&name)));
                if let Some(ref p) = contact.phone {
                    text.push_str(&format!("  ☎ Phone: `{}`\n", escape_md(p)));
                }
                if let Some(ref e) = contact.email {
                    text.push_str(&format!("  ✉ Email: `{}`\n", escape_md(e)));
                }
                text.push('\n');
            }

            bot.send_message(chat_id, text)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .await?;
        }

        Command::Drop(code) => {
            let clean_code = code.trim().to_uppercase();
            if clean_code.is_empty() {
                bot.send_message(
                    chat_id,
                    "⚠️ Please provide a course code to drop (e.g. `/drop CSE-4101`)",
                )
                .await?;
                return Ok(());
            }

            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.drop_course(&clean_code) {
                let _ = state.save(&ctx.state_path);
                let sync_msg = match ctx.regenerate_and_sync() {
                    Ok(url) => format!("\n🔄 _Calendar feed updated:_\n`{}`", escape_md(&url)),
                    Err(e) => format!("\n⚠️ _Could not sync Gist: {}_", escape_md(&e)),
                };

                let resp = format!(
                    "✅ *Dropped course `{}`!*\n\
                     It has been excluded from your schedule, Telegram alerts, and `.ics` feed.{}\n\n\
                     _To undo, send `/restore {}`_",
                    escape_md(&clean_code),
                    sync_msg,
                    escape_md(&clean_code)
                );
                bot.send_message(chat_id, resp)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("ℹ️ Course `{clean_code}` is already marked as dropped."),
                )
                .await?;
            }
        }

        Command::Restore(code) => {
            let clean_code = code.trim().to_uppercase();
            if clean_code.is_empty() {
                bot.send_message(
                    chat_id,
                    "⚠️ Please provide a course code to restore (e.g. `/restore CSE-4101`)",
                )
                .await?;
                return Ok(());
            }

            let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.restore_course(&clean_code) {
                let _ = state.save(&ctx.state_path);
                let sync_msg = match ctx.regenerate_and_sync() {
                    Ok(url) => format!("\n🔄 _Calendar feed updated:_\n`{}`", escape_md(&url)),
                    Err(e) => format!("\n⚠️ _Could not sync Gist: {}_", escape_md(&e)),
                };

                let resp = format!(
                    "✅ *Restored course `{}`!*\n\
                     It will now appear on your schedule and calendar exports.{}",
                    escape_md(&clean_code),
                    sync_msg
                );
                bot.send_message(chat_id, resp)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("ℹ️ Course `{clean_code}` was not in the dropped list."),
                )
                .await?;
            }
        }

        Command::Dropped => {
            let state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();
            if state.dropped_courses.is_empty() {
                bot.send_message(chat_id, "ℹ️ No courses are currently dropped.")
                    .await?;
            } else {
                let mut text = "🚫 *Currently Dropped Courses:*\n\n".to_string();
                for c in &state.dropped_courses {
                    text.push_str(&format!("• `{}`\n", escape_md(c)));
                }
                text.push_str("\n_Send `/restore <CODE>` to re-enable any course._");
                bot.send_message(chat_id, text)
                    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
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
                .caption("📅 *Your University Class Schedule (.ics)*\n\nTap this file on your phone to import all classes and 15-minute alarms directly into your calendar app with 1 tap!")
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .await?;
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

/// Escapes reserved characters for Telegram MarkdownV2 format.
pub fn escape_md(text: &str) -> String {
    let reserved = [
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if reserved.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
