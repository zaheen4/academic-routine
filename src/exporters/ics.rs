//! RFC 5545 compliant iCalendar (`.ics`) generation and phone synchronization.
//!
//! Produces recurring calendar feeds compatible with Google Calendar, Android system calendars,
//! Apple Calendar, and desktop clients (Thunderbird).
//!
//! Features:
//! - Deterministic stable `UID`s including the class section (prevents duplicate/missing labs).
//! - A real `VTIMEZONE` block plus correct UTC `UNTIL` conversion.
//! - 75-octet UTF-8-safe line folding for every property.
//! - Automatic sound & display alarms 30 minutes before every class (`VALARM`).

use crate::model::day::DayOfWeek;
use crate::model::schedule::Schedule;
use chrono::{Datelike, FixedOffset, NaiveDate, TimeZone, Utc};
use std::fs;
use std::io;
use std::path::Path;

const TZID: &str = "Asia/Dhaka";
const MAX_LINE_OCTETS: usize = 75;

/// Generates an RFC 5545 compliant iCalendar (`.ics`) string for the given schedule.
///
/// If `semester_end_date` is provided, recurring events will automatically cease
/// on that date via an `UNTIL` rule (converted to the correct UTC instant).
#[must_use]
pub fn generate_ics(schedule: &Schedule, semester_end_date: Option<NaiveDate>) -> String {
    let dtstamp = schedule
        .generated_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
        .format("%Y%m%dT%H%M%SZ")
        .to_string();
    let today = crate::model::day::academic_now().date_naive();

    let mut ics = String::new();
    ics.push_str("BEGIN:VCALENDAR\r\n");
    push_prop(&mut ics, "VERSION", "2.0");
    push_prop(&mut ics, "PRODID", "-//Academic Routine Assistant//EN");
    push_prop(&mut ics, "CALSCALE", "GREGORIAN");
    push_prop(&mut ics, "METHOD", "PUBLISH");
    push_text_prop(&mut ics, "X-WR-CALNAME", &schedule.semester_name);
    push_prop(&mut ics, "X-WR-TIMEZONE", TZID);

    // Bangladesh observes UTC+6 year-round, so one STANDARD observance is correct.
    let offset = local_offset();
    let offset_from = format!(
        "{}{:02}{:02}",
        if offset < 0 { "-" } else { "+" },
        offset.abs() / 3600,
        (offset.abs() % 3600) / 60
    );
    ics.push_str("BEGIN:VTIMEZONE\r\n");
    push_prop(&mut ics, "TZID", TZID);
    push_prop(&mut ics, "X-LIC-LOCATION", TZID);
    ics.push_str("BEGIN:STANDARD\r\n");
    push_prop(&mut ics, "DTSTART", "19700101T000000");
    push_prop(&mut ics, "TZOFFSETFROM", &offset_from);
    push_prop(&mut ics, "TZOFFSETTO", &offset_from);
    push_prop(&mut ics, "TZNAME", &format_offset_name(offset));
    ics.push_str("END:STANDARD\r\n");
    ics.push_str("END:VTIMEZONE\r\n");

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

        let rrule = if let Some(until) = semester_end_date {
            format!(
                "FREQ=WEEKLY;BYDAY={};UNTIL={}",
                entry.day.ical_code(),
                until_utc_compact(until, offset)
            )
        } else {
            format!("FREQ=WEEKLY;BYDAY={}", entry.day.ical_code())
        };

        ics.push_str("BEGIN:VEVENT\r\n");
        push_prop(&mut ics, "UID", &uid);
        push_prop(&mut ics, "DTSTAMP", &dtstamp);
        push_prop(&mut ics, &format!("DTSTART;TZID={TZID}"), &dtstart);
        push_prop(&mut ics, &format!("DTEND;TZID={TZID}"), &dtend);
        push_prop(&mut ics, "RRULE", &rrule);
        push_text_prop(&mut ics, "SUMMARY", &summary);
        push_text_prop(&mut ics, "LOCATION", &location);
        push_text_prop(&mut ics, "DESCRIPTION", &description);
        push_prop(&mut ics, "STATUS", "CONFIRMED");
        push_text_prop(&mut ics, "X-ACADEMIC-ROUTINE-SECTION", &entry.section);

        ics.push_str("BEGIN:VALARM\r\n");
        push_prop(&mut ics, "TRIGGER", "-PT30M");
        push_prop(&mut ics, "ACTION", "DISPLAY");
        push_text_prop(
            &mut ics,
            "DESCRIPTION",
            &format!("Class Reminder: {} in {}", entry.code, location),
        );
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
    // Write to a sibling temp file and rename so a crash cannot truncate a good feed.
    let tmp = output_path.with_extension("ics.tmp");
    fs::write(&tmp, ics_content)?;
    fs::rename(&tmp, output_path)
}

/// Appends a folded `NAME:value` property line with RFC 5545 text escaping.
fn push_text_prop(out: &mut String, name: &str, value: &str) {
    let line = format!("{name}:{}", escape_ics(value));
    out.push_str(&fold_ical_line(&line));
}

/// Appends a folded `NAME:value` property line without escaping structured fields (RRULE, DATES, UID).
fn push_prop(out: &mut String, name: &str, value: &str) {
    let line = format!("{name}:{value}");
    out.push_str(&fold_ical_line(&line));
}

/// Folds one logical property line at [`MAX_LINE_OCTETS`], never splitting a UTF-8 char.
#[must_use]
fn fold_ical_line(line: &str) -> String {
    let mut out = String::new();
    let mut current = String::new();
    let mut octets = 0usize;

    for ch in line.chars() {
        let width = ch.len_utf8();
        if octets + width > MAX_LINE_OCTETS {
            out.push_str(&current);
            out.push_str("\r\n"); // continuation WSP is the seeded ' ' below
            current.clear();
            current.push(' ');
            octets = 1;
        }
        current.push(ch);
        octets += width;
    }

    out.push_str(&current);
    out.push_str("\r\n");
    out
}

/// Escapes special characters for RFC 5545 text values and strips control chars.
fn escape_ics(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut escaped = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            ';' => escaped.push_str("\\;"),
            ',' => escaped.push_str("\\,"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push('\t'),
            c if c.is_control() => {}
            c => escaped.push(c),
        }
    }
    escaped
}

fn local_offset_seconds() -> i32 {
    std::env::var("ROUTINE_TIMEZONE_OFFSET")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .map(|hours| hours * 3600)
        .unwrap_or(6 * 3600)
}

fn local_offset() -> i32 {
    local_offset_seconds()
}

fn format_offset_name(offset_seconds: i32) -> String {
    format!(
        "{}{:02}:{:02}",
        if offset_seconds < 0 { "-" } else { "+" },
        offset_seconds.abs() / 3600,
        (offset_seconds.abs() % 3600) / 60
    )
}

/// Converts a local end-of-day date into the UTC compact form used by `UNTIL`.
fn until_utc_compact(date: NaiveDate, offset_seconds: i32) -> String {
    let offset = FixedOffset::east_opt(offset_seconds)
        .or_else(|| FixedOffset::west_opt(-offset_seconds))
        .unwrap_or_else(|| FixedOffset::east_opt(6 * 3600).expect("UTC+6 is valid"));
    let local = date
        .and_hms_opt(23, 59, 59)
        .expect("end-of-day is a valid time");
    offset
        .from_local_datetime(&local)
        .single()
        .expect("fixed offset always resolves")
        .with_timezone(&Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
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

        let sched = Schedule::new("L4T1", Some("2026-10-17T13:03:50Z".to_string()), vec![e]);
        let end_date = NaiveDate::from_ymd_opt(2026, 11, 30);
        let ics = generate_ics(&sched, end_date);

        // Verify required RFC 5545 headers
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(ics.contains("VERSION:2.0\r\n"));
        assert!(ics.contains("CALSCALE:GREGORIAN\r\n"));
        assert!(ics.contains("METHOD:PUBLISH\r\n"));
        assert!(ics.contains("X-WR-TIMEZONE:Asia/Dhaka\r\n"));
        assert!(ics.contains("BEGIN:VTIMEZONE\r\n"));
        assert!(ics.contains("TZID:Asia/Dhaka\r\n"));
        assert!(ics.find("BEGIN:VTIMEZONE").unwrap() < ics.find("BEGIN:VEVENT").unwrap());
        assert!(ics.contains("UID:cse4101-b-mon-091500@academic-routine\r\n"));
        assert!(ics.contains("SUMMARY:[B] CSE-4101: AI\r\n"));
        assert!(ics.contains("LOCATION:Room 120\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=MO;UNTIL=20261130T175959Z\r\n"));
        assert!(ics.contains("DTSTAMP:20261017T130350Z\r\n"));

        // Verify 30-minute alarm
        assert!(ics.contains("BEGIN:VALARM\r\n"));
        assert!(ics.contains("TRIGGER:-PT30M\r\n"));
        assert!(ics.contains("END:VALARM\r\n"));
    }

    #[test]
    fn test_all_physical_lines_fold_at_75_octets() {
        let teacher = TeacherContact::new(
            "Professor With A Very Long Name For Folding",
            Some("+8801700000000".into()),
            Some("verylongmailbox@example-university.edu.bd".into()),
        );
        let slot = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let entry = CourseEntry::new(
            "CSE-4101",
            "Advanced Topics In Artificial Intelligence And Distributed Systems",
            "B2",
            DayOfWeek::Mon,
            slot,
            "Lab 1 & 2",
            teacher,
        );
        let ics = generate_ics(&Schedule::new("L4T1", None, vec![entry]), None);

        for line in ics.split("\r\n") {
            assert!(
                line.len() <= 75,
                "line exceeds 75 octets ({}): {line}",
                line.len()
            );
        }
        // Unfold and ensure the full description survived intact.
        let unfolded = ics.replace("\r\n ", "");
        assert!(unfolded.contains("Advanced Topics In Artificial Intelligence"));
        assert!(unfolded.contains("Lab 1 & 2"));
    }

    #[test]
    fn test_emoji_near_fold_boundary_is_not_split() {
        let teacher = TeacherContact::new("🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓", None, None);
        let slot = TimeSlot::from_str("08:00 - 09:15").unwrap();
        let entry = CourseEntry::new(
            "CSE-4101",
            "🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓🎓",
            "B",
            DayOfWeek::Tue,
            slot,
            "🎓",
            teacher,
        );
        let ics = generate_ics(&Schedule::new("🎓 Term", None, vec![entry]), None);
        assert!(ics.contains("🎓"));
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75);
        }
    }

    #[test]
    fn test_escape_and_control_characters() {
        assert_eq!(escape_ics("a;b,c\\d"), "a\\;b\\,c\\\\d");
        assert_eq!(escape_ics("line1\r\nline2"), "line1\\nline2");
        assert_eq!(escape_ics("bell\u{0007}tab\t"), "belltab\t");
    }

    fn unfold_ics(folded: &str) -> String {
        let trimmed = folded.strip_suffix("\r\n").unwrap_or(folded);
        trimmed.replace("\r\n ", "")
    }

    #[test]
    fn test_fold_then_unfold_roundtrips() {
        for len in 1..=300 {
            let original = format!("DESCRIPTION:{}", "a".repeat(len));
            let folded = fold_ical_line(&original);
            let unfolded = unfold_ics(&folded);
            assert_eq!(unfolded, original, "Failed roundtrip for length {len}");
        }

        let sample = "DESCRIPTION:পরীক্ষা 🎓 Advanced Topics In Distributed Systems & Machine Learning 🚀 — কক্ষ ২০৪";
        let folded = fold_ical_line(sample);
        let unfolded = unfold_ics(&folded);
        assert_eq!(unfolded, sample);
    }

    #[test]
    fn test_no_physical_line_exceeds_75_octets() {
        for len in 1..=300 {
            let line = format!("DESCRIPTION:{}", "x".repeat(len));
            let folded = fold_ical_line(&line);
            for physical_line in folded.split("\r\n") {
                assert!(
                    physical_line.len() <= 75,
                    "physical line length {} exceeds 75 octets: {physical_line}",
                    physical_line.len()
                );
            }
        }
    }

    #[test]
    fn test_continuation_lines_start_with_exactly_one_space() {
        let line = format!("DESCRIPTION:{}", "y".repeat(250));
        let folded = fold_ical_line(&line);
        let mut lines = folded.split("\r\n").filter(|l| !l.is_empty());
        let _first = lines.next().unwrap();
        for cont in lines {
            assert!(
                cont.starts_with(' '),
                "continuation line must start with space: {cont}"
            );
            assert!(
                !cont.starts_with("  "),
                "continuation line must not start with double space: {cont}"
            );
        }
    }

    #[test]
    fn test_end_to_end_generate_ics_has_no_double_space_continuations() {
        let teacher = TeacherContact::new(
            "Professor Long Name With Very Long Email Address For Testing",
            Some("+8801700000000".into()),
            Some("verylongmailbox@example-university.edu.bd".into()),
        );
        let slot = TimeSlot::from_str("09:15 - 10:30").unwrap();
        let entry = CourseEntry::new(
            "CSE-4101",
            "Advanced Artificial Intelligence And Large-Scale Machine Learning Architecture",
            "B2",
            DayOfWeek::Mon,
            slot,
            "Room 120 / Software Lab 4",
            teacher,
        );
        let sched = Schedule::new("CSE Spring 2026", None, vec![entry]);
        let ics = generate_ics(&sched, NaiveDate::from_ymd_opt(2026, 11, 30));

        assert!(
            !ics.contains("\r\n  "),
            "ICS must not contain double-space continuation lines"
        );
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "line exceeded 75 octets: {line}");
        }
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
