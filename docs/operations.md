# Operations Runbook

This document covers service management, environment paths, execution pipelines, flag matrices, credential management, troubleshooting, releases, and rollback procedures.

## 1. Systemd Units and Execution Cadence

Production automation runs through user-level systemd units located in `~/.config/systemd/user/`:

- **Bot Server (`academic-routine.service`):**
  - **ExecStart:** `%h/.cargo/bin/academic-routine bot`
  - **WorkingDirectory:** `%h/Documents/projects/academic-routine` (or configured repository path)
  - **EnvironmentFile:** `%h/Documents/projects/academic-routine/.env`
  - **Policy:** `Restart=on-failure`, `RestartSec=5s`
  - **Role:** Long-polling Telegram daemon, scheduled daily briefings at 06:00 AM, 30-minute pre-class alerts, and HTTP health check server.
- **Weekly Scraper (`academic-routine-sync.timer` & `academic-routine-sync.service`):**
  - **OnCalendar:** `Sat *-*-* 20:00:00` (Saturday 08:00 PM)
  - **Settings:** `Persistent=true`, `RandomizedDelaySec=60`
  - **Command:** `%h/.cargo/bin/academic-routine sync-ucam --force`
  - **Linger:** Requires `loginctl enable-linger <user>` so user timers fire without an active desktop session.

> [!IMPORTANT]
> Systemd units fail with `status=203/EXEC` if the binary does not exist at `%h/.cargo/bin/academic-routine` or if `WorkingDirectory` does not match the repository root containing `.venv` and `scraper/`.

## 2. Weekly Execution Pipeline

When `academic-routine-sync.service` executes, it performs the following pipeline in order:

1. **Extraction:** Executes `.venv/bin/python scraper/extract_ucam.py` using credentials configured in `UCAM_ACCOUNTS`. Scrapes Account 1 (routine and attendance) and Account 2 (donor lab sections).
2. **Merge & Deduplicate:** Combines slots across accounts (e.g., 13 entries each merging to 16 unique classes for theory B and labs B1/B2).
3. **Downgrade Guard:** Replaces `output/routine_data.json` only if the new entry count meets or exceeds the previous baseline, unless `--force` is set.
4. **Attendance Auto-Import:** If official attendance is extracted, writes `output/attendance_data.json` and updates `output/routine_state.json` for the admin chat (`ADMIN_CHAT_ID`).
5. **Calendar Export:** Generates RFC 5545 iCalendar feed at `output/routine.ics` with 30-minute alarms and weekly recurrence rules.
6. **Cloud Gist Sync:** Uploads `routine.ics`, `routine_data.json`, and `routine_meta.json` to the configured GitHub Gist.
7. **Google Sheets Sync:** Publishes schedule tables to the configured Google Spreadsheet (`NewMain` and `backend` tabs).

## 3. Command Flag Matrix

### Rust CLI (`academic-routine sync-ucam`)

| Flag | Purpose & Effect |
|---|---|
| `--force` | Re-scrapes immediately; bypasses the row-count downgrade guard. |
| `--headed` | Runs browser with a visible display for local visual debugging. |
| `--local-only` | Writes local `routine_data.json` and `routine.ics`; skips Gist and Google Sheets sync. |
| `--no-gist` | Skips updating GitHub Gist while allowing Google Sheets sync. |
| `--no-gsheet` | Skips updating Google Spreadsheet while allowing Gist publication. |

### Scraper CLI (`scraper/extract_ucam.py`)

| Flag | Purpose & Effect |
|---|---|
| `-o`, `--output` | Destination path for routine JSON output (default: `output/routine_data.json`). |
| `--force` | Bypasses row-count downgrade guard against an existing output file. |
| `--dry-run` | Performs full login, extraction, and parsing across accounts without writing files. |
| `--html-file` | Parses an offline saved HTML report without launching a browser or network connection. |
| `--section` | Fallback section label when row section is omitted (default: `B`). |
| `--headed` | Runs browser visibly instead of offscreen headless mode. |

## 4. Tokens and Identifiers

- **`GITHUB_TOKEN`:** Requires a GitHub Personal Access Token (classic) with the `gist` permission.
  - *Rotation Procedure:* Generate new token -> update `GITHUB_TOKEN` in `.env` -> verify via `academic-routine sync-gist` -> revoke expired token on GitHub.
- **`GIST_ID`:** Hexadecimal identifier of the destination Gist.
  - *Configuration:* `academic-routine set-gist <GIST_ID>` updates `output/routine_state.json`.
- **Security Rule:** Never commit tokens, credentials, or actual Gist IDs to source control.

## 5. GitHub Gist File Contract

The published Gist must maintain **strictly three files**:

1. `routine.ics`: iCalendar subscription feed.
2. `routine_data.json`: Current raw class schedule JSON array.
3. `routine_meta.json`: Administrative metadata containing `schema_version`, `semester_name`, `semester_end_date`, `owner_dropped_courses`, and `updated_at`.

### Purging Stray Gist Files

If stray comparison files (e.g., `current.ics`, `fixed.ics`) appear on the Gist, remove them via GitHub CLI:

```bash
set -a && . ./.env && set +a
gh api -X PATCH "gists/$GIST_ID" --input - <<'JSON'
{"files": {"stray_file.ics": null}}
JSON
```

Verify exact file bundle:
```bash
gh api "gists/$GIST_ID" --jq '.files | keys'
# Expected: ["routine.ics", "routine_data.json", "routine_meta.json"]
```

## 6. Google Sheets Integration

- **Spreadsheet Target:** Configured by `SPREADSHEET_NAME` in `.env` (default: `CSE-03_B_ClassRoutine`).
- **Credentials:** Service account JSON specified via `GOOGLE_SERVICE_ACCOUNT_KEY`.
- **Tabs:**
  - `NewMain`: High-density visual weekly schedule formatted with merged time cells.
  - `backend`: Flat row-level records utilized by downstream automation.
- **Fault Tolerance:** Google Sheets API errors or quota limits log a warning and do not block calendar generation or Gist publication.

## 7. HTTP Health Endpoint

The daemon embeds an HTTP health server:

- **Endpoints:** `GET /health` and `GET /` return HTTP 200 with status JSON.
- **Configuration:** Port set via `PORT` (default: `8000`).
- **Disable:** Set `DISABLE_HTTP_HEALTHCHECK=1` in `.env` to disable the embedded HTTP listener without port manipulation.

## 8. Troubleshooting Matrix

| Symptom | Probable Cause | Corrective Action |
|---|---|---|
| Zero routine entries extracted | UCAM report layout or header label changed | Run offline parser on captured DOM: `python scraper/extract_ucam.py --html-file <file> --dry-run` to inspect headers. |
| Attendance grid never rendered | Dropdown postback timed out or element id changed | Verify `#ctl00_MainContainer_ddlHeldIn` is populated and change events dispatch properly. |
| Downgrade refused by parser | Extracted count is lower than existing file count | Inspect portal for missing courses; if legitimate reduction, run with `--force`. |
| Binary refuses to start after downgrade | On-disk `schema_version` is newer than binary | Upgrade the binary (`cargo install --path .`) or restore a matching state backup. |
| `.corrupt-<timestamp>` quarantine exists | JSON decode failed during atomic read/write | Inspect quarantined file; state automatically restored from `output/routine_state.json.bak`. |
| Gist has extra files | Manual testing or comparison run pushed custom file names | Patch stray file keys to `null` using `gh api -X PATCH "gists/$GIST_ID"`. |
| Telegram bot unresponsive | Process exited or long-polling stalled | Check `systemctl --user status academic-routine.service` and restart daemon. |
| Cloudflare challenge stalls during probes | Process ran without desktop display / cold profile | Run via `xvfb-run --server-args="-screen 0 1920x1080x24"` and allow warmup requests. |

## 9. Release Procedure

When cutting a new patch or minor release:

1. Update version string in `Cargo.toml` and `pyproject.toml`.
2. Commit changes and push `main` branch to remote.
3. Confirm GitHub Actions CI completes green:
   ```bash
   gh run list --limit 1
   ```
4. Create an annotated git tag named after the release title:
   ```bash
   git tag -a v0.3.X -m "v0.3.X — Release summary title"
   git push origin v0.3.X
   ```
5. Publish release notes via GitHub CLI:
   ```bash
   gh release create v0.3.X --title "v0.3.X — Release summary title" --notes-file /tmp/release-notes.md
   ```
6. **Release Constraints:** No binary assets attached to the release; no `CHANGELOG.md` file in the repository; never modify or delete existing tags (`v0.3.0`, `v0.3.1`, `v0.3.2`).

## 10. Rollback Procedure

If a deployed scraper or binary release introduces an operational issue:

1. Revert the commit: `git revert <commit-sha>`.
2. Restore previous routine baseline:
   ```bash
   cp output/routine_data.prev.json output/routine_data.json
   academic-routine export --ics output/routine.ics --sync-gist
   ```
3. If bot state requires recovery, restore from pre-sync backup:
   ```bash
   cp /tmp/routine_state.pre-attendance.json output/routine_state.json
   systemctl --user restart academic-routine.service
   ```

*Last reviewed against v0.3.2 (2026-10-29)*
