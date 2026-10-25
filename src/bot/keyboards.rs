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
            InlineKeyboardButton::callback("📊 Attendance", "nav_attendance"),
            InlineKeyboardButton::callback("🏷️ Section", "nav_section"),
        ],
        vec![
            InlineKeyboardButton::callback("☕ Free Breaks", "nav_free"),
            InlineKeyboardButton::callback("📚 Faculty", "nav_teachers"),
        ],
        vec![InlineKeyboardButton::callback(
            "📅 Live Calendar Feed",
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
            InlineKeyboardButton::callback("📅 Live Calendar", "nav_calendar"),
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
/// Weekdays are strictly emojiless (Sun–Thu academic week; Friday & Saturday are weekend holidays).
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
pub fn teacher_contact_keyboard(phone: Option<&str>, email: Option<&str>) -> InlineKeyboardMarkup {
    let mut row1 = Vec::new();

    if let Some(p) = phone {
        let clean_phone: String = p
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect();
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

/// Builds section-switcher buttons for /section.
#[must_use]
pub fn section_selector_keyboard(active: Option<&str>) -> InlineKeyboardMarkup {
    let act = active.unwrap_or("B2").to_uppercase();

    let b1_label = if act == "B1" {
        "❯ Section B1 ❮"
    } else {
        "Section B1"
    };
    let b2_label = if act == "B2" {
        "❯ Section B2 ❮"
    } else {
        "Section B2"
    };
    let all_label = if act == "ALL" {
        "❯ All Sections ❮"
    } else {
        "All Sections"
    };

    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(b1_label, "sec_b1"),
            InlineKeyboardButton::callback(b2_label, "sec_b2"),
        ],
        vec![InlineKeyboardButton::callback(all_label, "sec_all")],
        vec![
            InlineKeyboardButton::callback("📅 Today", "nav_today"),
            InlineKeyboardButton::callback("📋 Routine", "nav_routine"),
        ],
    ])
}

/// Builds action buttons for /attendance dashboard.
#[must_use]
pub fn attendance_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("📅 Today's Schedule", "nav_today"),
        InlineKeyboardButton::callback("📋 Routine", "nav_routine"),
    ]])
}

/// Builds 1-tap live calendar subscription buttons (Apple Calendar webcal, Google Calendar web add).
#[must_use]
pub fn calendar_subscription_keyboard(gist_id: Option<&str>) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();

    if let Some(gid) = gist_id {
        let clean_id = gid.trim();
        if !clean_id.is_empty() {
            // Apple Calendar direct HTTPS subscription link (RFC 5545)
            let apple_url_str =
                format!("https://gist.githubusercontent.com/raw/{clean_id}/routine.ics");
            // Google Calendar 1-tap web add URL
            let gcal_url_str = format!(
                "https://calendar.google.com/calendar/r?cid=webcal%3A%2F%2Fgist.githubusercontent.com%2Fraw%2F{clean_id}%2Froutine.ics"
            );

            if let (Ok(apple_url), Ok(gcal_url)) =
                (Url::parse(&apple_url_str), Url::parse(&gcal_url_str))
            {
                rows.push(vec![
                    InlineKeyboardButton::url("🍏 Apple Calendar", apple_url),
                    InlineKeyboardButton::url("📅 Google Calendar", gcal_url),
                ]);
            }
        }
    }

    rows.push(vec![
        InlineKeyboardButton::callback("📅 Today", "nav_today"),
        InlineKeyboardButton::callback("📋 Full Routine", "nav_routine"),
    ]);

    InlineKeyboardMarkup::new(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routine_days_keyboard_academic_days_only() {
        let kb = routine_days_keyboard(Some(DayOfWeek::Mon));
        assert_eq!(kb.inline_keyboard.len(), 2);

        // Row 1: strictly 5 academic teaching days (Sun..=Thu)
        let row1 = &kb.inline_keyboard[0];
        assert_eq!(row1.len(), 5);

        let labels: Vec<&str> = row1.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(labels, vec!["Sun", "❯ Mon ❮", "Tue", "Wed", "Thu"]);

        // Verify Friday and Saturday are excluded
        for btn in row1 {
            assert!(!btn.text.contains("Fri"));
            assert!(!btn.text.contains("Sat"));
        }
    }
}
