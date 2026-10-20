//! Telegram Bot service for interactive schedule queries and 6:00 AM morning alerts.

pub mod handlers;
pub mod scheduler;

pub use handlers::{handle_command, BotContext, Command};
pub use scheduler::start_morning_scheduler;

use std::sync::Arc;
use teloxide::prelude::*;

/// Launches the interactive Telegram bot daemon and 6:00 AM morning scheduler.
///
/// # Errors
/// Returns an error if the Telegram token is missing or if connection fails.
pub async fn run_bot_daemon(ctx: BotContext) -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Support both TELOXIDE_TOKEN and TELEGRAM_BOT_TOKEN
    if std::env::var("TELOXIDE_TOKEN").is_err() {
        if let Ok(token) = std::env::var("TELEGRAM_BOT_TOKEN") {
            std::env::set_var("TELOXIDE_TOKEN", token);
        } else {
            return Err("Missing TELEGRAM_BOT_TOKEN environment variable. Create a .env file with your token.".into());
        }
    }

    let bot = Bot::from_env();
    let shared_ctx = Arc::new(ctx);

    // Spawn 6:00 AM scheduler if TELEGRAM_CHAT_ID is set
    if let Ok(chat_id_str) = std::env::var("TELEGRAM_CHAT_ID") {
        if let Ok(chat_id) = chat_id_str.trim().parse::<i64>() {
            let sched_bot = bot.clone();
            let sched_ctx = Arc::clone(&shared_ctx);
            tokio::spawn(async move {
                start_morning_scheduler(sched_bot, chat_id, sched_ctx).await;
            });
            println!("🔔 Proactive 06:00 AM morning briefings active for Chat ID: {chat_id}");
        }
    }

    println!("🤖 Academic Routine Telegram Bot is online and listening for messages!");
    println!("   Commands supported: /today, /next, /tomorrow, /free, /drop, /restore, /calendar");

    let handler_ctx = Arc::clone(&shared_ctx);
    Command::repl(bot, move |b: Bot, msg: Message, cmd: Command| {
        let c = Arc::clone(&handler_ctx);
        async move { handle_command(b, msg, cmd, c).await }
    })
    .await;

    Ok(())
}
