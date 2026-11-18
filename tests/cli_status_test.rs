//! `status` must not assert a daemon state it cannot observe.

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
fn status_never_prints_an_unprobed_daemon_claim() {
    let temp = tempfile::tempdir().unwrap();
    let routine = fixture();
    let out = run(
        temp.path(),
        &["--file", routine.to_str().unwrap(), "status"],
    );
    assert!(
        out.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);

    // The old line was a constant, printed whether or not anything was running.
    assert!(
        !text.contains("ONLINE (Local daemon)"),
        "status still prints the unconditional daemon claim:\n{text}"
    );
    assert!(
        text.contains("Telegram Bot"),
        "status dropped the daemon line instead of qualifying it:\n{text}"
    );
    // Whatever it says must come from the three defined outcomes.
    assert!(
        text.contains("ONLINE (health check")
            || text.contains("NOT DETECTED (no health endpoint")
            || text.contains("UNKNOWN (health check"),
        "unexpected daemon wording:\n{text}"
    );
}
