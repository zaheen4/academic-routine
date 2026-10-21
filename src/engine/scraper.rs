//! Subprocess orchestrator for the standalone UCAM extraction engine.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Runs the standalone UCAM multi-account scraper and returns the generated routine path.
///
/// Runs in offscreen virtual display mode (`xvfb-run` on Linux with `WAYLAND_DISPLAY` stripped)
/// to ensure the browser is completely invisible and no windows pop up on the desktop.
///
/// # Errors
/// Returns [`ScraperError`] if Python is missing, extraction fails, or output is absent.
pub fn run_ucam_scraper(output_path: &Path) -> Result<PathBuf, ScraperError> {
    run_ucam_scraper_opts(output_path, false)
}

/// Runs the standalone UCAM multi-account scraper with explicit visibility options.
///
/// When `headed` is false, runs strictly offscreen via an isolated Xvfb virtual framebuffer.
///
/// # Errors
/// Returns [`ScraperError`] if Python is missing, extraction fails, or output is absent.
pub fn run_ucam_scraper_opts(output_path: &Path, headed: bool) -> Result<PathBuf, ScraperError> {
    // 1. Resolve Python executable
    let python_candidates = [
        PathBuf::from(".venv/bin/python"),
        PathBuf::from("venv/bin/python"),
        PathBuf::from("/home/eyepatch/Documents/projects/routine-to-gsheet/.venv/bin/python"),
        PathBuf::from("python3"),
        PathBuf::from("python"),
    ];

    let python_bin = python_candidates
        .into_iter()
        .find(|p| p.exists() || is_executable_available(p.to_str().unwrap_or("python3")))
        .ok_or(ScraperError::PythonNotFound)?;

    // 2. Resolve script path
    let script_candidates = [
        PathBuf::from("scraper/extract_ucam.py"),
        PathBuf::from("/home/eyepatch/Documents/projects/academic-routine/scraper/extract_ucam.py"),
    ];

    let script_path = script_candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("scraper/extract_ucam.py"));

    // 3. Build command with xvfb-run on Linux if present and not headed
    let has_xvfb = cfg!(target_os = "linux") && is_executable_available("xvfb-run");

    let mut cmd = if !headed && has_xvfb {
        let mut c = Command::new("xvfb-run");
        c.args([
            "--auto-servernum",
            "--server-args=-screen 0 1920x1080x24",
            python_bin.to_str().unwrap_or("python3"),
            script_path.to_str().unwrap_or("scraper/extract_ucam.py"),
            "--output",
            output_path.to_str().unwrap_or("output/routine_data.json"),
        ]);
        c
    } else {
        let mut c = Command::new(&python_bin);
        c.args([
            script_path.to_str().unwrap_or("scraper/extract_ucam.py"),
            "--output",
            output_path.to_str().unwrap_or("output/routine_data.json"),
        ]);
        if headed {
            c.arg("--headed");
        }
        c
    };

    // Crucial: remove WAYLAND_DISPLAY so Chrome cannot bypass Xvfb and render onto the desktop
    if !headed {
        cmd.env_remove("WAYLAND_DISPLAY");
    }

    let result = cmd
        .output()
        .map_err(|e| ScraperError::ExecutionFailed(e.to_string()))?;

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
/// # Errors
/// Returns [`ScraperError`] if Python is missing or the sync script fails.
pub fn run_gsheet_sync(data_path: &Path) -> Result<(), ScraperError> {
    let python_candidates = [
        PathBuf::from(".venv/bin/python"),
        PathBuf::from("venv/bin/python"),
        PathBuf::from("/home/eyepatch/Documents/projects/routine-to-gsheet/.venv/bin/python"),
        PathBuf::from("python3"),
        PathBuf::from("python"),
    ];

    let python_bin = python_candidates
        .into_iter()
        .find(|p| p.exists() || is_executable_available(p.to_str().unwrap_or("python3")))
        .ok_or(ScraperError::PythonNotFound)?;

    let script_candidates = [
        PathBuf::from("scraper/sync_gsheet.py"),
        PathBuf::from("/home/eyepatch/Documents/projects/academic-routine/scraper/sync_gsheet.py"),
    ];

    let script_path = script_candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("scraper/sync_gsheet.py"));

    let mut cmd = Command::new(&python_bin);
    cmd.args([
        script_path.to_str().unwrap_or("scraper/sync_gsheet.py"),
        "--data",
        data_path.to_str().unwrap_or("output/routine_data.json"),
    ]);

    let result = cmd
        .output()
        .map_err(|e| ScraperError::ExecutionFailed(e.to_string()))?;

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
