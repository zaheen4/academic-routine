//! Scheduled morning briefings and proactive notifications.

use crate::bot::handlers::{escape_html, BotContext};
use crate::model::day::{academic_now, current_academic_day};
use crate::model::state::RoutineState;
use chrono::NaiveTime;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatId;

/// Starts the morning briefing scheduler daemon at 6:00 AM every class day.
pub async fn start_morning_scheduler(bot: Bot, target_chat_id: i64, ctx: Arc<BotContext>) {
    loop {
        let sleep_duration = duration_until_next_6am();
        let hours = sleep_duration.as_secs() / 3600;
        let mins = (sleep_duration.as_secs() % 3600) / 60;
        println!(
            "⏰ Scheduler: Next morning briefing at 06:00 AM (in {}h {}m)",
            hours, mins
        );

        tokio::time::sleep(sleep_duration).await;

        // Trigger briefing
        send_morning_briefing(&bot, target_chat_id, &ctx).await;

        // Sleep 15 seconds to ensure we do not double-trigger within the same minute
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    }
}

async fn send_morning_briefing(bot: &Bot, target_chat_id: i64, ctx: &Arc<BotContext>) {
    // Attempt automated refresh from Gist so briefings always reflect the latest routine
    if let Err(e) = ctx.refresh_from_gist().await {
        eprintln!("Pre-briefing Gist sync note: {e}");
    }

    let mut state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();

    if state.is_vacation_active() {
        println!("⏰ Scheduler: Vacation mode is active. Skipping 06:00 AM morning briefing.");
        return;
    }

    if !state.morning_alerts {
        println!("⏰ Scheduler: Morning alerts globally paused. Skipping 06:00 AM morning briefing.");
        return;
    }

    let mut recipient_chats = state.all_registered_chats();
    if target_chat_id > 0 {
        recipient_chats.insert(target_chat_id);
    }

    if recipient_chats.is_empty() {
        return;
    }

    let today = current_academic_day();
    let mut state_dirty = false;

    for &cid in &recipient_chats {
        if !state.is_morning_alerts_enabled(cid) {
            continue;
        }

        let (sched, user_state) = match ctx.load_active_schedule_for_chat(cid) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Scheduler error loading schedule for chat {cid}: {e}");
                continue;
            }
        };

        let classes = sched.for_day(today);
        if classes.is_empty() {
            continue; // Don't disturb on days without classes for this section
        }

        let section_badge = match user_state.get_user_section(cid) {
            Some(sec) if !sec.eq_ignore_ascii_case("all") => format!(" • Section <b>{sec}</b>"),
            _ => String::new(),
        };

        let mut msg = format!(
            "🌅 <b>Good Morning! Academic Daily Briefing</b>{}\n\n\
             Today is <b>{}</b> — you have <b>{}</b> class(es) scheduled:\n\n",
            section_badge,
            today.full_name(),
            classes.len()
        );

        for (i, c) in classes.iter().enumerate() {
            let lab_badge = if c.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
            msg.push_str(&format!(
                "{}. <b>{}</b> | Room <b>{}</b>\n   └ <code>{}</code>: <b>{}</b>{}\n",
                i + 1,
                escape_html(&c.time_slot.format_12h()),
                escape_html(&c.room),
                escape_html(&c.code),
                escape_html(&c.title),
                lab_badge
            ));
            if !c.teacher.name.is_empty() {
                msg.push_str(&format!("     👤 {}\n", escape_html(&c.teacher.name)));
            }
            msg.push('\n');
        }

        msg.push_str("💡 <i>Have a productive day! Send /today or /next anytime for updates.</i>");

        let send_res = bot
            .send_message(ChatId(cid), msg)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;

        if let Err(ref e) = send_res {
            let err_msg = e.to_string().to_lowercase();
            if err_msg.contains("blocked by the user")
                || err_msg.contains("chat not found")
                || err_msg.contains("user is deactivated")
            {
                eprintln!("Chat {cid} blocked the bot or is invalid; muting morning alerts.");
                state.set_user_morning_alerts(cid, false);
                state_dirty = true;
            } else {
                eprintln!("Failed to send morning briefing to {cid}: {e}");
            }
        }

        // 35ms sleep to prevent Telegram rate limit / 429 flood issues across multiple users
        tokio::time::sleep(std::time::Duration::from_millis(35)).await;
    }

    if state_dirty {
        let _ = state.save(&ctx.state_path);
    }
}

fn duration_until_next_6am() -> std::time::Duration {
    let now = academic_now();
    let target_time = NaiveTime::from_hms_opt(6, 0, 0).unwrap();

    let target_dt = if now.time() < target_time {
        now.date_naive().and_time(target_time)
    } else {
        (now.date_naive() + chrono::Duration::days(1)).and_time(target_time)
    };

    let now_naive = now.naive_local();
    let diff_seconds = (target_dt - now_naive).num_seconds().max(1) as u64;

    std::time::Duration::from_secs(diff_seconds)
}

/// Proactively monitors upcoming classes for today and sends a Telegram reminder 30 minutes before each class starts.
pub async fn start_class_reminder_daemon(bot: Bot, target_chat_id: i64, ctx: Arc<BotContext>) {
    use chrono::Timelike;
    let mut notified_sessions = std::collections::HashSet::<String>::new();
    let mut current_day = current_academic_day();

    loop {
        // Check every 30 seconds
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

        let today = current_academic_day();
        if today != current_day {
            // New academic day: reset notified sessions
            notified_sessions.clear();
            current_day = today;
        }

        let state = RoutineState::load_or_default(&ctx.state_path).unwrap_or_default();

        if state.is_vacation_active() || !state.morning_alerts {
            continue;
        }

        let mut candidate_chats = state.all_registered_chats();
        if target_chat_id > 0 {
            candidate_chats.insert(target_chat_id);
        }

        // Filter to only chats that have explicitly opted into 30-minute pre-class reminders
        candidate_chats.retain(|&cid| state.is_preclass_reminders_enabled(cid));
        if candidate_chats.is_empty() {
            continue;
        }

        let now = academic_now();
        let now_time = now.time();
        let now_mins = now_time.hour() * 60 + now_time.minute();

        for &cid in &candidate_chats {
            let (sched, _) = match ctx.load_active_schedule_for_chat(cid) {
                Ok(res) => res,
                Err(_) => continue,
            };

            let classes = sched.for_day(today);
            for c in &classes {
                let start_mins = c.time_slot.start().hour() * 60 + c.time_slot.start().minute();
                let session_key = format!(
                    "{cid}-{today}-{}-{}-{}-{}",
                    c.code,
                    c.section,
                    c.room,
                    c.time_slot.start()
                );

                if start_mins >= now_mins {
                    let diff = start_mins - now_mins;
                    // Alert in the 29-30 minute window
                    if (29..=30).contains(&diff) && !notified_sessions.contains(&session_key) {
                        notified_sessions.insert(session_key);

                        let lab_badge = if c.is_lab() { " 🔬 <i>[LAB]</i>" } else { "" };
                        let mut msg = format!(
                            "⏰ <b>Upcoming Class Reminder (Starts in {} minutes!)</b>\n\n\
                             • Course: <b>[{}] {}</b>{}\n\
                             • Code: <code>{}</code>\n\
                             • Room: <b>Room {}</b>\n\
                             • Time: <b>{} – {}</b>\n",
                            diff,
                            escape_html(&c.section),
                            escape_html(&c.title),
                            lab_badge,
                            escape_html(&c.code),
                            escape_html(&c.room),
                            c.time_slot.start().format("%I:%M %p"),
                            c.time_slot.end().format("%I:%M %p")
                        );

                        if !c.teacher.name.is_empty() {
                            msg.push_str(&format!(
                                "• Instructor: {}\n",
                                escape_html(&c.teacher.formatted_summary())
                            ));
                        }

                        let _ = bot
                            .send_message(ChatId(cid), msg)
                            .parse_mode(teloxide::types::ParseMode::Html)
                            .await;

                        // 35ms spacing for multi-user rate limit safety
                        tokio::time::sleep(std::time::Duration::from_millis(35)).await;
                    }
                }
            }
        }
    }
}
