# AGENTS.md

Two-language repo: a Rust core (`src/`, lib `academic_routine` + bin `academic-routine`) and a
Python scraper (`scraper/`) that the Rust binary shells out to. `output/`, `configs/`, `.env`, and
`google_cloud_keys/` are gitignored and are the only runtime inputs worth knowing about.

## Commands

CI is the source of truth (`.github/workflows/ci.yml`). Run all six before calling work done:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo test --doc
uv sync --frozen
uv run ruff check scraper && uv run pytest
```

- Single Rust test: `cargo test --test bot_commands_test test_free_breaks_calculation_logic` or
  `cargo test some_fn_name` for a unit test.
- Single Python test: `uv run pytest scraper/tests/test_extract_ucam.py -k attendance`.
- Pytest only collects `scraper/tests` (`testpaths` in `pyproject.toml`); `tests/` at the root is
  Rust integration tests.
- `cargo test --all-targets` does **not** run doc-tests — Cargo excludes them — which is why CI
  runs `cargo test --doc` as a separate step. Plain `cargo test` happens to run both, so it is not
  a substitute for the CI pair.
- CI skips pushes that only touch `**.md`, `docs/**`, `.gitignore`, `LICENSE`. A docs-only commit
  legitimately shows no CI run — that is not a failure.

## Gotchas

- `#![warn(missing_docs)]` plus `-D warnings` in Clippy means every new `pub` item needs a doc
  comment, including enum variants and struct fields. This is the most common CI break.
- `DayOfWeek` is ordered `Sat = 1 … Fri = 7` (university week), but the bot's teaching week is
  Sun–Thu; Fri/Sat are weekend and the scheduler sends no briefing on days with no entries.
- Use `academic_now()` — never `Local::now()` — for anything that decides the day, the time, or
  "is this class active". It honours `ROUTINE_TIMEZONE_OFFSET` (hours from UTC, default `6`).
  `Utc::now()` is correct only for absolute timestamps (`updated_at`, ICS `DTSTAMP`).
- The scraper is spawned with `.venv/bin/python` **resolved relative to the process CWD**, not the
  repo root or the executable. Running the binary from another directory silently falls back to
  `python3` and loses the venv's dependencies.
- Scraper `--force` only bypasses the "do not overwrite newer data with older data" guard. A
  refusal exits non-zero, which surfaces from the Rust side as an execution failure, not as a
  warning.
- `tests/ingestion_test.rs` returns early and passes when `output/routine_data.json` is absent, so
  it proves nothing on a clean checkout.
- Attendance is scraped and imported only on a `/sync`, and only into the admin chat (`ADMIN_CHAT_ID`).
  Every other chat has no attendance record, and the briefing omits those lines entirely for it.
- `cargo install --path . --locked` puts the binary in `~/.cargo/bin`, which is not on `PATH` in
  this environment. Invoke it by full path or use `systemctl --user` (the unit already points at
  `%h/.cargo/bin/academic-routine`).
- Rebuilding is the upgrade; `systemctl --user restart` alone relaunches the old binary. Verify
  with `/proc/$(systemctl --user show academic-routine.service -p MainPID --value)/exe --version`.

## Docs and releases

- `docs/index.md` is a routing table: portal/DOM knowledge → `ucam-scraping.md`, commands and
  systemd → `operations.md`, schema and concurrency → `state-model.md`, rationale →
  `decisions.md`. One home per fact; cross-reference instead of duplicating.
- Each `docs/*.md` ends with `*Last reviewed against vX.Y.Z (date)*`. Bump it in the same commit
  that changes a documented contract, not on every release.
- Exactly one README, at the root. No `docs/README.md`, no `CHANGELOG.md`.
- A release is an annotated tag whose body **is** the release note — no GitHub Release, no binary
  assets. See `docs/operations.md` §9 for the template and `docs/decisions.md` for why.
- `git tag -a -F` defaults to `--cleanup=strip`, which deletes every line starting with `#` and
  therefore every markdown heading. Always pass `--cleanup=whitespace`.
- Never move, re-point, or delete an existing tag. The one-off 2026-10-30 migration of
  `v0.2.0`–`v0.3.2` is the only exception, and it is finished.
- Version lives in `Cargo.toml`, `pyproject.toml`, `Cargo.lock`, and `uv.lock`; `uv lock` syncs
  the last one.

## Deploy

`deployment/` holds the systemd user units (bot service + weekly sync timer, Sat 20:00 with
`Persistent=true`). Both units pin `WorkingDirectory=%h/academic-routine` and read `.env` from
there, so that path must keep resolving to this tree; the binary itself is installed separately
under `~/.cargo/bin`.

```bash
cargo install --path . --locked
systemctl --user restart academic-routine.service
curl -fsS http://127.0.0.1:8000/health   # "Bot is healthy!"
```
