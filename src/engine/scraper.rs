//! Subprocess orchestrator for the standalone UCAM extraction engine.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tokio::process::Command as TokioCommand;

const DEFAULT_SCRAPER_TIMEOUT_SECS: u64 = 900; // 15 minutes

/// Runtime limit for a scraper subprocess, from `SCRAPER_TIMEOUT_SECS`.
fn scraper_timeout() -> std::time::Duration {
    let secs = std::env::var("SCRAPER_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_SCRAPER_TIMEOUT_SECS);
    std::time::Duration::from_secs(secs)
}

/// Error executing the standalone UCAM scraper.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScraperError {
    /// Python interpreter not found on system.
    #[error("Python interpreter not found on system. Please ensure python3 is installed.")]
    PythonNotFound,

    /// Scraper script failed with non-zero exit status.
    #[error("UCAM scraper failed: {0}")]
    ExecutionFailed(String),

    /// Generated routine JSON file was not found after execution.
    #[error("Routine file was not created at expected path: {0}")]
    OutputFileNotFound(String),
}

fn is_executable_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn run_child_with_timeout(
    mut cmd: TokioCommand,
    limit: std::time::Duration,
) -> Result<std::process::Output, ScraperError> {
    // MANDATORY: `wait_with_output` consumes the `Child`. On timeout the future is dropped,
    // dropping the `Child` with it. Without `kill_on_drop`, tokio's default of `false` leaves
    // the scraper alive as an orphan that keeps running after we have given up on it.
    cmd.kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| ScraperError::ExecutionFailed(e.to_string()))?;
    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(ScraperError::ExecutionFailed(e.to_string())),
        Err(_elapsed) => Err(ScraperError::ExecutionFailed(format!(
            "Scraper command timed out after {}s (set SCRAPER_TIMEOUT_SECS to change)",
            limit.as_secs()
        ))),
    }
}

/// Runs the standalone UCAM multi-account scraper and returns the generated routine path.
///
/// This is an async function; the subprocess is bounded by `SCRAPER_TIMEOUT_SECS`.
///
/// Runs in offscreen virtual display mode (`xvfb-run` on Linux with `WAYLAND_DISPLAY` stripped)
/// to ensure the browser is completely invisible and no windows pop up on the desktop.
///
/// # Errors
/// Returns [`ScraperError`] if Python is missing, extraction fails, or output is absent.
pub async fn run_ucam_scraper(output_path: &Path) -> Result<PathBuf, ScraperError> {
    run_ucam_scraper_opts(output_path, false).await
}

/// Runs the standalone UCAM multi-account scraper with explicit visibility options.
///
/// This is an async function; the subprocess is bounded by `SCRAPER_TIMEOUT_SECS`.
///
/// When `headed` is false, runs strictly offscreen via an isolated Xvfb virtual framebuffer.
///
/// # Errors
/// Returns [`ScraperError`] if Python is missing, extraction fails, or output is absent.
pub async fn run_ucam_scraper_opts(
    output_path: &Path,
    headed: bool,
) -> Result<PathBuf, ScraperError> {
    run_ucam_scraper_opts_force(output_path, headed, false).await
}

/// Runs the standalone UCAM multi-account scraper with explicit visibility and force overwrite options.
///
/// This is an async function; the subprocess is bounded by `SCRAPER_TIMEOUT_SECS`.
///
/// # Errors
/// Returns [`ScraperError`] if Python is missing, extraction fails, or output is absent.
pub async fn run_ucam_scraper_opts_force(
    output_path: &Path,
    headed: bool,
    force: bool,
) -> Result<PathBuf, ScraperError> {
    // 1. Resolve Python executable
    let python_candidates = [
        PathBuf::from(".venv/bin/python"),
        PathBuf::from("python3"),
        PathBuf::from("python"),
    ];

    let python_bin = python_candidates
        .into_iter()
        .find(|p| p.exists() || is_executable_available(p.to_str().unwrap_or("python3")))
        .ok_or(ScraperError::PythonNotFound)?;

    // 2. Resolve script path
    let mut script_candidates = vec![PathBuf::from("scraper/extract_ucam.py")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            script_candidates.push(parent.join("scraper/extract_ucam.py"));
            if let Some(grandparent) = parent.parent() {
                script_candidates.push(grandparent.join("scraper/extract_ucam.py"));
            }
        }
    }

    let script_path = script_candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("scraper/extract_ucam.py"));

    // 3. Preserve existing routine as baseline for timetable diffing
    if output_path.exists() {
        let prev_path = {
            let parent = output_path.parent().unwrap_or_else(|| Path::new(""));
            let stem = output_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("routine_data");
            parent.join(format!("{stem}.prev.json"))
        };
        let _ = std::fs::copy(output_path, &prev_path);
    }

    // 4. Build command with xvfb-run on Linux if present and not headed
    let has_xvfb = cfg!(target_os = "linux") && is_executable_available("xvfb-run");

    let mut cmd = if !headed && has_xvfb {
        let mut c = TokioCommand::new("xvfb-run");
        let mut args = vec![
            "--auto-servernum".to_string(),
            "--server-args=-screen 0 1920x1080x24".to_string(),
            python_bin.to_str().unwrap_or("python3").to_string(),
            script_path
                .to_str()
                .unwrap_or("scraper/extract_ucam.py")
                .to_string(),
            "--output".to_string(),
            output_path
                .to_str()
                .unwrap_or("output/routine_data.json")
                .to_string(),
        ];
        if force {
            args.push("--force".to_string());
        }
        c.args(&args);
        c
    } else {
        let mut c = TokioCommand::new(&python_bin);
        c.args([
            script_path.to_str().unwrap_or("scraper/extract_ucam.py"),
            "--output",
            output_path.to_str().unwrap_or("output/routine_data.json"),
        ]);
        if headed {
            c.arg("--headed");
        }
        if force {
            c.arg("--force");
        }
        c
    };

    // Crucial: remove WAYLAND_DISPLAY so Chrome cannot bypass Xvfb and render onto the desktop
    if !headed {
        cmd.env_remove("WAYLAND_DISPLAY");
    }

    let result = run_child_with_timeout(cmd, scraper_timeout()).await?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        return Err(ScraperError::ExecutionFailed(detail));
    }

    if !output_path.exists() {
        return Err(ScraperError::OutputFileNotFound(
            output_path.display().to_string(),
        ));
    }

    Ok(output_path.to_path_buf())
}

/// Runs the Google Sheets routine synchronizer script (`scraper/sync_gsheet.py`).
///
/// This is an async function; the subprocess is bounded by `SCRAPER_TIMEOUT_SECS`.
///
/// # Errors
/// Returns [`ScraperError`] if Python is missing or the sync script fails.
pub async fn run_gsheet_sync(data_path: &Path) -> Result<(), ScraperError> {
    let python_candidates = [
        PathBuf::from(".venv/bin/python"),
        PathBuf::from("python3"),
        PathBuf::from("python"),
    ];

    let python_bin = python_candidates
        .into_iter()
        .find(|p| p.exists() || is_executable_available(p.to_str().unwrap_or("python3")))
        .ok_or(ScraperError::PythonNotFound)?;

    let mut script_candidates = vec![PathBuf::from("scraper/sync_gsheet.py")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            script_candidates.push(parent.join("scraper/sync_gsheet.py"));
            if let Some(grandparent) = parent.parent() {
                script_candidates.push(grandparent.join("scraper/sync_gsheet.py"));
            }
        }
    }

    let script_path = script_candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("scraper/sync_gsheet.py"));

    let mut cmd = TokioCommand::new(&python_bin);
    cmd.args([
        script_path.to_str().unwrap_or("scraper/sync_gsheet.py"),
        "--data",
        data_path.to_str().unwrap_or("output/routine_data.json"),
    ]);

    let result = run_child_with_timeout(cmd, scraper_timeout()).await?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        return Err(ScraperError::ExecutionFailed(detail));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run_child_with_timeout;
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_a_hung_child() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let beat_path = dir.path().join("beat.txt");
        let script_path = dir.path().join("hung.sh");
        let script = format!(
            "#!/bin/sh\nwhile true; do echo tick >> \"{}\"; sleep 0.2; done\n",
            beat_path.display()
        );
        std::fs::write(&script_path, script).expect("write script");
        let mut perms = std::fs::metadata(&script_path)
            .expect("metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("chmod");

        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg(&script_path);

        let start = Instant::now();
        let result = run_child_with_timeout(cmd, Duration::from_millis(800)).await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "hung child should hit the timeout");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("timed out"), "unexpected error: {msg}");
        println!("timeout test elapsed: {elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(10),
            "timeout took too long: {elapsed:?}"
        );

        let beat_count = || -> usize {
            std::fs::read_to_string(&beat_path)
                .map(|s| s.lines().count())
                .unwrap_or(0)
        };

        tokio::time::sleep(Duration::from_millis(500)).await;
        let a = beat_count();
        assert!(
            a > 0,
            "the child never wrote a heartbeat, so the comparison below would prove nothing"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        let b = beat_count();
        assert_eq!(a, b, "child kept running after the timeout");
    }
}
