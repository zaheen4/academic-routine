//! Scheduled morning briefings and proactive notifications.

use crate::bot::handlers::{escape_html, BotContext};
use crate::model::day::DayOfWeek;
use chrono::{Datelike, Local, NaiveTime};
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

    let today = match current_academic_day() {
        Some(d) => d,
        None => return,
    };

    let (sched, _) = match ctx.load_active_schedule() {
        Ok(res) => res,
        Err(e) => {
            eprintln!("Scheduler error loading schedule: {e}");
            return;
        }
    };

    let classes = sched.for_day(today);
    if classes.is_empty() {
        return; // Don't disturb on days without classes
    }

    let mut msg = format!(
        "🌅 <b>Good Morning! Academic Daily Briefing</b>\n\n\
         Today is <b>{}</b> — you have <b>{}</b> class(es) scheduled:\n\n",
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

    let _ = bot
        .send_message(ChatId(target_chat_id), msg)
        .parse_mode(teloxide::types::ParseMode::Html)
        .await;
}

fn duration_until_next_6am() -> std::time::Duration {
    let now = Local::now();
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
