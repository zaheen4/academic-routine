//! Terminal table formatting and timetable visualization using comfy-table.

use crate::model::day::DayOfWeek;
use crate::model::schedule::Schedule;
use chrono::{NaiveTime, Timelike};
use comfy_table::presets::UTF8_FULL;
use comfy_table::{
    Attribute, Cell, CellAlignment, Color, ColumnConstraint, ContentArrangement, Table, Width,
};

/// Renders a beautiful ASCII/Unicode table for a single day's classes.
#[must_use]
pub fn render_day_table(schedule: &Schedule, day: DayOfWeek) -> String {
    let classes = schedule.for_day(day);
    if classes.is_empty() {
        return format!("No classes scheduled for {}.\n", day.full_name());
    }

    let mut table = Table::new();
    table
        .load_style(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);

    // If terminal width is available, set it to expand gracefully
    if let Ok((width, _)) = crossterm::terminal::size() {
        if width > 80 {
            table.set_width(width);
        }
    }

    // Table Header
    table.set_header(vec![
        Cell::new("Time Slot")
            .add_attribute(Attribute::Bold)
            .fg(Color::Cyan)
            .set_alignment(CellAlignment::Center),
        Cell::new("Course")
            .add_attribute(Attribute::Bold)
            .fg(Color::Yellow)
            .set_alignment(CellAlignment::Center),
        Cell::new("Course Title")
            .add_attribute(Attribute::Bold)
            .fg(Color::White)
            .set_alignment(CellAlignment::Left),
        Cell::new("Sec")
            .add_attribute(Attribute::Bold)
            .fg(Color::Magenta)
            .set_alignment(CellAlignment::Center),
        Cell::new("Room")
            .add_attribute(Attribute::Bold)
            .fg(Color::Green)
            .set_alignment(CellAlignment::Center),
        Cell::new("Instructor")
            .add_attribute(Attribute::Bold)
            .fg(Color::Blue)
            .set_alignment(CellAlignment::Left),
    ]);

    // Ensure Time Slot column never wraps "09:15 AM - 10:30 AM"
    if let Some(col) = table.column_mut(0) {
        col.set_constraint(ColumnConstraint::LowerBoundary(Width::Fixed(23)));
    }

    for c in classes {
        let is_lab = c.is_lab();
        let course_cell = if is_lab {
            Cell::new(format!("{} [LAB]", c.code))
                .fg(Color::Red)
                .add_attribute(Attribute::Bold)
        } else {
            Cell::new(&c.code).fg(Color::Yellow)
        };

        let teacher_info = if !c.teacher.name.is_empty() {
            if let Some(ref phone) = c.teacher.phone {
                format!("{}\n☎ {}", c.teacher.name, phone)
            } else {
                c.teacher.name.clone()
            }
        } else {
            "-".to_string()
        };

        table.add_row(vec![
            Cell::new(c.time_slot.format_12h())
                .set_alignment(CellAlignment::Center)
                .fg(Color::Cyan),
            course_cell.set_alignment(CellAlignment::Center),
            Cell::new(&c.title),
            Cell::new(&c.section)
                .set_alignment(CellAlignment::Center)
                .fg(Color::Magenta),
            Cell::new(format!("Room {}", c.room))
                .set_alignment(CellAlignment::Center)
                .fg(Color::Green),
            Cell::new(teacher_info),
        ]);
    }

    format!("── {} ──\n{table}\n", day.full_name())
}

/// Renders a terminal profile card for an individual instructor.
#[must_use]
pub fn render_teacher_card(
    name: &str,
    contact: &crate::model::course::TeacherContact,
    courses: &[&crate::model::course::CourseEntry],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "┌── Faculty Profile: {name} ───────────────────────────\n"
    ));
    if let Some(ref p) = contact.phone {
        out.push_str(&format!("│ Phone   : {p}\n"));
    }
    if let Some(ref e) = contact.email {
        out.push_str(&format!("│ Email   : {e}\n"));
    }
    if !courses.is_empty() {
        out.push_str("│\n│ Courses Taught:\n");
        for c in courses {
            let lab_badge = if c.is_lab() { " [LAB]" } else { "" };
            out.push_str(&format!(
                "│ • {} : {}{}\n│   {} {} (Room {})\n",
                c.code,
                c.title,
                lab_badge,
                c.day.full_name(),
                c.time_slot.format_12h(),
                c.room
            ));
        }
    }
    out.push_str("└──────────────────────────────────────────────────────\n");
    out
}

/// Renders the complete weekly schedule across all class days.
#[must_use]
pub fn render_weekly_overview(schedule: &Schedule) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "📚 Semester: {}\n📊 Total Classes: {}\n\n",
        schedule.semester_name,
        schedule.entries.len()
    ));

    for day in DayOfWeek::all() {
        let day_classes = schedule.for_day(*day);
        if !day_classes.is_empty() {
            out.push_str(&render_day_table(schedule, *day));
        }
    }

    out
}

/// Renders a live "Today's Schedule" card with current class and upcoming class indicators.
#[must_use]
pub fn render_today_card(schedule: &Schedule, day: DayOfWeek, now: NaiveTime) -> String {
    let classes = schedule.for_day(day);
    if classes.is_empty() {
        return format!(
            "🎉 No classes scheduled for today ({})! Enjoy your free time.\n",
            day.full_name()
        );
    }

    let mut out = format!("Today's Itinerary ({}):\n", day.full_name());
    out.push_str(&render_day_table(schedule, day));

    // Find current active class
    let active_class = classes.iter().find(|c| c.time_slot.contains_time(now));

    // Find next upcoming class
    let next_class = classes.iter().find(|c| c.time_slot.start() > now);

    out.push_str("──────────────────────────────────────────────────\n");
    if let Some(active) = active_class {
        let mins_left = (active.time_slot.end().hour() * 60 + active.time_slot.end().minute())
            .saturating_sub(now.hour() * 60 + now.minute());
        out.push_str(&format!(
            "🔴 CURRENTLY IN SESSION: [{}] {} (Room {}) — ends in {mins_left} min(s)\n",
            active.section, active.title, active.room
        ));
    } else if let Some(next) = next_class {
        let mins_until = (next.time_slot.start().hour() * 60 + next.time_slot.start().minute())
            .saturating_sub(now.hour() * 60 + now.minute());
        out.push_str(&format!(
            "🟢 NEXT UP: [{}] {} in Room {} at {} (starts in {mins_until} min(s))\n",
            next.section,
            next.title,
            next.room,
            next.time_slot.start().format("%I:%M %p")
        ));
    } else {
        out.push_str("🏁 All classes for today have concluded. Good job!\n");
    }
    out.push_str("──────────────────────────────────────────────────\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::course::{CourseEntry, TeacherContact};
    use crate::model::time_slot::TimeSlot;
    use std::str::FromStr;

    #[test]
    fn test_render_day_table() {
        let t = TeacherContact::new("Prof. X", Some("0123456789".into()), None);
        let slot = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let e = CourseEntry::new("CSE-4101", "AI", "B", DayOfWeek::Mon, slot, "120", t);

        let sched = Schedule::new("L4T1", None, vec![e]);
        let rendered = render_day_table(&sched, DayOfWeek::Mon);

        assert!(rendered.contains("Monday"));
        assert!(rendered.contains("CSE-4101"));
        assert!(rendered.contains("Room 120"));
        assert!(rendered.contains("09:15 AM - 10:30 AM"));
    }

    #[test]
    fn test_render_today_card_next_class() {
        let t = TeacherContact::new("Prof. X", None, None);
        let slot = TimeSlot::from_str("11:45 - 1:0").unwrap();
        let e = CourseEntry::new("CSE-4101", "AI", "B", DayOfWeek::Mon, slot, "120", t);

        let sched = Schedule::new("L4T1", None, vec![e]);
        let clock = NaiveTime::from_hms_opt(11, 0, 0).unwrap(); // 45 mins before
        let card = render_today_card(&sched, DayOfWeek::Mon, clock);

        assert!(card.contains("NEXT UP"));
        assert!(card.contains("starts in 45 min(s)"));
    }
}
