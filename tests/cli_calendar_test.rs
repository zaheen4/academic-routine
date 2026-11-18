//! `complete`/`resume` must leave the local calendar in step with the state file.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_academic-routine");

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routine_sample.json")
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to execute academic-routine")
}

#[test]
fn complete_and_resume_regenerate_the_calendar() {
    let temp = tempfile::tempdir().unwrap();
    let routine = fixture();
    let routine = routine.to_str().unwrap();
    let ics = temp.path().join("output/routine.ics");

    let out = run(temp.path(), &["--file", routine, "complete", "CSE-4101"]);
    assert!(
        out.status.success(),
        "complete failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&ics).expect("complete did not write output/routine.ics");
    assert!(
        !text.contains("CSE-4101"),
        "the completed course is still in the calendar"
    );
    assert!(
        text.contains("CSE-4102"),
        "an unrelated course vanished from the calendar"
    );

    let out = run(temp.path(), &["--file", routine, "resume", "CSE-4101"]);
    assert!(
        out.status.success(),
        "resume failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&ics).unwrap();
    assert!(
        text.contains("CSE-4101"),
        "the resumed course is missing from the calendar"
    );
}

#[test]
fn complete_on_an_already_completed_course_still_repairs_the_calendar() {
    let temp = tempfile::tempdir().unwrap();
    let routine = fixture();
    let routine = routine.to_str().unwrap();
    let ics = temp.path().join("output/routine.ics");

    let out = run(temp.path(), &["--file", routine, "complete", "CSE-4101"]);
    assert!(out.status.success());
    assert!(ics.exists());

    // Simulate a calendar that was deleted or never written after the state change.
    std::fs::remove_file(&ics).unwrap();

    let out = run(temp.path(), &["--file", routine, "complete", "CSE-4101"]);
    assert!(
        out.status.success(),
        "re-running complete failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        ics.exists(),
        "re-running complete on an already-completed course must still regenerate"
    );
}
