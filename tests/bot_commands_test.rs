//! Integration tests verifying all Telegram bot commands, parsing, and scheduling behavior.

use academic_routine::{generate_ics, load_routine_file, BotContext, Command, DayOfWeek};
use chrono::{NaiveTime, Timelike};
use std::path::{Path, PathBuf};
use teloxide::utils::command::BotCommands;

#[test]
fn test_all_bot_commands_syntax_parsing() {
    // 1. Core navigation commands
    assert_eq!(Command::parse("/start", "").unwrap(), Command::Start);
    assert_eq!(Command::parse("/help", "").unwrap(), Command::Help);
    assert_eq!(Command::parse("/today", "").unwrap(), Command::Today);
    assert_eq!(Command::parse("/next", "").unwrap(), Command::Next);
    assert_eq!(Command::parse("/tomorrow", "").unwrap(), Command::Tomorrow);
    assert_eq!(Command::parse("/free", "").unwrap(), Command::Free);
    assert_eq!(Command::parse("/teachers", "").unwrap(), Command::Teachers);
    assert_eq!(Command::parse("/dropped", "").unwrap(), Command::Dropped);
    assert_eq!(Command::parse("/calendar", "").unwrap(), Command::Calendar);

    // 2. Commands with arguments
    assert_eq!(
        Command::parse("/drop CSE-4125", "").unwrap(),
        Command::Drop("CSE-4125".to_string())
    );
    assert_eq!(
        Command::parse("/drop cse 4125", "").unwrap(),
        Command::Drop("cse 4125".to_string())
    );
    assert_eq!(
        Command::parse("/restore CSE-4125", "").unwrap(),
        Command::Restore("CSE-4125".to_string())
    );
    assert_eq!(
        Command::parse("/restore cse 4125", "").unwrap(),
        Command::Restore("cse 4125".to_string())
    );

    // 3. Bot handle mentions (e.g. /today@routine_bot)
    assert_eq!(
        Command::parse("/today@routine_bot", "routine_bot").unwrap(),
        Command::Today
    );
    assert_eq!(
        Command::parse("/drop@routine_bot CSE-4125", "routine_bot").unwrap(),
        Command::Drop("CSE-4125".to_string())
    );

    // 4. Invalid commands
    assert!(Command::parse("/nonexistent", "").is_err());
    assert!(Command::parse("just random text", "").is_err());
}

#[test]
fn test_free_breaks_calculation_logic() {
    let scraper_path = Path::new(
        "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
    );
    if !scraper_path.exists() {
        return;
    }

    let raw = load_routine_file(scraper_path).unwrap();
    let sched = raw.filter_section("B2");

    // Check every day for breaks
    for day in DayOfWeek::all() {
        let classes = sched.for_day(*day);
        if classes.len() >= 2 {
            for i in 0..classes.len() - 1 {
                let cur = &classes[i];
                let next = &classes[i + 1];

                if let Some(gap) = cur.time_slot.gap_between(&next.time_slot) {
                    assert!(
                        cur.time_slot.end() <= next.time_slot.start(),
                        "Break start must be <= break end"
                    );
                    assert_eq!(
                        gap,
                        (next.time_slot.start().hour() * 60 + next.time_slot.start().minute())
                            - (cur.time_slot.end().hour() * 60 + cur.time_slot.end().minute())
                    );
                }
            }
        }
    }
}

#[test]
fn test_next_class_countdown_simulation() {
    let scraper_path = Path::new(
        "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
    );
    if !scraper_path.exists() {
        return;
    }

    let sched = load_routine_file(scraper_path)
        .unwrap()
        .filter_section("B2");
    let sunday_classes = sched.for_day(DayOfWeek::Sun);
    assert!(
        !sunday_classes.is_empty(),
        "Expected classes on Sunday for section B2"
    );

    let first = &sunday_classes[0];
    let before_time = NaiveTime::from_hms_opt(7, 0, 0).unwrap();

    // 1. Simulate 07:00 AM (before first class)
    let next_class = sunday_classes
        .iter()
        .find(|c| c.time_slot.start() > before_time);
    assert!(next_class.is_some());
    let next = next_class.unwrap();
    assert_eq!(next.code, first.code);

    let mins_until = (next.time_slot.start().hour() * 60 + next.time_slot.start().minute())
        .saturating_sub(before_time.hour() * 60 + before_time.minute());
    assert!(
        mins_until > 0,
        "Countdown to morning class must be positive"
    );

    // 2. Simulate time inside class
    let mid_time = NaiveTime::from_hms_opt(
        first.time_slot.start().hour(),
        first.time_slot.start().minute() + 10,
        0,
    )
    .unwrap();
    let in_session = sunday_classes
        .iter()
        .find(|c| c.time_slot.contains_time(mid_time));
    assert!(in_session.is_some());
    assert_eq!(in_session.unwrap().code, first.code);

    // 3. Simulate night time (after all classes)
    let late_night = NaiveTime::from_hms_opt(23, 0, 0).unwrap();
    let none_left = sunday_classes
        .iter()
        .find(|c| c.time_slot.start() > late_night);
    assert!(none_left.is_none());
}

#[test]
fn test_teachers_directory_extraction() {
    let scraper_path = Path::new(
        "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
    );
    if !scraper_path.exists() {
        return;
    }

    let sched = load_routine_file(scraper_path)
        .unwrap()
        .filter_section("B2");
    let teachers = sched.teacher_directory();

    assert!(
        !teachers.is_empty(),
        "Instructor directory should contain teachers"
    );

    for (name, contact) in &teachers {
        assert!(!name.is_empty(), "Teacher name should not be blank");
        // Verify formatted output
        let summary = contact.formatted_summary();
        assert!(summary.contains(name.as_str()));
    }
}

#[test]
fn test_bot_context_lifecycle_and_calendar_sync() {
    let scraper_path = PathBuf::from(
        "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
    );
    if !scraper_path.exists() {
        return;
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let state_path = temp_dir.path().join("routine_state.json");

    let ctx = BotContext {
        routine_path: scraper_path,
        state_path: state_path.clone(),
        section: Some("B2".to_string()),
    };

    // Load active schedule
    let (sched, mut state) = ctx.load_active_schedule().unwrap();
    assert!(!sched.entries.is_empty());

    // Drop course with spaces in name
    assert!(state.drop_course("cse 4125"));
    state.save(&state_path).unwrap();

    let (sched_after_drop, state_after_drop) = ctx.load_active_schedule().unwrap();
    assert!(state_after_drop.is_dropped("CSE-4125"));
    assert!(!sched_after_drop.all_course_codes().contains("CSE-4125"));

    // Verify ICS export matches dropped status
    let ics_str = generate_ics(&sched_after_drop, None);
    assert!(
        !ics_str.contains("CSE 4125"),
        "ICS should omit dropped course"
    );
    assert!(ics_str.contains("BEGIN:VALARM"), "ICS must contain alarms");

    // Restore course with hyphen
    let mut state_to_restore = state_after_drop;
    assert!(state_to_restore.restore_course("CSE-4125"));
    state_to_restore.save(&state_path).unwrap();

    let (sched_restored, _) = ctx.load_active_schedule().unwrap();
    assert!(sched_restored.all_course_codes().contains("CSE-4125"));
}
