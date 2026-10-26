//! GitHub Gist synchronization via the native GitHub REST API with a `gh` CLI fallback.
//!
//! The public bundle is deliberately limited to `routine.ics`, `routine_data.json`,
//! and a sanitized `routine_meta.json`. The private `routine_state.json` is never
//! uploaded and is removed from legacy Gists on the first sync after upgrade.

use crate::model::state::PublicRoutineMeta;
use std::fs;
use std::path::Path;
use std::process::Command;

const ROUTINE_META_FILENAME: &str = "routine_meta.json";
const LEGACY_STATE_FILENAME: &str = "routine_state.json";

const USER_AGENT: &str = concat!("academic-routine-engine/", env!("CARGO_PKG_VERSION"));

/// Builds the PATCH payload used to retry after a `422 Unprocessable Entity`.
///
/// GitHub rejects a PATCH that deletes a file the Gist does not contain, so the
/// `routine_state.json: null` entry is stripped before the retry. Returns `None`
/// when the bundle has no legacy deletion key, in which case a retry would send
/// the same request again.
fn retry_payload_without_legacy(files: &serde_json::Value) -> Option<serde_json::Value> {
    let mut retry = files.as_object()?.clone();
    retry.remove(LEGACY_STATE_FILENAME)?;
    Some(serde_json::json!({ "files": retry }))
}

/// Resolves a GitHub token from the environment or the `gh` CLI.
fn resolve_github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok()
        .filter(|token| !token.trim().is_empty())
        .or_else(|| {
            let output = Command::new("gh").args(["auth", "token"]).output().ok()?;
            if !output.status.success() {
                return None;
            }
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!token.is_empty()).then_some(token)
        })
}

/// Builds the exact file map uploaded to the public Gist.
///
/// `routine_state.json` is set to JSON `null`, which tells the GitHub API to
/// delete it from legacy Gists. No private state is ever placed in this map.
#[must_use]
pub fn build_bundle_files(
    ics_filename: &str,
    ics_content: &str,
    json_filename: &str,
    json_content: &str,
    meta: &PublicRoutineMeta,
) -> serde_json::Value {
    let meta_json = serde_json::to_string_pretty(meta).unwrap_or_else(|_| "{}".to_string());
    let mut files = serde_json::Map::new();
    files.insert(
        ics_filename.to_string(),
        serde_json::json!({ "content": ics_content }),
    );
    files.insert(
        json_filename.to_string(),
        serde_json::json!({ "content": json_content }),
    );
    files.insert(
        ROUTINE_META_FILENAME.to_string(),
        serde_json::json!({ "content": meta_json }),
    );
    files.insert(LEGACY_STATE_FILENAME.to_string(), serde_json::Value::Null);
    serde_json::Value::Object(files)
}

/// Pushes the calendar, routine data, and sanitized metadata to a GitHub Gist.
///
/// Priority:
/// 1. `GITHUB_TOKEN`/`GH_TOKEN`/`gh auth token` with the GitHub REST API.
/// 2. `gh gist edit`, selecting existing files and adding missing ones.
///
/// # Errors
/// Returns an error message if both the API and the `gh` fallback fail.
pub async fn sync_gist_bundle(
    gist_id: &str,
    ics_path: &Path,
    json_path: &Path,
    meta: &PublicRoutineMeta,
) -> Result<String, String> {
    let ics_content = fs::read_to_string(ics_path)
        .map_err(|e| format!("Failed to read .ics file at '{}': {e}", ics_path.display()))?;
    let json_content = fs::read_to_string(json_path)
        .map_err(|e| format!("Failed to read JSON file at '{}': {e}", json_path.display()))?;

    let ics_filename = ics_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("routine.ics");
    let json_filename = json_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("routine_data.json");

    let files = build_bundle_files(
        ics_filename,
        &ics_content,
        json_filename,
        &json_content,
        meta,
    );

    if let Some(token) = resolve_github_token() {
        let client = reqwest::Client::new();
        let url = format!("https://api.github.com/gists/{gist_id}");
        let mut response = client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", token.trim()))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "files": files }))
            .send()
            .await
            .map_err(|e| format!("GitHub API network error: {e}"))?;

        // GitHub returns 422 Unprocessable Entity when the Gist does not contain the legacy
        // `routine_state.json` that the bundle asks to delete. Retry without that key; the file
        // being absent is already the desired end state.
        if response.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            if let Some(payload) = retry_payload_without_legacy(&files) {
                response = client
                    .patch(&url)
                    .header("Authorization", format!("Bearer {}", token.trim()))
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "application/vnd.github+json")
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| format!("GitHub API network error: {e}"))?;
            }
        }

        if response.status().is_success() {
            return Ok(feed_url(gist_id, ics_filename));
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("GitHub API error (HTTP {status}): {body}"));
    }

    // Fallback: `gh` is available but no token env var is set.
    let meta_json = serde_json::to_string_pretty(meta)
        .map_err(|e| format!("Failed to serialize routine metadata: {e}"))?;
    for (filename, content) in [
        (ics_filename, ics_content.as_str()),
        (json_filename, json_content.as_str()),
        (ROUTINE_META_FILENAME, meta_json.as_str()),
    ] {
        gh_gist_upsert(gist_id, filename, content)?;
    }
    // Best-effort cleanup of the legacy state file; absence is fine.
    let _ = gh_gist_remove(gist_id, LEGACY_STATE_FILENAME);

    Ok(feed_url(gist_id, ics_filename))
}

/// Pushes a single generated `.ics` calendar file to a GitHub Gist.
///
/// # Errors
/// Returns an error message if both the API and `gh` fallback fail.
pub async fn sync_gist_file(gist_id: &str, ics_path: &Path) -> Result<String, String> {
    let ics_content = fs::read_to_string(ics_path)
        .map_err(|e| format!("Failed to read .ics file at '{}': {e}", ics_path.display()))?;
    let filename = ics_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("routine.ics");

    if let Some(token) = resolve_github_token() {
        let client = reqwest::Client::new();
        let url = format!("https://api.github.com/gists/{gist_id}");
        let mut files = serde_json::Map::new();
        files.insert(
            filename.to_string(),
            serde_json::json!({ "content": ics_content }),
        );
        let response = client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", token.trim()))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "files": serde_json::Value::Object(files) }))
            .send()
            .await
            .map_err(|e| format!("GitHub API network error: {e}"))?;

        if response.status().is_success() {
            return Ok(feed_url(gist_id, filename));
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("GitHub API error (HTTP {status}): {body}"));
    }

    gh_gist_upsert(gist_id, filename, &ics_content)?;
    Ok(feed_url(gist_id, filename))
}

fn feed_url(gist_id: &str, filename: &str) -> String {
    format!("https://gist.githubusercontent.com/raw/{gist_id}/{filename}")
}

/// Lists the filenames currently present in a Gist via the `gh` CLI.
fn gh_gist_files(gist_id: &str) -> Result<Vec<String>, String> {
    let output = Command::new("gh")
        .args([
            "api",
            &format!("gists/{gist_id}"),
            "--jq",
            ".files | keys[]",
        ])
        .output()
        .map_err(|e| {
            format!("Neither GITHUB_TOKEN/GH_TOKEN is set nor could 'gh' be executed: {e}")
        })?;

    if !output.status.success() {
        return Err(format!(
            "'gh api gists/{gist_id}' failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

/// Creates or replaces one file in a Gist through the `gh` CLI.
fn gh_gist_upsert(gist_id: &str, filename: &str, content: &str) -> Result<(), String> {
    let existing = gh_gist_files(gist_id)?;
    let staging_dir = std::env::temp_dir().join(format!(
        "academic-routine-gist-{}-{}",
        std::process::id(),
        sanitize_filename(filename)
    ));
    fs::create_dir_all(&staging_dir).map_err(|e| {
        format!(
            "Failed to create staging dir '{}': {e}",
            staging_dir.display()
        )
    })?;
    // The file basename must be exactly the target name for `--add`.
    let staged = staging_dir.join(filename);
    fs::write(&staged, content).map_err(|e| format!("Failed to stage '{filename}': {e}"))?;

    let result = if existing.iter().any(|name| name == filename) {
        Command::new("gh")
            .args([
                "gist",
                "edit",
                gist_id,
                "--filename",
                filename,
                staged.to_str().unwrap_or_default(),
            ])
            .output()
    } else {
        Command::new("gh")
            .args([
                "gist",
                "edit",
                gist_id,
                "--add",
                staged.to_str().unwrap_or_default(),
            ])
            .output()
    };

    let _ = fs::remove_dir_all(&staging_dir);

    let output = result.map_err(|e| format!("Could not execute 'gh gist edit': {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "'gh gist edit' failed for '{filename}': {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Removes one file from a Gist through the `gh` CLI.
fn gh_gist_remove(gist_id: &str, filename: &str) -> Result<(), String> {
    let output = Command::new("gh")
        .args(["gist", "edit", gist_id, "--remove", filename])
        .output()
        .map_err(|e| format!("Could not execute 'gh gist edit': {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "'gh gist edit --remove {filename}' failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// Fetches the latest `routine_data.json` content from the specified GitHub Gist.
///
/// # Errors
/// Returns an error message if the network request fails or the file is missing.
pub async fn fetch_gist_routine_data(gist_id: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok();

    // 1. GitHub REST API first (real-time content, no CDN cache delay).
    let api_url = format!("https://api.github.com/gists/{gist_id}");
    let mut request = client
        .get(&api_url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json");
    if let Some(ref token) = token {
        request = request.header("Authorization", format!("Bearer {}", token.trim()));
    }

    if let Ok(response) = request.send().await {
        if response.status().is_success() {
            if let Ok(data) = response.json::<serde_json::Value>().await {
                if let Some(content) = data
                    .get("files")
                    .and_then(|files| files.get("routine_data.json"))
                    .and_then(|file| file.get("content"))
                    .and_then(serde_json::Value::as_str)
                {
                    return Ok(content.to_string());
                }
            }
        }
    }

    // 2. Raw URL with cache busting.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let raw_url =
        format!("https://gist.githubusercontent.com/raw/{gist_id}/routine_data.json?t={ts}");
    let response = client
        .get(&raw_url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch routine from Gist raw URL: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Gist raw download failed with HTTP status {}",
            response.status()
        ));
    }

    response
        .text()
        .await
        .map_err(|e| format!("Failed to read Gist response body: {e}"))
}

/// Fetches and parses the sanitized `routine_meta.json` if the Gist has one.
///
/// Returns `Ok(None)` for legacy Gists without metadata; network failures are
/// reported as errors so callers can distinguish "absent" from "unreachable".
pub async fn fetch_gist_meta_data(gist_id: &str) -> Result<Option<PublicRoutineMeta>, String> {
    let client = reqwest::Client::new();
    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok();

    let api_url = format!("https://api.github.com/gists/{gist_id}");
    let mut request = client
        .get(&api_url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json");
    if let Some(ref token) = token {
        request = request.header("Authorization", format!("Bearer {}", token.trim()));
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Gist metadata: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "GitHub API request failed with HTTP status {}",
            response.status()
        ));
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gist JSON response: {e}"))?;

    if let Some(content) = data
        .get("files")
        .and_then(|files| files.get(ROUTINE_META_FILENAME))
        .and_then(|file| file.get("content"))
        .and_then(serde_json::Value::as_str)
    {
        return serde_json::from_str(content)
            .map(Some)
            .map_err(|e| format!("Invalid {ROUTINE_META_FILENAME}: {e}"));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn sample_meta() -> PublicRoutineMeta {
        PublicRoutineMeta {
            schema_version: 2,
            semester_name: "L4T1".to_string(),
            semester_end_date: NaiveDate::from_ymd_opt(2026, 11, 30),
            owner_dropped_courses: vec!["CSE-4125".to_string()],
            updated_at: "2026-10-25T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_bundle_contains_only_public_files() {
        let files = build_bundle_files(
            "routine.ics",
            "BEGIN:VCALENDAR",
            "routine_data.json",
            "{}",
            &sample_meta(),
        );
        let object = files.as_object().unwrap();
        let names: Vec<&String> = object.keys().collect();
        assert!(names.contains(&&"routine.ics".to_string()));
        assert!(names.contains(&&"routine_data.json".to_string()));
        assert!(names.contains(&&"routine_meta.json".to_string()));
        assert!(names.contains(&&"routine_state.json".to_string()));
        assert_eq!(object.len(), 4);
        assert!(object.get("routine_state.json").unwrap().is_null());
    }

    #[test]
    fn test_retry_payload_drops_only_the_legacy_deletion() {
        let files = build_bundle_files(
            "routine.ics",
            "BEGIN:VCALENDAR",
            "routine_data.json",
            "{}",
            &sample_meta(),
        );

        let payload = retry_payload_without_legacy(&files).expect("legacy key is present");
        let retried = payload["files"].as_object().unwrap();
        assert!(!retried.contains_key("routine_state.json"));
        assert!(retried.contains_key("routine.ics"));
        assert!(retried.contains_key("routine_data.json"));
        assert!(retried.contains_key("routine_meta.json"));
        assert_eq!(retried.len(), 3);

        // The first attempt must still ask for the deletion.
        assert!(files["routine_state.json"].is_null());
    }

    #[test]
    fn test_retry_payload_is_none_without_legacy_key() {
        let mut files = build_bundle_files(
            "routine.ics",
            "BEGIN:VCALENDAR",
            "routine_data.json",
            "{}",
            &sample_meta(),
        );
        files.as_object_mut().unwrap().remove("routine_state.json");

        assert!(retry_payload_without_legacy(&files).is_none());
        assert!(retry_payload_without_legacy(&serde_json::json!([])).is_none());
    }

    #[test]
    fn test_bundle_meta_has_no_pii() {
        let meta = sample_meta();
        let files = build_bundle_files("routine.ics", "", "routine_data.json", "{}", &meta);
        let serialized = serde_json::to_string(&files).unwrap();
        for forbidden in [
            "user_attendance",
            "user_sections",
            "telegram_username",
            "owner_username",
            "muted_morning_alerts",
            "user_preclass_reminders",
            "reminders_sent",
            "gist_id",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
    }
}
