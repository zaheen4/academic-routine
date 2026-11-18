use academic_routine::{load_routine_file, DayOfWeek};
use std::path::Path;

#[test]
fn test_sample_ucam_file_ingestion() {
    let real_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routine_sample.json");

    let schedule = load_routine_file(&real_path).expect("Failed to ingest real UCAM routine file");

    println!("Successfully loaded semester: {}", schedule.semester_name);
    println!("Total classes ingested: {}", schedule.entries.len());

    assert!(
        !schedule.entries.is_empty(),
        "Expected at least one class entry in real routine"
    );

    // Verify course codes are extracted
    let codes = schedule.all_course_codes();
    assert!(codes.contains("CSE-4101"));

    // Verify day ordering
    for day in DayOfWeek::all() {
        let classes = schedule.for_day(*day);
        for window in classes.windows(2) {
            assert!(
                window[0].time_slot.start() <= window[1].time_slot.start(),
                "Classes on {} should be sorted chronologically",
                day
            );
        }
    }
}
