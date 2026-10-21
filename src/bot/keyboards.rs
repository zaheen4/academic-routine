//! Interactive Telegram inline keyboards for quick 1-tap navigation and faculty actions.

use crate::model::day::DayOfWeek;
use reqwest::Url;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

/// Builds the home navigation grid for /start and /help cards.
#[must_use]
pub fn main_menu_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("⏱️ Next Class", "nav_next"),
            InlineKeyboardButton::callback("📅 Today", "nav_today"),
        ],
        vec![
            InlineKeyboardButton::callback("🗓️ Tomorrow", "nav_tomorrow"),
            InlineKeyboardButton::callback("📋 Routine", "nav_routine"),
        ],
        vec![
            InlineKeyboardButton::callback("☕ Free Breaks", "nav_free"),
            InlineKeyboardButton::callback("📚 Faculty", "nav_teachers"),
        ],
        vec![InlineKeyboardButton::callback(
            "📥 Download .ics Calendar",
            "nav_calendar",
        )],
    ])
}

/// Builds quick action buttons for /today schedule cards.
#[must_use]
pub fn today_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("⏱️ Next Class", "nav_next"),
            InlineKeyboardButton::callback("🗓️ Tomorrow", "nav_tomorrow"),
        ],
        vec![
            InlineKeyboardButton::callback("📋 Full Routine", "nav_routine"),
            InlineKeyboardButton::callback("📥 .ics File", "nav_calendar"),
        ],
    ])
}

/// Builds countdown action buttons for /next class cards.
#[must_use]
pub fn next_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("🔄 Refresh Countdown", "nav_next"),
        InlineKeyboardButton::callback("📅 Today's Schedule", "nav_today"),
    ]])
}

/// Builds day-switcher buttons for the weekly routine view.
/// Weekdays are strictly emojiless.
#[must_use]
pub fn routine_days_keyboard(active_day: Option<DayOfWeek>) -> InlineKeyboardMarkup {
    let days = [
        (DayOfWeek::Sun, "Sun", "day_sun"),
        (DayOfWeek::Mon, "Mon", "day_mon"),
        (DayOfWeek::Tue, "Tue", "day_tue"),
        (DayOfWeek::Wed, "Wed", "day_wed"),
        (DayOfWeek::Thu, "Thu", "day_thu"),
    ];

    let row1 = days
        .into_iter()
        .map(|(day, label, cb)| {
            let text = if Some(day) == active_day {
                format!("❯ {label} ❮")
            } else {
                label.to_string()
            };
            InlineKeyboardButton::callback(text, cb)
        })
        .collect();

    let overview_label = if active_day.is_none() {
        "❯ 📋 Overview ❮"
    } else {
        "📋 Overview"
    };

    let row2 = vec![
        InlineKeyboardButton::callback(overview_label, "day_all"),
        InlineKeyboardButton::callback("📅 Today", "nav_today"),
    ];

    InlineKeyboardMarkup::new(vec![row1, row2])
}

/// Builds 1-tap contact buttons for a faculty profile card.
#[must_use]
pub fn teacher_contact_keyboard(
    phone: Option<&str>,
    email: Option<&str>,
) -> InlineKeyboardMarkup {
    let mut row1 = Vec::new();

    if let Some(p) = phone {
        let clean_phone: String = p.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
        if !clean_phone.is_empty() {
            if let Ok(url) = Url::parse(&format!("tel:{clean_phone}")) {
                row1.push(InlineKeyboardButton::url("📞 Call", url));
            }
        }
    }

    if let Some(e) = email {
        let clean_email = e.trim();
        if !clean_email.is_empty() {
            if let Ok(url) = Url::parse(&format!("mailto:{clean_email}")) {
                row1.push(InlineKeyboardButton::url("✉️ Email", url));
            }
        }
    }

    let row2 = vec![
        InlineKeyboardButton::callback("📚 All Faculty", "nav_teachers"),
        InlineKeyboardButton::callback("📅 Today", "nav_today"),
    ];

    if row1.is_empty() {
        InlineKeyboardMarkup::new(vec![row2])
    } else {
        InlineKeyboardMarkup::new(vec![row1, row2])
    }
}
