# Architecture Decision Records

This document logs architectural and operational choices that govern the codebase. Consult these entries before proposing changes to established behaviors.

### 2026-10-20 — Section filtering aligns theory and lab subdivisions

**Decision.** The routine system defaults to Section `B` for general theory lectures while allowing individual students to subscribe to lab sub-sections (`B1` or `B2`) via `/section`.
**Why.** Academic routine schedules group all students into cohort `B` for lectures, but split them into sub-cohorts for practical laboratory sessions.
**Rules out.** Hard-coding single-section schedules or generating completely separate calendars per cohort.
**Revisit if.** The university ceases dividing cohorts into lab sub-sections.

### 2026-10-22 — Admin ID takes precedence over general chat ID

**Decision.** The CLI and daemon prioritize `ADMIN_CHAT_ID` over `TELEGRAM_CHAT_ID` when targeting administrative updates.
**Why.** Multi-user operation requires distinguishing the primary host administrator from broadcast channels or non-administrative subscribers.
**Rules out.** Hardcoding a single chat recipient or relying on runtime guesswork.
**Revisit if.** Multi-tenant administrative RBAC is implemented.

### 2026-10-25 — State schema v2 with downgrade refusal and corruption quarantine

**Decision.** `RoutineState` enforces `schema_version = 2`. Binaries refuse execution if the file schema version is newer than supported, and unparseable state files are quarantined to `.corrupt-<timestamp>` rather than overwritten.
**Why.** Prevents silent state corruption when rolling back binaries or recovering from unclean host shutdowns.
**Rules out.** In-place truncation or silent resets to default empty state.
**Revisit if.** An external database engine replaces JSON flat-file storage.

### 2026-10-25 — Gist carries only public calendar assets; state never leaves the host

**Decision.** Cloud synchronization is strictly restricted to `routine.ics`, `routine_data.json`, and `routine_meta.json`. `routine_state.json` is never uploaded to GitHub.
**Why.** State contains chat IDs, user preferences, and private notification markers that must remain private to the host instance.
**Rules out.** Syncing entire state bundles across machines via GitHub Gist.
**Revisit if.** End-to-end encrypted distributed state storage is designed.

### 2026-10-26 — Release convention uses annotated tags and GitHub Releases without binary assets

**Decision.** Releases use annotated git tags named after the release title (`v0.3.X — Title`) and publish notes via GitHub Releases without attaching compiled binary assets or tracking `CHANGELOG.md`.
**Why.** Keeps the repository lean and minimal for a self-hosted single-maintainer project built from source via Cargo and Python uv.
**Rules out.** Bloating git history with binary release artifacts or duplicating notes across changelogs.
**Superseded by.** 2026-10-30 — the annotated tag carries the notes; the GitHub Releases tab is retired.
**Revisit if.** Pre-compiled distribution packages (e.g., Debian packages or Homebrew formulae) are distributed.

### 2026-10-29 — Attendance extraction relies exclusively on the course grid

**Decision.** Automated attendance data is parsed solely from the postback table `#ctl00_MainContainer_gvCourseList`. Dashboard progress cards are used only for offline validation.
**Why.** The course grid provides complete section associations and all 9 enrolled courses in a single navigation step. Visiting dashboard cards doubles session duration without providing extra data.
**Rules out.** Multi-page navigation scraping loops per account.
**Revisit if.** The portal removes percentages or classes from the table view.

### 2026-10-29 — Portal attendance is authoritative for the admin chat

**Decision.** Scraping automatically updates attendance counts in the admin's chat for every course returned by the portal, overwriting prior manual increments. Other users' records are untouched.
**Why.** Manual session counting frequently drifts from official registrar counts. The primary user requires synchronization with official records.
**Rules out.** Requiring manual confirmation prompts on every scheduled scrape.
**Revisit if.** Portal attendance reporting latency exceeds several weeks.

### 2026-10-29 — Teacher phones normalized to 11-digit local format

**Decision.** Phone numbers matched with international country prefixes (`+88` or `88`) are stripped down to the canonical 11-digit local format (`01XXXXXXXXX`).
**Why.** Downstream presentation cards, dialer links, and Telegram messages require consistent number formatting.
**Rules out.** Storing mixed international and local phone strings in JSON schemas.
**Revisit if.** International instructors with non-Bangladesh phone numbers are listed.

### 2026-10-29 — Weekly sync unit maintains `--force` flag

**Decision.** The systemd timer invokes `academic-routine sync-ucam --force` by default, bypassing row-count downgrade checks during scheduled runs.
**Why.** Automated execution cannot prompt interactive confirmation if course schedules are legitimately reduced. Pre-release verification and dry-runs serve as the safety gate.
**Rules out.** Stalling automated weekly syncs when terms conclude and class counts decrease.
**Revisit if.** Interactive headless approval queues are implemented.

### 2026-10-29 — One-time room and teacher change notifications are accepted

**Decision.** When parser corrections restore valid room numbers (e.g., from `Room B` to `Room 120`), the resulting diff notifications sent to subscribers are accepted rather than suppressed.
**Why.** Subscribers require clear notice that calendar locations have changed to real rooms.
**Rules out.** Silently suppressing diff events during major scraper migrations.
**Revisit if.** Routine changes affect hundreds of users simultaneously with noisy spam.

### 2026-10-29 — Tests utilize sanitized live DOM captures

**Decision.** Scraper unit tests parse sanitized live HTML captures (`scraper/tests/fixtures/`) rather than hand-crafted synthetic HTML snippets.
**Why.** Synthetic HTML failed to replicate real SSRS nested table layouts and `rowspan` continuation shifts that broke production parsing.
**Rules out.** Writing toy HTML fixtures for complex WebForms reports.
**Revisit if.** The portal migrates away from SSRS to clean REST/JSON APIs.

### 2026-10-29 — Scraper fixes need no rebuild; schema changes do need a restart

**Decision.** Python-only fixes deploy on the next scrape. Any change to `src/`, `Cargo.*` or `STATE_SCHEMA_VERSION` requires `cargo install --path . --locked` and a daemon restart.
**Why.** The daemon and timer invoke the script from `WorkingDirectory`; the binary is compiled.
**Rules out.** Restarting services for every parser fix, or shipping a binary change without a restart.
**Revisit if.** The scraper is rewritten in Rust.

### *(Open)* 2026-10-29 — iCalendar recurrence has no UNTIL boundary

**Decision.** Published `RRULE` lines carry no `UNTIL` until a semester end date has a real source.
**Why.** The portal publishes no structured end date. The semester label cannot be trusted for this: "2024-2025 Level 4 Term 1 January-July 2026" is the active term while the first event starts in November 2026, so deriving a date from the label would truncate valid classes.
**What already exists.** `academic-routine export --until YYYY-MM-DD` applies an `UNTIL` boundary (`src/cli.rs:139-141`); the weekly unit simply does not pass it.
**Open question.** Add `--until` to the sync unit, or add a `semester_end_date` source to state.
**Revisit if.** The portal publishes an academic calendar, or a term end date is recorded in state.

### 2026-10-30 — Release notes live in the annotated tag; GitHub Releases are retired

**Decision.** A release is an annotated tag whose message is the complete release note: a subject line carrying the impact, an `Impact:` line naming what has to be rebuilt or restarted, and change bullets. Nothing is published to the GitHub Releases tab, and no `CHANGELOG.md` exists. Tags created before this date (`v0.2.0`–`v0.3.2`) were migrated once: their GitHub Release bodies were copied verbatim into the tag annotation, the tags re-created with `--cleanup=whitespace`, force-pushed, and the releases deleted.
**Why.** The tag message and the release title were byte-identical, every release body restated the upgrade matrix that belongs in `operations.md` §10, and the Releases page was a projection of facts already stored elsewhere. A tag is immutable, readable offline with `git show`, and survives a move off GitHub; a checked-in changelog would be a mutable document describing immutable events.
**Rules out.** `gh release create`, `--notes-file` as a second artifact, a maintained `CHANGELOG.md`, and per-release "Upgrading" sections. It also rules out moving tags: the migration above is the only re-annotation to date.
**Revisit if.** Releases gain downloadable assets, a second maintainer, or automated publishing.

*Last reviewed against v0.3.2 (2026-10-29)*
