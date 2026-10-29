# Academic Routine & Attendance Assistant

[![Rust](https://img.shields.io/badge/Rust-2021%20Edition-orange?logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Docker Ready](https://img.shields.io/badge/Docker-Ready-2496ED?logo=docker)]()

An autonomous academic timetable engine, RFC 5545 iCalendar synchronizer, official university attendance auditor, and multi-user Telegram assistant built with Rust.

---

## Highlights

- **High-Performance Schedule Core**: Zero-allocation Rust parser handling university timetable schemas, section filtering (`B1`, `B2`, theory `B`), and room double-booking conflict detection.
- **Multi-User Telegram Assistant**: Interactive inline keyboard interface with silent callbacks. Supports personalized section filtering, morning briefings, pre-class reminders, and attendance auditing across multiple users.
- **Live Calendar Subscriptions**: Native Apple Calendar (`webcal://`) and Google Calendar web subscriptions. Devices automatically synchronize when schedules or room allocations change without downloading static files.
- **University Attendance Tracker**: Enforces official university attendance regulations (`Collegiate` $\ge 75\%$, `Non-Collegiate` $60\text{--}74\%$, `Dis-Collegiate` $<60\%$). Calculates exact safe bunk allowances and required recovery classes.
- **Timetable Diff Detection**: Detects room swaps, time shifts, teacher reassignments, and course additions/removals across schedule versions.
- **Faculty Directory**: Search instructors by name, initials, or course code with direct phone call and email action buttons.
- **Decoupled Cloud Architecture**: Scrapes locally or via residential worker, publishes to a synchronized GitHub Gist bundle, and streams updates to cloud bot servers with zero Cloudflare friction.

---

## Documentation

Depth lives in [`docs/`](docs/index.md) — the README stays the front door:

- [`docs/ucam-scraping.md`](docs/ucam-scraping.md) — the UCAM portal contract, re-capture procedure, and fixture sanitization.
- [`docs/operations.md`](docs/operations.md) — systemd units, tokens, Gist/Sheet contracts, troubleshooting, releases.
- [`docs/state-model.md`](docs/state-model.md) — local vs published artifacts, schema v2, concurrency, attendance import.
- [`docs/decisions.md`](docs/decisions.md) — why settled behaviours are settled.

---

## System Architecture

```mermaid
flowchart TD
    classDef source fill:#f8fafc,stroke:#64748b,stroke-width:1.5px,color:#0f172a;
    classDef scraper fill:#eff6ff,stroke:#3b82f6,stroke-width:1.5px,color:#1e3a8a;
    classDef core fill:#fef3c7,stroke:#f59e0b,stroke-width:1.5px,color:#78350f;
    classDef cloud fill:#f3e8ff,stroke:#a855f7,stroke-width:1.5px,color:#581c87;
    classDef client fill:#ecfdf5,stroke:#10b981,stroke-width:1.5px,color:#064e3b;

    subgraph Sources ["Upstream University Services"]
        UCAM["UCAM Portal: Timetable and Faculty Directory"]:::source
    end

    subgraph Scraping ["Automated Ingestion"]
        Worker["Headless nodriver Scraper: CDP Stealth and Session Rotation"]:::scraper
    end

    subgraph Engine ["Rust Core Processing Engine"]
        direction TB
        Parser["JSON Parser and Schema Validator"]:::core
        DiffEngine["Baseline Diff Engine: Room and Time Detection"]:::core
        SectionFilter["Section Filtering Engine: B1, B2 and Theory Alignment"]:::core
        ConflictAuditor["Conflict and Overlap Auditor"]:::core
        AttendanceAuditor["Attendance and Safe Bunk Auditor"]:::core
        ICalGenerator["RFC 5545 iCalendar Generator: Alarms and Recurrence"]:::core
    end

    subgraph SyncLayer ["Cloud Synchronization and State Cache"]
        direction TB
        GistBundle[("GitHub Gist Cloud Bundle: routine.ics, routine_data.json, routine_meta.json")]:::cloud
        LocalState[("Local Baseline Cache: routine_data.prev.json, attendance_data.json, routine_state.json")]:::cloud
    end

    subgraph Delivery ["Consumer Endpoints and Multi-Channel Delivery"]
        direction TB
        subgraph TelegramDaemon ["Telegram Assistant Daemon"]
            TGBot["Teloxide Bot Server and Callback Dispatcher"]:::client
            MorningBriefings["06:00 AM Daily Briefing Service"]:::client
            PreclassAlerts["30-Minute Pre-Class Alert Service"]:::client
            HealthServer["HTTP Health Check Server: GET /health"]:::client
        end
        AppleCal["Apple Calendar: webcal:// Subscription"]:::client
        GoogleCal["Google Calendar: Direct Account Sync"]:::client
        GSheet["Google Sheets: Formatted Schedule Grid"]:::client
        TerminalCLI["Terminal CLI: Table Formatter"]:::client
    end

    UCAM -->|Residential Scrape| Worker
    Worker -->|Raw Timetable Data| Parser

    Parser --> SectionFilter
    Parser --> ConflictAuditor
    Parser --> DiffEngine
    DiffEngine <--> LocalState
    AttendanceAuditor <--> LocalState

    SectionFilter --> ICalGenerator
    ICalGenerator --> GistBundle
    Parser --> GistBundle

    GistBundle -->|webcal Protocol| AppleCal
    GistBundle -->|Web Import CID| GoogleCal
    GistBundle -->|Cloud Polling Fallback| TGBot
    LocalState --> TGBot

    TGBot --> MorningBriefings
    TGBot --> PreclassAlerts

    SectionFilter --> GSheet
    SectionFilter --> TerminalCLI
```

---

## Telegram Bot Commands

| Command | Arguments | Description |
| :--- | :--- | :--- |
| `/start` | — | Launch the assistant and show the weekly routine overview. |
| `/help` | — | Command reference and inline keyboard shortcuts. |
| `/today` | — | Today's classes, room allocations, instructor info, and active session status. |
| `/tomorrow` | — | Tomorrow's class schedule with rooms and instructors. |
| `/next` | — | Countdown timer to the immediate next class session. |
| `/routine` | — | Interactive weekly timetable with individual day navigation buttons. |
| `/section` | `[B1 / B2 / All]` | Set or inspect your personalized student section. |
| `/diff` | — | Display room swaps, time shifts, and schedule changes against baseline. |
| `/attendance` | — | View official collegiate status, attendance percentage, and safe bunk math. |
| `/attend` | `<CODE>` | Mark a class session as attended. |
| `/miss` | `<CODE>` | Mark a class session as missed or bunked. |
| `/attendance set`| `<CODE> <ATT> <MIS>`| Set exact attended and missed counts for a course. |
| `/teacher` | `<NAME / CODE>` | Search faculty directory with phone and email action buttons. |
| `/teachers` | — | List every instructor in the directory with contact actions. |
| `/calendar` | — | Live calendar subscription card for Apple and Google Calendar sync. |
| `/alerts` | `[on / off]` | Toggle 6:00 AM daily morning briefings (active by default). |
| `/reminders` | `[on / off]` | Toggle 30-minute pre-class reminders (opt-in). |
| `/free` | — | Calculate free periods, study windows, and break durations. |
| `/complete` | `<CODE>` | Mark completed courses (hides from alerts and calendar feeds). |
| `/completed` | — | List courses currently marked as completed. |
| `/resume` | `<CODE>` | Restore a completed or dropped course to your active routine. |
| `/vacation` | `[on / off / until YYYY-MM-DD]` | Pause briefings and reminders; `until` auto-resumes after that date (owner only). |
| `/sync` | — | Pull latest routine updates from GitHub Gist or UCAM portal (owner only). |
| `/status` | — | View system health, active section, vacation state, and alert preferences. |

Aliases are registered but hidden from the menu: `/week`, `/finish`, `/finished`, `/drop`, `/restore`, `/dropped`, `/bunk`, `/present`, and `/absent`.

---

## Attendance Auditor Regulations

The attendance subsystem calculates safe bunks and collegiate standing according to official university rules:

$$
\text{Attendance Percentage} = \frac{\text{Attended Classes}}{\text{Total Held Classes}} \times 100
$$

- **Collegiate ($\ge 75\%$)**: Eligible for final examinations without penalty.
- **Non-Collegiate ($60\% - 74.9\%$)**: Allowed to sit for examinations subject to university fine.
- **Dis-Collegiate ($< 60\%$)**: Disqualified from examinations.

### Safe Bunk and Recovery Mathematics
- **Safe Bunks Allowed**:
  $$\left\lfloor \frac{\text{Attended} - 3 \times \text{Missed}}{3} \right\rfloor$$
- **Classes Needed for Collegiate**:
  $$\max(0, 3 \times \text{Missed} - \text{Attended})$$

> **Privacy & Self-Hosting Model**: Automated UCAM attendance scraping is strictly a self-hosting privilege. The bot never solicits or stores student portal passwords from other users. On a shared bot instance, classmates can manually record their sessions (`/attend`, `/miss`, `/attendance set`) to calculate collegiate thresholds in their private chat. To enable automated attendance extraction directly from UCAM, run your own self-hosted instance using personal credentials in `.env`. On a self-hosted instance the weekly scrape also mirrors the official attendance grid into `/attendance` for the admin account; see the **Official Attendance Import** note under [Environment Configuration](#environment-configuration).

---

## Live Calendar Subscription Setup

Rather than static `.ics` file downloads that become stale when rooms change, the system publishes a live auto-updating calendar feed:

- **Apple Calendar (iOS / iPadOS / macOS)**:
  Tapping **Apple Calendar** opens `webcal://gist.githubusercontent.com/raw/<GIST_ID>/routine.ics`, triggering native one-tap calendar subscription.
- **Google Calendar (Android / Web)**:
  Tapping **Google Calendar** links `https://calendar.google.com/calendar/r?cid=webcal%3A%2F%2F...` directly to the user's Google account, syncing down to mobile devices.
- **Outlook & WebCal**:
  Copy the raw feed link directly into your calendar client.

---

## Command-Line Interface (CLI)

```bash
# Global flags: --file <PATH> selects an explicit routine JSON, --section <B1|B2|...> filters it

# Daily views
academic-routine today
academic-routine next
academic-routine view                    # full week
academic-routine view --day Mon          # single day
academic-routine --section B2 today

# Schedule auditing
academic-routine diff output/routine_data.prev.json output/routine_data.json
academic-routine conflicts

# Faculty directory
academic-routine teachers
academic-routine teacher basar

# Attendance state and course lifecycle
academic-routine status
academic-routine complete CSE-4125       # alias: drop
academic-routine resume CSE-4125         # alias: restore
academic-routine completed               # alias: dropped

# Semester switches
academic-routine vacation on --until 2026-11-15
academic-routine alerts off
academic-routine pause gist              # pause | unpause: alerts | gist | gsheet | all
academic-routine timer status            # status | pause | resume | run

# Calendar export and publication
academic-routine export --ics output/routine.ics --sync-gist
academic-routine export --until 2026-12-31 --include-all
academic-routine set-gist <GIST_ID>
academic-routine sync-gist

# Scraping and spreadsheet sync
academic-routine sync-ucam               # --force re-scrapes, --local-only skips publishing
academic-routine sync-ucam --dry-run     # full two-account run, writes no files
academic-routine sync-gsheet

# Offline parser, no browser (fixtures are sanitized live captures)
.venv/bin/python scraper/extract_ucam.py \
  --html-file scraper/tests/fixtures/routine_rowspan_2026.html --dry-run

# Telegram assistant with embedded HTTP health-check server (--no-http to disable)
academic-routine bot
```

> [!NOTE]
> On read commands, `--file` is strict: a path that does not exist is an error rather than a silent fallback to `output/routine_data.json`. Omit the flag to keep the `output/` → repository-root search order.

---

## Local Development & Setup

### Prerequisites
- [Rust](https://www.rust-lang.org/tools/install) 1.89+ (2021 edition, declared as `rust-version` in `Cargo.toml`)
- [Python 3.10+](https://www.python.org/) with `uv` (`uv sync`)
- System Chromium or Google Chrome with `xvfb-run` (for offscreen headless scraping on Linux)

### Environment Configuration
Copy `.env.example` to `.env` and configure credentials:

```bash
cp .env.example .env
```

| Variable | Description |
| :--- | :--- |
| `TELEGRAM_BOT_TOKEN` | Telegram Bot API token from @BotFather (`TELOXIDE_TOKEN` also supported) |
| `ADMIN_CHAT_ID` | Numeric Telegram user ID of owner for admin gate & notifications (`TELEGRAM_CHAT_ID` fallback) |
| `TELEGRAM_USERNAME` | Optional owner handle shown in `status` and alert lines when Telegram does not supply one |
| `DEFAULT_SECTION` | Default section filter (e.g. `B2`, `B1`, or `All`) |
| `GITHUB_TOKEN` | GitHub Personal Access Token with `gist` scope (`GH_TOKEN` also supported) |
| `GIST_ID` | GitHub Gist ID for public routine and calendar feed distribution |
| `ROUTINE_TIMEZONE_OFFSET` | Local timezone offset in hours from UTC (defaults to `6`, Asia/Dhaka) |
| `PORT` | Health check server port (defaults to 8000) |
| `DISABLE_HTTP_HEALTHCHECK` | Set to `1` to skip the embedded health check server instead of using `PORT=0` |
| `SPREADSHEET_NAME` | Target Google Spreadsheet for `sync-gsheet` (default `CSE-03_B_ClassRoutine`) |
| `GOOGLE_SERVICE_ACCOUNT_KEY` | Path to the Google service-account JSON key used for Sheets sync |
| `UCAM_ACCOUNTS` | JSON array of UCAM portal accounts (`id`, `username`, `password`, `section_label`) read by the scraper |

> [!IMPORTANT]
> `UCAM_ACCOUNTS` holds raw university portal passwords. Keep it in `.env` (already git-ignored) and never in the repository, a Gist, or a shell history file.

> [!NOTE]
> **State Architecture & Local Isolation**:
> - Routine state (`routine_state.json`) is strictly local to a single host and must never be shared between different machines. Cloud syncing is strictly limited to public calendar assets (`routine.ics`, `routine_data.json`, `routine_meta.json`).
> - **Concurrent Access**: The background scraper service (`academic-routine-sync.service`) and the long-running bot daemon (`academic-routine.service`) may run simultaneously on the host. Mutations use a reload-before-write pattern via `StateStore` to avoid clobbering concurrent state changes.
> - **Official Attendance Import**: a successful self-hosted scrape writes `output/attendance_data.json` and merges it into the admin chat's attendance state, replacing manually recorded counts for every course the portal returned. Other users' manual counts are untouched. If the attendance grid fails to render, the run logs a warning and still publishes the routine.
> - **Schema Protection**: Binaries older than the on-disk state schema version refuse to start rather than corrupting or resetting the file.
> - **Admin Precedence**: Both the CLI and the daemon prioritize `ADMIN_CHAT_ID` over the fallback `TELEGRAM_CHAT_ID`.

### Verification
Execute the test suite:

```bash
cargo test --all-targets
uv run pytest -q
```

The scraper suite runs against sanitized live DOM captures in `scraper/tests/fixtures/` — a nested SSRS report with `rowspan` continuation rows, and the 5-column attendance grid with both status variants — so parser regressions surface without launching a browser.

Check code formatting and lints:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
uv run ruff check scraper
```

CI (`.github/workflows/ci.yml`) runs these same commands plus both test suites on every push and pull request to `main`.

---

## Production & Self-Hosting Deployment

The project is structured for easy self-hosting across local machines, Docker containers, Linux servers, or cloud PaaS providers.

### Option 1: Docker Container

A multi-stage Dockerfile bundles the optimized Rust binary, headless Chromium, virtual display (Xvfb), and Python dependencies:

```bash
# Build Docker image
docker build -t academic-routine .

# Run container with environment file
docker run -d \
  --name routine-bot \
  -p 8000:8000 \
  --env-file .env \
  academic-routine
```

### Option 2: Linux Systemd User Daemon & Timer

Pre-configured user-level service and timer units are provided in `deployment/`:

- `academic-routine.service`: Runs the Telegram bot daemon and 6:00 AM briefing scheduler with automatic restart.
- `academic-routine-sync.timer`: Automatically executes weekly routine scraping every Saturday at 8:00 PM.

The units hardcode three paths, so set them up before enabling the units (see [Prerequisites](#prerequisites) for the toolchain):

```bash
# Clone to the directory the units use as WorkingDirectory
git clone https://github.com/zaheen4/academic-routine.git ~/academic-routine
cd ~/academic-routine

# Binary the units execute: %h/.cargo/bin/academic-routine
cargo install --path . --locked

# Python dependencies for the weekly sync unit (.venv/bin/python is preferred)
uv sync

# Credentials read via EnvironmentFile=-%h/academic-routine/.env
cp .env.example .env
```

```bash
# Copy systemd units to user services directory
mkdir -p ~/.config/systemd/user
cp deployment/academic-routine*.service ~/.config/systemd/user/
cp deployment/academic-routine-sync.timer ~/.config/systemd/user/

# Reload user daemon and enable services
systemctl --user daemon-reload
systemctl --user enable --now academic-routine.service
systemctl --user enable --now academic-routine-sync.timer

# Enable linger so user services run persistently across reboots
loginctl enable-linger "$USER"
```

### Option 3: Cloud PaaS (Render, Railway, Fly.io)

The bot includes an embedded HTTP health check server listening on `PORT` (`GET /health` and `GET /`) to satisfy zero-downtime platform requirements.

---

## License

This project is licensed under the [MIT License](LICENSE).
