//! GitHub Gist synchronization via native GitHub REST API with fallback to `gh` CLI.

use std::fs;
use std::path::Path;

/// Pushes the generated `.ics` calendar file to a GitHub Gist.
///
/// Priority:
/// 1. If `GITHUB_TOKEN` or `GH_TOKEN` is set in the environment, uses the native GitHub REST API (`PATCH /gists/:id`).
/// 2. Otherwise falls back to the `gh gist edit` CLI command if available on the system.
///
/// # Errors
/// Returns an error message if both GitHub API and `gh` CLI fail or are unavailable.
pub async fn sync_gist_file(gist_id: &str, ics_path: &Path) -> Result<String, String> {
    let ics_content = fs::read_to_string(ics_path)
        .map_err(|e| format!("Failed to read .ics file at '{}': {e}", ics_path.display()))?;

    let filename = ics_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("routine.ics");

    // 1. Try native GitHub REST API if GITHUB_TOKEN or GH_TOKEN is present
    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok();

    if let Some(token) = token {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "files": {
                filename: {
                    "content": ics_content
                }
            }
        });

        let url = format!("https://api.github.com/gists/{gist_id}");
        let resp = client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", token.trim()))
            .header("User-Agent", "academic-routine-engine/0.1.0")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("GitHub API network error: {e}"))?;

        if resp.status().is_success() {
            return Ok(format!(
                "https://gist.githubusercontent.com/raw/{gist_id}/{filename}"
            ));
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API error (HTTP {status}): {body}"));
    }

    // 2. Fallback to `gh` CLI if token is not set
    let output = std::process::Command::new("gh")
        .args([
            "gist",
            "edit",
            gist_id,
            ics_path.to_str().unwrap_or(filename),
        ])
        .output()
        .map_err(|e| {
            format!(
                "Neither GITHUB_TOKEN environment variable is set, nor 'gh' CLI could be executed: {e}"
            )
        })?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("'gh gist edit' failed: {err}"));
    }

    Ok(format!(
        "https://gist.githubusercontent.com/raw/{gist_id}/{filename}"
    ))
}
