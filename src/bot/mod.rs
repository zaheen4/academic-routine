//! Telegram Bot service for interactive schedule queries and 6:00 AM morning alerts.

pub mod handlers;
pub mod keyboards;
pub mod scheduler;

pub use handlers::{handle_callback_query, handle_command, BotContext, Command};
pub use scheduler::{start_class_reminder_daemon, start_morning_scheduler};

use std::sync::Arc;
use teloxide::dispatching::UpdateFilterExt;
use teloxide::prelude::*;
use teloxide::types::Update;
use teloxide::utils::command::BotCommands;

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

    // Register command menu popup with Telegram (enables the "/" suggestion popup)
    if let Err(e) = bot.set_my_commands(Command::bot_commands()).await {
        eprintln!("⚠️ Failed to register bot command menu: {e}");
    } else {
        println!("✨ Successfully registered command menu with Telegram ('/' suggestions active)!");
    }

    let shared_ctx = Arc::new(ctx);

    // Spawn 6:00 AM scheduler if TELEGRAM_CHAT_ID is set
    if let Ok(chat_id_str) = std::env::var("TELEGRAM_CHAT_ID") {
        if let Ok(chat_id) = chat_id_str.trim().parse::<i64>() {
            let sched_bot = bot.clone();
            let sched_ctx = Arc::clone(&shared_ctx);
            tokio::spawn(async move {
                start_morning_scheduler(sched_bot, chat_id, sched_ctx).await;
            });

            // Spawn 30-minute before-class proactive reminder daemon
            let reminder_bot = bot.clone();
            let reminder_ctx = Arc::clone(&shared_ctx);
            tokio::spawn(async move {
                start_class_reminder_daemon(reminder_bot, chat_id, reminder_ctx).await;
            });

            // Inspect Telegram handle on startup to cache into routine_state.json
            let info_bot = bot.clone();
            let state_file = shared_ctx.state_path.clone();
            tokio::spawn(async move {
                if let Ok(chat) = info_bot.get_chat(teloxide::types::ChatId(chat_id)).await {
                    let handle = chat
                        .username()
                        .map(|u| format!("@{u}"))
                        .or_else(|| chat.first_name().map(|n| n.to_string()));
                    if let Some(h) = handle {
                        let mut state =
                            crate::model::state::RoutineState::load_or_default(&state_file)
                                .unwrap_or_default();
                        if state.telegram_username.as_deref() != Some(&h) {
                            state.telegram_username = Some(h.clone());
                            let _ = state.save(&state_file);
                        }
                    }
                }
            });

            let state = crate::model::state::RoutineState::load_or_default(&shared_ctx.state_path)
                .unwrap_or_default();
            let user_info = state
                .telegram_username
                .as_deref()
                .map(|u| format!("{u} [Chat {chat_id}]"))
                .unwrap_or_else(|| format!("Chat ID: {chat_id}"));
            println!("🔔 Proactive 06:00 AM morning briefings active for {user_info}");
        }
    }

    // Spawn lightweight HTTP health check server for cloud platforms (e.g. Render) unless disabled
    if !shared_ctx.no_http && std::env::var("DISABLE_HTTP_HEALTHCHECK").is_err() {
        let port = std::env::var("PORT").unwrap_or_else(|_| "8000".to_string());
        if port != "0" {
            tokio::spawn(async move {
                let addr = format!("0.0.0.0:{port}");
                if let Ok(listener) = tokio::net::TcpListener::bind(&addr).await {
                    println!("🌐 Cloud health check server listening on {addr}");
                    loop {
                        if let Ok((mut stream, _)) = listener.accept().await {
                            tokio::spawn(async move {
                                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                                let mut buf = [0u8; 512];
                                let _ = stream.read(&mut buf).await;
                                let resp = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 16\r\nConnection: close\r\n\r\nBot is healthy!\n";
                                let _ = stream.write_all(resp).await;
                            });
                        }
                    }
                }
            });
        }
    }

    // Check if initial routine sync is needed on fresh cloud deployment
    if !shared_ctx.routine_path.exists() {
        let startup_ctx = Arc::clone(&shared_ctx);
        tokio::spawn(async move {
            println!("🌐 No routine data file found on startup. Checking cloud Gist feed...");
            match startup_ctx.refresh_from_gist().await {
                Ok(url) => println!("✅ Initial routine synced from Gist: {url}"),
                Err(e) => {
                    eprintln!("⚠️ Gist initial sync note: {e}. Falling back to portal scrape...");
                    let target = startup_ctx.routine_path.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        crate::engine::scraper::run_ucam_scraper(&target)
                    })
                    .await;
                }
            }
        });
    }

    println!("🤖 Academic Routine Telegram Bot is online and listening for messages!");
    println!("   Commands supported: /today, /next, /tomorrow, /free, /teacher, /teachers, /routine, /calendar, /sync");

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(|bot: Bot, msg: Message, cmd: Command, ctx: Arc<BotContext>| async move {
                    let sender = msg
                        .from
                        .as_ref()
                        .map(|u| u.first_name.clone())
                        .unwrap_or_else(|| "Unknown".to_string());
                    println!(
                        "📩 Received command: {:?} from {} (Chat ID: {})",
                        cmd, sender, msg.chat.id
                    );
                    let res = handle_command(bot, msg, cmd, ctx).await;
                    if let Err(ref e) = res {
                        eprintln!("❌ Telegram API error: {e:?}");
                    } else {
                        println!("✅ Successfully replied to command.");
                    }
                    res
                }),
        )
        .branch(
            Update::filter_callback_query()
                .endpoint(|bot: Bot, q: CallbackQuery, ctx: Arc<BotContext>| async move {
                    let sender = &q.from.first_name;
                    let data = q.data.as_deref().unwrap_or("none");
                    println!("🔘 Received button callback '{data}' from {sender}");
                    let res = handle_callback_query(bot, q, ctx).await;
                    if let Err(ref e) = res {
                        eprintln!("❌ Callback handling error: {e:?}");
                    }
                    res
                }),
        );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![Arc::clone(&shared_ctx)])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}
