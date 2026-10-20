//! GitHub Gist synchronization via native GitHub REST API with fallback to `gh` CLI.

use std::fs;
use std::path::Path;

fn resolve_github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok()
        .or_else(|| {
            std::process::Command::new("gh")
                .args(["auth", "token"])
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if !t.is_empty() {
                            Some(t)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
        })
}

/// Pushes both the `.ics` calendar file and structured `routine_data.json` to a GitHub Gist atomically.
///
/// Priority:
/// 1. If `GITHUB_TOKEN` or `GH_TOKEN` is set in the environment, uses the native GitHub REST API (`PATCH /gists/:id`).
/// 2. Otherwise falls back to the `gh gist edit` CLI command if available on the system.
///
/// # Errors
/// Returns an error message if both GitHub API and `gh` CLI fail or are unavailable.
pub async fn sync_gist_bundle(
    gist_id: &str,
    ics_path: &Path,
    json_path: &Path,
) -> Result<String, String> {
    let ics_content = fs::read_to_string(ics_path)
        .map_err(|e| format!("Failed to read .ics file at '{}': {e}", ics_path.display()))?;

    let json_content = fs::read_to_string(json_path)
        .map_err(|e| format!("Failed to read JSON file at '{}': {e}", json_path.display()))?;

    let ics_filename = ics_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("routine.ics");

    let json_filename = json_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("routine_data.json");

    let token = resolve_github_token();

    if let Some(token) = token {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "files": {
                ics_filename: {
                    "content": ics_content
                },
                json_filename: {
                    "content": json_content
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
                "https://gist.githubusercontent.com/raw/{gist_id}/{ics_filename}"
            ));
        }

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API error (HTTP {status}): {body}"));
    }

    // Fallback to sequential `gh gist edit` if token is absent
    let _ = std::process::Command::new("gh")
        .args([
            "gist",
            "edit",
            gist_id,
            "-f",
            ics_filename,
            ics_path.to_str().unwrap_or(ics_filename),
        ])
        .output();

    let output = std::process::Command::new("gh")
        .args([
            "gist",
            "edit",
            gist_id,
            "-a",
            json_path.to_str().unwrap_or(json_filename),
        ])
        .output()
        .map_err(|e| {
            format!(
                "Neither GitHub token is available, nor 'gh' CLI could be executed: {e}"
            )
        })?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("'gh gist edit' failed: {err}"));
    }

    Ok(format!(
        "https://gist.githubusercontent.com/raw/{gist_id}/{ics_filename}"
    ))
}

/// Pushes the generated `.ics` calendar file to a GitHub Gist.
pub async fn sync_gist_file(gist_id: &str, ics_path: &Path) -> Result<String, String> {
    let ics_content = fs::read_to_string(ics_path)
        .map_err(|e| format!("Failed to read .ics file at '{}': {e}", ics_path.display()))?;

    let filename = ics_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("routine.ics");

    let token = resolve_github_token();

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

    let output = std::process::Command::new("gh")
        .args([
            "gist",
            "edit",
            gist_id,
            "-f",
            filename,
            ics_path.to_str().unwrap_or(filename),
        ])
        .output()
        .map_err(|e| {
            format!(
                "Neither GitHub token is available, nor 'gh' CLI could be executed: {e}"
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

/// Fetches the latest `routine_data.json` content from the specified GitHub Gist.
///
/// # Errors
/// Returns an error message if the network request fails or the file is missing from the Gist.
pub async fn fetch_gist_routine_data(gist_id: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok();

    // 1. Try GitHub REST API first (provides real-time content without CDN cache delays)
    let api_url = format!("https://api.github.com/gists/{gist_id}");
    let mut req = client
        .get(&api_url)
        .header("User-Agent", "academic-routine-engine/0.1.0")
        .header("Accept", "application/vnd.github+json");

    if let Some(ref t) = token {
        req = req.header("Authorization", format!("Bearer {}", t.trim()));
    }

    if let Ok(resp) = req.send().await {
        if resp.status().is_success() {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(content) = data
                    .get("files")
                    .and_then(|f| f.get("routine_data.json"))
                    .and_then(|f| f.get("content"))
                    .and_then(|c| c.as_str())
                {
                    return Ok(content.to_string());
                }
            }
        }
    }

    // 2. Fallback to raw gist URL with cache-busting timestamp
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let raw_url = format!(
        "https://gist.githubusercontent.com/raw/{gist_id}/routine_data.json?t={ts}"
    );

    let resp = client
        .get(&raw_url)
        .header("User-Agent", "academic-routine-engine/0.1.0")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch routine from Gist raw URL: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Gist raw download failed with HTTP status {}",
            resp.status()
        ));
    }

    resp.text()
        .await
        .map_err(|e| format!("Failed to read Gist response body: {e}"))
}
