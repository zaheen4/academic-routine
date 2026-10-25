//! Environment-derived configuration shared by the CLI and the bot daemon.

/// Resolves the administrator's Telegram chat ID from the environment.
///
/// Tries `ADMIN_CHAT_ID` first, then `TELEGRAM_CHAT_ID`. Values that are empty,
/// whitespace-only, or not an integer are skipped, so a malformed `ADMIN_CHAT_ID`
/// does not hide a valid fallback.
#[must_use]
pub fn resolve_admin_chat_id() -> Option<i64> {
    ["ADMIN_CHAT_ID", "TELEGRAM_CHAT_ID"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find_map(|raw| raw.trim().parse::<i64>().ok())
}
