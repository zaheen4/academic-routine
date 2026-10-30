# Operations Runbook

This document covers service management, environment paths, execution pipelines, flag matrices, credential management, troubleshooting, releases, upgrades, and rollback procedures.

## 1. Systemd Units and Execution Cadence

Production automation runs through user-level systemd units located in `~/.config/systemd/user/`:

- **Bot Server (`academic-routine.service`):**
  - **ExecStart:** `%h/.cargo/bin/academic-routine bot`
  - **WorkingDirectory:** `%h/academic-routine`
  - **EnvironmentFile:** `-%h/academic-routine/.env`
  - **Policy:** `Restart=always`, `RestartSec=5s`
  - **Role:** Long-polling Telegram daemon, scheduled daily briefings at 06:00 AM, 30-minute pre-class alerts, and HTTP health check server.
- **Weekly Scraper (`academic-routine-sync.timer` & `academic-routine-sync.service`):**
  - **OnCalendar:** `Sat *-*-* 20:00:00` (Saturday 08:00 PM)
  - **WorkingDirectory:** `%h/academic-routine`
  - **EnvironmentFile:** `-%h/academic-routine/.env`
  - **Settings:** `Persistent=true`, `RandomizedDelaySec=60`
  - **Command:** `%h/.cargo/bin/academic-routine sync-ucam --force`
  - **Linger:** Requires `loginctl enable-linger <user>` so user timers fire without an active desktop session.

> [!IMPORTANT]
> Systemd units fail with `status=203/EXEC` if the binary does not exist at `%h/.cargo/bin/academic-routine` or if `WorkingDirectory` does not match the repository root containing `.venv` and `scraper/`.
>
> The units hard-code `%h/academic-routine`; the setup steps in the README clone there. If your checkout lives elsewhere, edit the **installed copies** under `~/.config/systemd/user/` and run `systemctl --user daemon-reload` — editing the repository copies changes nothing.

## 2. Weekly Execution Pipeline

When `academic-routine-sync.service` executes, it performs the following pipeline in order:

1. **Extraction:** Runs the extractor against every account in `UCAM_ACCOUNTS`; the routine is the union of all accounts, deduplicated. Official attendance is fetched **only** for the account marked `"primary": true` (or the first account when no flag is set). This host runs a primary student account plus a second, schedule-only account that contributes lab sections.
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

## 4. Credentials and Access Tokens

### GitHub Tokens and Gist Identifier

- **`GITHUB_TOKEN`:** Requires a GitHub Personal Access Token (classic) with the `gist` permission.
  - *Rotation Procedure:* Generate new token -> update `GITHUB_TOKEN` in `.env` -> verify via `academic-routine sync-gist` -> revoke expired token on GitHub.
- **`GIST_ID`:** Hexadecimal identifier of the destination Gist.
  - *Configuration:* `academic-routine set-gist <GIST_ID>` updates `output/routine_state.json`.
- **Security Rule:** Never commit tokens, credentials, or actual Gist IDs to source control.

### UCAM Portal Credentials Hierarchy

The scraper resolves student portal credentials using a 4-tier hierarchy (`scraper/extract_ucam.py`):

| Order | Source | Description |
|---|---|---|
| 1 | `UCAM_ACCOUNTS` | JSON array of account objects: `id`, `username`, `password`, `section_label`, optional `primary`. Highest precedence. |
| 2 | `UCAM_USER_<n>_USERNAME` / `_PASSWORD` / `_SECTION` | Numbered environment variables (1-based). `n = 1` also accepts unnumbered `UCAM_USERNAME`, `UCAM_PASSWORD`, `DEFAULT_SECTION`; `n >= 2` defaults section to `B1`. |
| 3 | `configs/ucam_accounts.json` | Local JSON file fallback with shape `{"users": [...]}`. Directory is git-ignored. |
| 4 | `ucam_login_credentials.json` | Legacy root file fallback with shape `{"users": [...]}`. |

If none of the tiers provide valid accounts, the scraper halts immediately:
`No UCAM accounts configured! Set UCAM_ACCOUNTS or UCAM_USERNAME/UCAM_PASSWORD in .env.`

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
| Binary refuses to start after downgrade | On-disk `schema_version` is newer than binary | Upgrade the binary (`cargo install --path . --locked`) or restore a matching state backup. |
| `.corrupt-<timestamp>` quarantine exists | JSON decode failed during atomic read/write | Inspect quarantined file; state automatically restored from `output/routine_state.json.bak`. |
| Gist has extra files | Manual testing or comparison run pushed custom file names | Patch stray file keys to `null` using `gh api -X PATCH "gists/$GIST_ID"`. |
| Telegram bot unresponsive | Process exited or long-polling stalled | Check `systemctl --user status academic-routine.service` and restart daemon. |
| Cloudflare challenge stalls during probes | Process ran without desktop display / cold profile | Run via `xvfb-run --server-args="-screen 0 1920x1080x24"` and allow warmup requests. |

## 9. Release Procedure

A release is an annotated tag. Nothing else is published: no GitHub Release, no `CHANGELOG.md`, no
binary asset. The tag body is the release record and is readable offline with `git show`.

When cutting a new patch or minor release:

1. Update the version string in `Cargo.toml` and `pyproject.toml`.
2. Commit the change and push `main`:
   ```bash
   git push origin main
   ```
3. Confirm GitHub Actions CI completes green:
   ```bash
   gh run list --limit 1
   ```
4. **Documentation Gate:** If the release changes behavior, paths, flags, or contracts, update the affected `docs/` file and its `Last reviewed against vX.Y.Z` stamp **in the same commit**. The release is not ready while `git diff` still shows stale docs.
5. Write the release notes as the annotated tag body. Draft the file first — the tag is write-once, and a typo can only be repaired by moving a tag, which this procedure forbids:
   ```bash
   $EDITOR /tmp/v0.3.3-notes.md
   git tag -a --cleanup=whitespace v0.3.3 -F /tmp/v0.3.3-notes.md
   git push origin v0.3.3
   ```
   `--cleanup=whitespace` is not optional: the default `strip` deletes every line beginning with `#`, which silently removes all markdown headings from the note. `whitespace` trims trailing blank lines and leaves the text intact.
6. **Release Constraints:** No GitHub Release is published (`gh release create` is not used); no binary assets; no `CHANGELOG.md` file in the repository; never modify, re-point, or delete an existing tag.

### Release Note Template

```
v0.3.3 — one-line summary; impact in plain words

Impact: rust · rebuild + restart · schema 2 · no scraper change

- What changed, and the one-line reason.
- ...

Full diff: https://github.com/zaheen4/academic-routine/compare/v0.3.2...v0.3.3
```

The first line becomes the tag subject, which is all `git tag -n` prints, so it must carry the impact
rather than only the title. The `Impact:` line answers the only question an upgrade asks — what has to
be rebuilt, restarted, or re-read — and is the one thing commit history cannot answer. Do not add an
upgrade procedure or a test summary to the notes: the procedure lives in §10, and test counts live in
the commit body.

### Reading the Release History

```bash
git tag -l --sort=v:refname --format='%(refname:short)%n%(contents)%n'   # full history
git show v0.3.2                                                          # a single release
```

Tags created before 2026-10-30 were re-annotated once to absorb their GitHub Release bodies; this is
described in [`decisions.md`](decisions.md). No tag has moved since.

## 10. Upgrade Procedure

Follow these steps to upgrade an existing production deployment to a newer release:

### Step 0: Record Version and Back Up State
```bash
cd ~/academic-routine
academic-routine --version
cp output/routine_state.json "output/routine_state.json.pre-$(academic-routine --version | awk '{print $2}')"
```

### Step 1: Pull Upstream and Check Changes
```bash
git pull --ff-only
git diff --stat HEAD@{1} -- src Cargo.toml Cargo.lock pyproject.toml uv.lock deployment scraper README.md docs
```

### Step 2: Apply What Changed

| Modified Paths in Diff | Required Upgrade Action |
|---|---|
| `scraper/*.py` only | Nothing. Next scrape picks up changes automatically; no rebuild, no restart. |
| `docs/**`, `README.md` only | Nothing. |
| `src/**`, `Cargo.toml`, `Cargo.lock` | Rebuild and restart: `cargo install --path . --locked` then restart daemon. |
| `pyproject.toml`, `uv.lock` | Update Python virtualenv: `uv sync`. |
| `deployment/*.service`, `deployment/*.timer` | Copy units to `~/.config/systemd/user/` then run `systemctl --user daemon-reload`. |
| `STATE_SCHEMA_VERSION` bump | Rebuild and restart immediately; older binaries refuse newer state files. |

Execute applicable update commands:
```bash
uv sync                                       # only if Python dependencies changed
cargo install --path . --locked               # only if src/ or Cargo files changed
cp deployment/academic-routine*.service \
   deployment/academic-routine-sync.timer ~/.config/systemd/user/   # only if units changed
systemctl --user daemon-reload
systemctl --user restart academic-routine.service
```

### Step 3: Run Verification
Execute the checklist in Section 12.

### Step 4: Roll Back if Issues Arise
If defects appear, restore the pre-upgrade state using Section 13.

## 11. Compatibility and State Handling

| Migration Path | Rebuild Required? | Daemon Restart? | State Action |
|---|---|---|---|
| `v0.3.x` -> later `v0.3.x` | Only if `src/` changed | Only if `src/` changed or schema bumped | None required. |
| `v0.2.x` -> `v0.3.x` | Yes | Yes (after install) | Automatic in-place upgrade from schema 1 to 2; do not run older binaries afterwards. |
| Future schema bump (`3+`) | Yes | Yes | Rewritten to new schema on load; older binaries refuse to start. |

### State Backup Retention Policy

Backups follow the format `output/routine_state.json.pre-<version>`. They are created prior to upgrades or manual sync interventions. They remain strictly local and git-ignored. Keep the latest two backups and prune older files.

## 12. Post-Upgrade Verification Checklist

Run these verification steps following an upgrade:

```bash
systemctl --user is-active academic-routine.service          # active
curl -fsS localhost:8000/health                             # 200 + JSON
.venv/bin/python -m pytest -q                               # python test suite
.venv/bin/python scraper/extract_ucam.py --dry-run --force  # full browser dry-run
academic-routine sync-ucam --force                          # real extraction
set -a && . ./.env && set +a
gh api "gists/$GIST_ID" --jq '.files | keys'                # exactly the 3 contract files
grep -c "^BEGIN:VEVENT" output/routine.ics                  # matches exported class count
```

Verify `/today` and `/attendance` in Telegram. Note: a legitimate term rollover may reduce event counts; `--force` accounts for valid term transitions.

## 13. Rollback Procedure

If an upgraded scraper or binary release introduces an operational regression:

1. Revert the commit: `git revert <commit-sha>`.
2. Restore previous routine baseline:
   ```bash
   cp output/routine_data.prev.json output/routine_data.json
   academic-routine export --ics output/routine.ics --sync-gist
   ```
3. If bot state was modified or corrupted, restore from pre-upgrade backup:
   ```bash
   cp output/routine_state.json.pre-<version> output/routine_state.json
   systemctl --user restart academic-routine.service
   ```

*Last reviewed against v0.3.3 (2026-10-30)*
