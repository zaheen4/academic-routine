//! `--sync-gist` must fail rather than silently skip publishing when there is no
//! Gist to publish to.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_academic-routine");

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routine_sample.json")
}

/// `GIST_ID` is stripped from the child environment on purpose: `StateStore`
/// back-fills the Gist id from it at load, so a developer with that variable set
/// would otherwise make the "no target" case pass for the wrong reason.
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .env_remove("GIST_ID")
        .output()
        .expect("failed to execute academic-routine")
}

#[test]
fn export_sync_gist_without_a_target_fails_and_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let routine = fixture();
    let out = run(
        temp.path(),
        &[
            "--file",
            routine.to_str().unwrap(),
            "export",
            "--ics",
            "output/routine.ics",
            "--sync-gist",
        ],
    );

    assert!(
        !out.status.success(),
        "export --sync-gist exited 0 with no Gist linked; stdout was:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no Gist is linked"),
        "stderr did not explain the problem:\n{stderr}"
    );
    assert!(
        stderr.contains("set-gist"),
        "stderr did not say how to fix it:\n{stderr}"
    );
    assert!(
        !temp.path().join("output/routine.ics").exists(),
        "the command wrote a calendar before failing, so it half-succeeded"
    );
}

#[test]
fn sync_gist_without_a_target_also_fails() {
    let temp = tempfile::tempdir().unwrap();
    let routine = fixture();
    let out = run(
        temp.path(),
        &["--file", routine.to_str().unwrap(), "sync-gist"],
    );
    assert!(
        !out.status.success(),
        "sync-gist exited 0 with no Gist linked; stdout was:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn export_without_sync_gist_still_writes_the_calendar_offline() {
    let temp = tempfile::tempdir().unwrap();
    let routine = fixture();
    let ics = temp.path().join("output/routine.ics");
    let out = run(
        temp.path(),
        &[
            "--file",
            routine.to_str().unwrap(),
            "export",
            "--ics",
            "output/routine.ics",
        ],
    );

    assert!(
        out.status.success(),
        "plain export failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&ics).expect("plain export wrote no calendar");
    assert!(
        text.contains("CSE-4101"),
        "the exported calendar lost its contents"
    );
}
