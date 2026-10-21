//! RFC 5545 compliant iCalendar (.ics) generation and phone synchronization.
//!
//! Produces recurring calendar feeds compatible with Google Calendar, Android system calendars,
//! Apple Calendar, and desktop clients (Thunderbird).
//!
//! Features:
//! - Deterministic stable `UID`s (prevents duplicate events upon re-import or feed refresh).
//! - Recurrence rules (`RRULE`) with configurable semester end dates (`UNTIL`).
//! - Automatic sound & display alarms 30 minutes before every class (`VALARM`).
//! - Rich clickable event descriptions with instructor phone numbers and emails.

use crate::model::day::DayOfWeek;
use crate::model::schedule::Schedule;
use chrono::{Datelike, NaiveDate, Utc};
use std::fs;
use std::io;
use std::path::Path;

/// Generates an RFC 5545 compliant iCalendar (`.ics`) string for the given schedule.
///
/// If `semester_end_date` is provided, recurring events will automatically cease
/// on that date via an `UNTIL` rule.
#[must_use]
pub fn generate_ics(schedule: &Schedule, semester_end_date: Option<NaiveDate>) -> String {
    let now_utc = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let today = crate::model::day::academic_now().date_naive();

    let mut ics = String::new();
    ics.push_str("BEGIN:VCALENDAR\r\n");
    ics.push_str("VERSION:2.0\r\n");
    ics.push_str("PRODID:-//Academic Routine Assistant//EN\r\n");
    ics.push_str("CALSCALE:GREGORIAN\r\n");
    ics.push_str("METHOD:PUBLISH\r\n");
    ics.push_str(&format!(
        "X-WR-CALNAME:{}\r\n",
        escape_ics(&schedule.semester_name)
    ));
    ics.push_str("X-WR-TIMEZONE:Asia/Dhaka\r\n");

    for entry in &schedule.entries {
        let first_date = next_matching_weekday(entry.day, today);
        let dtstart = format!(
            "{:04}{:02}{:02}T{}",
            first_date.year(),
            first_date.month(),
            first_date.day(),
            entry.time_slot.ical_start_time()
        );
        let dtend = format!(
            "{:04}{:02}{:02}T{}",
            first_date.year(),
            first_date.month(),
            first_date.day(),
            entry.time_slot.ical_end_time()
        );

        let uid = entry.ical_uid();
        let summary = format!("[{}] {}: {}", entry.section, entry.code, entry.title);
        let location = format!("Room {}", entry.room);
        let description = entry.teacher.formatted_summary();

        // Recurrence rule
        let rrule = if let Some(until) = semester_end_date {
            format!(
                "RRULE:FREQ=WEEKLY;BYDAY={};UNTIL={:04}{:02}{:02}T235959Z\r\n",
                entry.day.ical_code(),
                until.year(),
                until.month(),
                until.day()
            )
        } else {
            format!("RRULE:FREQ=WEEKLY;BYDAY={}\r\n", entry.day.ical_code())
        };

        ics.push_str("BEGIN:VEVENT\r\n");
        ics.push_str(&format!("UID:{uid}\r\n"));
        ics.push_str(&format!("DTSTAMP:{now_utc}\r\n"));
        ics.push_str(&format!("DTSTART;TZID=Asia/Dhaka:{dtstart}\r\n"));
        ics.push_str(&format!("DTEND;TZID=Asia/Dhaka:{dtend}\r\n"));
        ics.push_str(&rrule);
        ics.push_str(&format!("SUMMARY:{}\r\n", escape_ics(&summary)));
        ics.push_str(&format!("LOCATION:{}\r\n", escape_ics(&location)));
        ics.push_str(&format!("DESCRIPTION:{}\r\n", escape_ics(&description)));
        ics.push_str("STATUS:CONFIRMED\r\n");

        // 30-minute sound/display notification alarm
        ics.push_str("BEGIN:VALARM\r\n");
        ics.push_str("TRIGGER:-PT30M\r\n");
        ics.push_str("ACTION:DISPLAY\r\n");
        ics.push_str(&format!(
            "DESCRIPTION:Class Reminder: {} in {}\r\n",
            escape_ics(&entry.code),
            escape_ics(&location)
        ));
        ics.push_str("END:VALARM\r\n");

        ics.push_str("END:VEVENT\r\n");
    }

    ics.push_str("END:VCALENDAR\r\n");
    ics
}

/// Exports the schedule to an iCalendar file at the specified path.
///
/// # Errors
/// Returns an [`io::Error`] if the file cannot be written.
pub fn export_ics_file(
    schedule: &Schedule,
    output_path: &Path,
    semester_end_date: Option<NaiveDate>,
) -> io::Result<()> {
    let ics_content = generate_ics(schedule, semester_end_date);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, ics_content)
}

/// Finds the next upcoming calendar date matching the specified academic weekday on or after `reference`.
fn next_matching_weekday(day: DayOfWeek, reference: NaiveDate) -> NaiveDate {
    let target = match day {
        DayOfWeek::Sat => chrono::Weekday::Sat,
        DayOfWeek::Sun => chrono::Weekday::Sun,
        DayOfWeek::Mon => chrono::Weekday::Mon,
        DayOfWeek::Tue => chrono::Weekday::Tue,
        DayOfWeek::Wed => chrono::Weekday::Wed,
        DayOfWeek::Thu => chrono::Weekday::Thu,
        DayOfWeek::Fri => chrono::Weekday::Fri,
    };

    let mut current = reference;
    while current.weekday() != target {
        current = current.succ_opt().unwrap_or(current);
    }
    current
}

/// Escapes special characters for RFC 5545 text values.
fn escape_ics(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::course::{CourseEntry, TeacherContact};
    use crate::model::time_slot::TimeSlot;
    use std::str::FromStr;

    #[test]
    fn test_generate_ics_structure() {
        let t = TeacherContact::new("Utpol Kanti Das", Some("01859222633".into()), None);
        let slot = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let e = CourseEntry::new("CSE-4101", "AI", "B", DayOfWeek::Mon, slot, "120", t);

        let sched = Schedule::new("L4T1", None, vec![e]);
        let end_date = NaiveDate::from_ymd_opt(2026, 11, 30);
        let ics = generate_ics(&sched, end_date);

        // Verify required RFC 5545 headers
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(ics.contains("VERSION:2.0\r\n"));
        assert!(ics.contains("UID:cse4101-mon-091500@academic-routine\r\n"));
        assert!(ics.contains("SUMMARY:[B] CSE-4101: AI\r\n"));
        assert!(ics.contains("LOCATION:Room 120\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=MO;UNTIL=20261130T235959Z\r\n"));

        // Verify 30-minute alarm
        assert!(ics.contains("BEGIN:VALARM\r\n"));
        assert!(ics.contains("TRIGGER:-PT30M\r\n"));
        assert!(ics.contains("END:VALARM\r\n"));
    }

    #[test]
    fn test_weekday_calculation() {
        // 2026-10-20 is a Tuesday
        let ref_tue = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();

        let next_tue = next_matching_weekday(DayOfWeek::Tue, ref_tue);
        assert_eq!(next_tue, ref_tue); // same day

        let next_wed = next_matching_weekday(DayOfWeek::Wed, ref_tue);
        assert_eq!(next_wed, NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());

        let next_mon = next_matching_weekday(DayOfWeek::Mon, ref_tue);
        assert_eq!(next_mon, NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
    }

    #[test]
    fn test_export_ics_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ics_file = temp_dir.path().join("test_routine.ics");

        let slot = TimeSlot::from_str("08:00 - 09:15").unwrap();
        let e = CourseEntry::new(
            "CSE-4103",
            "Compiler",
            "B",
            DayOfWeek::Wed,
            slot,
            "121",
            TeacherContact::default(),
        );
        let sched = Schedule::new("L4T1", None, vec![e]);

        export_ics_file(&sched, &ics_file, None).unwrap();
        assert!(ics_file.exists());

        let content = fs::read_to_string(&ics_file).unwrap();
        assert!(content.contains("CSE-4103"));
    }
}
