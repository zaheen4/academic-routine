//! Exit-code regression tests for the CLI's single typed error boundary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_academic-routine");

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routine_sample.json")
}

fn run_cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to execute academic-routine")
}

#[test]
fn test_missing_previous_baseline_exits_nonzero() {
    let temp_dir = tempfile::tempdir().unwrap();
    let out = run_cli(temp_dir.path(), &["diff"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn test_export_write_failure_exits_nonzero_without_success_line() {
    let temp_dir = tempfile::tempdir().unwrap();
    let blocked = temp_dir.path().join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let ics = blocked.join("routine.ics");
    let fixture = fixture();

    let out = run_cli(
        temp_dir.path(),
        &[
            "--file",
            fixture.to_str().unwrap(),
            "export",
            "--ics",
            ics.to_str().unwrap(),
        ],
    );

    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("Generated iCalendar feed"));
}

#[test]
fn test_export_success_exits_zero() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ics = temp_dir.path().join("routine.ics");
    let fixture = fixture();

    let out = run_cli(
        temp_dir.path(),
        &[
            "--file",
            fixture.to_str().unwrap(),
            "export",
            "--ics",
            ics.to_str().unwrap(),
        ],
    );

    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Generated iCalendar feed"));
    assert!(ics.exists());
}
