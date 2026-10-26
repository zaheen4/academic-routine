#!/usr/bin/env python3
"""
Standalone UCAM Routine Extraction Engine.
Extracts student timetable directly from the official UCAM Student Class Routine Report.

Features:
- Multi-Account Support (Account 1, Account 2, ... Account N).
- Direct extraction of teacher phone numbers and emails (no external manual JSON required).
- Resilient Cloudflare bypass using Chrome DevTools Protocol (CDP) via nodriver.
- Explicit, bounded waits with retries instead of fixed multi-second sleeps.
- Header-driven report parsing with a positional fallback for headerless reports.
- Normalized 12-hour time intervals and room designations.
- Atomic JSON write contract with a row-count downgrade guard.
"""

import argparse
import asyncio
import importlib.metadata
import json
import logging
import os
import re
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path

from bs4 import BeautifulSoup
from dotenv import load_dotenv

try:
    import nodriver as nd
    import nodriver.core.browser as _nd_browser
except ImportError:  # pragma: no cover - only hit when browser deps are absent
    nd = None
    _nd_browser = None

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("ucam_scraper")

# The monkeypatch below is written against the nodriver 0.50.x Browser internals.
SUPPORTED_NODRIVER_MAJOR_MINOR = (0, 50)
SUPPORTED_NODRIVER_SPEC = ">=0.50.6,<0.51"

LOGIN_URL = "https://ucam.niter.edu.bd/Security/LogIn.aspx"
ROUTINE_REPORT_URL = (
    "https://ucam.niter.edu.bd/Module/schedular/Report/RptStudentClassRoutine.aspx?mmi=40545e1e42555f594e63"
)
ATTENDANCE_DASHBOARD_URL = (
    "https://ucam.niter.edu.bd/Module/Dashboard/StudentClassAttendanceDashboard.aspx?mmi=40545a1b42555b5c4e63"
)

RE_COURSE_CODE = re.compile(r"\b([A-Z]{2,4}-\d{4})\b")
RE_TIME_SLOT = re.compile(
    r"(\d{1,2}:\d{1,2}\s*(?:AM|PM)?\s*-\s*\d{1,2}:\d{1,2}\s*(?:AM|PM)?)",
    re.IGNORECASE,
)
RE_PHONE = re.compile(r"\b(01\d{9})\b")
RE_EMAIL = re.compile(r"\b([\w\.-]+@[\w\.-]+\.\w+)\b")
RE_ROOM = re.compile(r"\b(AC-\d{3}|\d{3}|CSE\s+Lab[-\s]?\d*|Lab[-\s]?\d+)\b", re.IGNORECASE)
RE_SECTION = re.compile(r"\b(B|B1|B2|A|A1|A2|C|C1|C2)\b")
DAYS_OF_WEEK = ("Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri")

# Header names (normalized to lower case, punctuation collapsed) mapped to result fields.
HEADER_ALIASES = {
    "contact": {
        "teacher phone and email",
        "teacher phone email",
        "teacher phone & email",
        "phone and email",
        "contact",
        "contact info",
        "contact information",
        "teacher contact",
    },
    "course": {"course", "course code", "code", "course no", "course number"},
    "title": {"course title", "title", "course name", "subject"},
    "day": {"day", "class day", "day of week"},
    "room": {"room", "room no", "room number", "class room", "venue"},
    "time": {"time", "time slot", "class time", "class time slot", "slot"},
    "section": {"section", "sect", "group", "batch"},
    "teacher": {"teacher", "teacher name", "faculty", "faculty name", "instructor"},
    "phone": {"phone", "mobile", "teacher phone"},
    "email": {"email", "e-mail", "teacher email"},
}

_SECRETS: set[str] = set()


def register_secret(value: str | None) -> None:
    """Registers a secret so log redaction can remove it from error output."""
    if isinstance(value, str) and len(value) >= 4:
        _SECRETS.add(value)


def redact(value: object) -> str:
    """Removes registered secrets and obvious credential assignments from text."""
    text = str(value)
    for secret in sorted(_SECRETS, key=len, reverse=True):
        text = text.replace(secret, "<redacted>")
    text = re.sub(
        r"(?i)\b(password|passwd|token|authorization|api[_-]?key)\b(\s*[:=]\s*)([^\s,;]+)",
        r"\1\2<redacted>",
        text,
    )
    return text


def find_chrome_executable() -> str:
    """Finds available Chrome or Chromium binary on the host system."""
    env_override = os.environ.get("CHROME_BINARY_PATH")
    if env_override and os.path.exists(env_override):
        return env_override

    candidates = [
        "/usr/bin/google-chrome-stable",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ]
    for c in candidates:
        if os.path.exists(c):
            return c

    # System PATH lookup
    for name in ("google-chrome-stable", "google-chrome", "chromium", "chromium-browser"):
        found = shutil.which(name)
        if found:
            return found

    raise FileNotFoundError("Could not find Google Chrome or Chromium executable on host system.")


def load_accounts() -> list[dict]:
    """Loads UCAM credentials from environment variables or local configs."""
    load_dotenv()

    # 1. Check UCAM_ACCOUNTS (JSON array of accounts)
    raw_accounts = os.environ.get("UCAM_ACCOUNTS")
    if raw_accounts:
        try:
            accounts = json.loads(raw_accounts)
            if isinstance(accounts, list) and accounts:
                for account in accounts:
                    register_secret(account.get("password"))
                logger.info("Loaded %d accounts from UCAM_ACCOUNTS env var.", len(accounts))
                return accounts
        except json.JSONDecodeError as e:
            logger.warning("Failed to parse UCAM_ACCOUNTS env var as JSON: %s", redact(e))

    # 2. Check individual UCAM_USER_* env vars
    env_accounts = []
    idx = 1
    while True:
        uname = os.environ.get(f"UCAM_USER_{idx}_USERNAME") or (
            os.environ.get("UCAM_USERNAME") if idx == 1 else None
        )
        pwd = os.environ.get(f"UCAM_USER_{idx}_PASSWORD") or (
            os.environ.get("UCAM_PASSWORD") if idx == 1 else None
        )
        sec = os.environ.get(f"UCAM_USER_{idx}_SECTION") or (
            os.environ.get("DEFAULT_SECTION", "B2") if idx == 1 else "B1"
        )
        if not (uname and pwd):
            break
        register_secret(pwd)
        env_accounts.append({
            "id": f"Account {idx}",
            "username": uname.strip(),
            "password": pwd.strip(),
            "section_label": sec.strip(),
        })
        idx += 1

    if env_accounts:
        logger.info("Loaded %d accounts from individual environment variables.", len(env_accounts))
        return env_accounts

    # 3. Fallback to local configs (canonical path first)
    fallback_paths = [
        Path("configs/ucam_accounts.json"),
        Path("ucam_login_credentials.json"),
    ]
    for p in fallback_paths:
        if p.exists():
            try:
                with open(p, encoding="utf-8") as f:
                    data = json.load(f)
                    users = data.get("users", [])
                    for user in users:
                        register_secret(user.get("password"))
                    if users:
                        logger.info("Loaded %d accounts from fallback file '%s'.", len(users), p)
                        return users
            except Exception as e:
                logger.warning("Error reading config '%s': %s", p, redact(e))

    raise ValueError(
        "No UCAM accounts configured! Set UCAM_ACCOUNTS or UCAM_USERNAME/UCAM_PASSWORD in .env."
    )


def _nodriver_version() -> tuple[int, int, str]:
    """Returns (major, minor, raw) for the installed nodriver distribution."""
    try:
        raw = importlib.metadata.version("nodriver")
    except importlib.metadata.PackageNotFoundError as exc:
        raise RuntimeError(
            "nodriver is not installed. Install the pinned scraper dependencies with "
            f"'uv sync --locked' (requires nodriver{SUPPORTED_NODRIVER_SPEC})."
        ) from exc

    match = re.match(r"(\d+)\.(\d+)", raw)
    if not match:
        raise RuntimeError(f"Could not parse installed nodriver version: {raw!r}")
    return int(match.group(1)), int(match.group(2)), raw


def assert_supported_nodriver() -> None:
    """Fails fast when the installed nodriver internals are outside the tested line."""
    major, minor, raw = _nodriver_version()
    if (major, minor) != SUPPORTED_NODRIVER_MAJOR_MINOR:
        raise RuntimeError(
            f"Unsupported nodriver version {raw}. This scraper patches nodriver 0.50.x "
            f"Browser internals and requires nodriver{SUPPORTED_NODRIVER_SPEC}. "
            "Pin the dependency or re-validate the patch before upgrading."
        )


# Resilient browser launcher for containerized environments (Render / Docker).
# Increases the DevTools CDP connection window from 2.5s (5 retries) to 15s (30 retries).
# The patch targets nodriver 0.50.x internals only; `install_resilient_browser_start()`
# verifies the installed version before applying it.
async def _resilient_browser_start(self=None):
    if not self:
        raise RuntimeError("use ``await Browser.create()`` to create a new instance")

    if self._process or self._process_pid:
        if self._process.returncode is not None:
            return await self.create(config=self.config)
        raise RuntimeError("ignored! this call has no effect when already running.")

    connect_existing = False
    if self.config.host is not None and self.config.port is not None:
        connect_existing = True
    else:
        self.config.host = "127.0.0.1"
        self.config.port = _nd_browser.util.free_port()

    if getattr(self.config, "_extensions", None):
        self.config.add_argument(
            f"--load-extension={','.join(str(_) for _ in self.config._extensions)}"
        )

    exe = self.config.browser_executable_path
    params = self.config()

    if not connect_existing:
        self._process = await asyncio.create_subprocess_exec(
            exe,
            *params,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            close_fds=_nd_browser.is_posix,
        )
        self._process_pid = self._process.pid

    self._http = _nd_browser.HTTPApi((self.config.host, self.config.port))
    _nd_browser.util.get_registered_instances().add(self)
    await asyncio.sleep(0.5)

    for attempt in range(1, 31):
        try:
            self.info = _nd_browser.ContraDict(await self._http.get("version"), silent=True)
            if self.info:
                break
        except Exception:
            if self._process and self._process.returncode is not None:
                stderr = await self._process.stderr.read()
                raise RuntimeError(
                    f"Browser process exited with code {self._process.returncode}: "
                    f"{stderr.decode(errors='ignore')}"
                )
            await asyncio.sleep(0.5)

    if not self.info:
        raise TimeoutError("Failed to connect to browser after 15 seconds.")

    self.websocket_url = self.info.webSocketDebuggerUrl
    await self.attach()
    await self.update_targets()
    return self


_PATCH_INSTALLED = False


def install_resilient_browser_start() -> None:
    """Validates nodriver and installs the isolated Browser.start monkeypatch once."""
    global _PATCH_INSTALLED

    if _nd_browser is None:  # pragma: no cover - browser dependency missing
        raise RuntimeError("nodriver is not installed; install the pinned scraper dependencies.")
    assert_supported_nodriver()
    if _PATCH_INSTALLED:
        return

    _nd_browser.Browser.start = _resilient_browser_start
    _PATCH_INSTALLED = True
    logger.info("Installed resilient nodriver launcher (nodriver %s).", _nodriver_version()[2])


async def wait_for_js(
    tab,
    expression: str,
    *,
    description: str,
    timeout: float = 20.0,
    acc_id: str = "",
    poll: float = 0.5,
) -> None:
    """Polls a JS expression until truthy, with exponential backoff and a hard deadline."""
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    attempt = 0
    last_error = None

    while True:
        attempt += 1
        try:
            if await tab.evaluate(expression):
                return
        except Exception as exc:  # navigation can race the evaluation
            last_error = exc

        remaining = deadline - loop.time()
        if remaining <= 0:
            message = f"Timed out after {timeout:.0f}s waiting for {description}"
            if last_error is not None:
                message += f" (last error: {redact(last_error)})"
            logger.error("[%s] %s", acc_id, message)
            raise TimeoutError(message)

        await asyncio.sleep(min(0.5 * (1.5 ** (attempt - 1)), 3.0, remaining))


async def wait_for_selector(tab, selector: str, *, timeout: float = 20.0, acc_id: str = ""):
    """Polls a CSS selector via tab.select until present, logging the selector on timeout."""
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    attempt = 0
    last_error = None

    while True:
        attempt += 1
        remaining = max(deadline - loop.time(), 0.0)
        try:
            element = await tab.select(selector, timeout=min(2.0, max(remaining, 0.5)))
            if element:
                return element
        except Exception as exc:
            last_error = exc

        remaining = deadline - loop.time()
        if remaining <= 0:
            message = f"Timed out waiting for selector {selector!r}"
            if last_error is not None:
                message += f" (last error: {redact(last_error)})"
            logger.error("[%s] %s", acc_id, message)
            raise TimeoutError(message)

        await asyncio.sleep(min(0.5 * (1.5 ** (attempt - 1)), 3.0, remaining))


async def wait_for_login_form(tab, *, acc_id: str, timeout: float = 90.0):
    """Waits through a Cloudflare interstitial until the login form is usable."""
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    attempt = 0

    while True:
        attempt += 1
        try:
            title = await tab.evaluate("document.title") or ""
            if any(w in title.lower() for w in ("just a moment", "cloudflare", "attention required")):
                logger.info("[%s] Cloudflare challenge active (attempt %d). Waiting...", acc_id, attempt)
            else:
                element = await tab.select("#logMain_UserName", timeout=1)
                if element:
                    return element
        except Exception as exc:
            logger.debug("[%s] Login form poll error: %s", acc_id, redact(exc))

        remaining = deadline - loop.time()
        if remaining <= 0:
            logger.error(
                "[%s] Timed out waiting for selector '#logMain_UserName' "
                "(Cloudflare challenge or slow login page).",
                acc_id,
            )
            raise TimeoutError(f"Could not load login fields for {acc_id} (Cloudflare block).")

        await asyncio.sleep(min(0.5 * (1.5 ** (attempt - 1)), 4.0, remaining))


def _normalize_header(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", " ", (text or "").lower()).strip()


def _header_key(text: str) -> str | None:
    token = _normalize_header(text)
    if not token:
        return None
    for key, aliases in HEADER_ALIASES.items():
        if token in aliases:
            return key
    return None


def _header_map(tds: list[str]) -> dict[str, int]:
    """Maps recognized header labels to column indices (first occurrence wins)."""
    mapping: dict[str, int] = {}
    for idx, text in enumerate(tds):
        key = _header_key(text)
        if key and key not in mapping:
            mapping[key] = idx
    return mapping


def _cell(tds: list[str], mapping: dict[str, int], key: str) -> str:
    idx = mapping.get(key)
    if idx is None or idx >= len(tds):
        return ""
    return tds[idx].strip()


def _is_header_row(mapping: dict[str, int]) -> bool:
    return len(mapping) >= 3 and any(k in mapping for k in ("day", "time", "course"))


def _parse_headerless_row(
    tds: list[str],
    row_str: str,
    current_course_code: str | None,
    current_title: str | None,
    default_section: str,
) -> dict | None:
    """Legacy positional parser, used only when the report has no recognizable header."""
    day_found = next((d for d in DAYS_OF_WEEK if d in tds), None)
    if not (day_found and current_course_code):
        return None

    day_idx = tds.index(day_found)

    # Room is strictly the column immediately following Day in the SSRS table
    room = tds[day_idx + 1].strip() if day_idx + 1 < len(tds) else ""
    if not room:
        room_match = RE_ROOM.search(row_str)
        room = room_match.group(1).strip() if room_match else ""

    # Time slot is strictly the column after Room
    time_slot = tds[day_idx + 2].strip() if day_idx + 2 < len(tds) else ""
    if not time_slot:
        time_match = RE_TIME_SLOT.search(row_str)
        time_slot = time_match.group(1).strip() if time_match else ""

    # Section is strictly the column after Time Slot
    section = tds[day_idx + 3].strip() if day_idx + 3 < len(tds) else ""
    if not section:
        sec_match = RE_SECTION.search(row_str)
        section = sec_match.group(1).strip() if sec_match else default_section

    teacher, teacher_phone, teacher_email = _extract_teacher(tds)

    return {
        "CourseCode": current_course_code,
        "CourseTitle": current_title or "",
        "Teacher": teacher,
        "TeacherPhone": teacher_phone,
        "TeacherEmail": teacher_email,
        "Day": day_found,
        "Room": room,
        "TimeSlot": time_slot,
        "Section": section,
    }


def _extract_teacher(tds: list[str]) -> tuple[str, str, str]:
    teacher = ""
    teacher_phone = ""
    teacher_email = ""
    for text in tds:
        if "_" in text and any(c.isupper() for c in text):
            teacher = text
        phone_match = RE_PHONE.search(text)
        if phone_match:
            teacher_phone = phone_match.group(1)
        email_match = RE_EMAIL.search(text)
        if email_match:
            teacher_email = email_match.group(1)
    return teacher, teacher_phone, teacher_email


def parse_report_html(html: str, default_section: str = "B") -> list[dict]:
    """Parses class routine entries from the UCAM SSRS ReportViewer HTML table.

    When a recognizable header row is present, columns are resolved by header name
    (with aliases). Headerless reports fall back to the historical positional layout:
    Day, Room (Day+1), Time (Day+2), Section (Day+3). The missing-column fallbacks
    inside a row are regex based and never assume a fixed offset.
    """
    soup = BeautifulSoup(html, "html.parser")
    entries = []

    current_course_code = None
    current_title = None
    current_teacher = ""
    current_teacher_phone = ""
    current_teacher_email = ""
    mapping: dict[str, int] = {}
    header_seen = False
    warned_missing: set[str] = set()

    rows = soup.find_all("tr")
    for row in rows:
        tds = [td.get_text(separator=" ", strip=True) for td in row.find_all(["td", "th"])]
        if not tds:
            continue

        row_str = " | ".join(tds)

        row_mapping = _header_map(tds)
        if _is_header_row(row_mapping):
            mapping = row_mapping
            header_seen = True
            continue

        if not header_seen:
            # A header may be absent entirely. Detect course rows first, then parse
            # only day-bearing rows with the legacy positional fallback.
            code_match = RE_COURSE_CODE.search(row_str)
            if code_match:
                current_course_code = code_match.group(1)
                for idx, text in enumerate(tds):
                    if text == current_course_code and idx + 1 < len(tds):
                        current_title = tds[idx + 1]
                        break

            # Cache teacher details from the course header row if present
            t_name = _cell(tds, mapping, "teacher")
            t_phone = _cell(tds, mapping, "phone") or _cell(tds, mapping, "contact")
            t_email = _cell(tds, mapping, "email") or _cell(tds, mapping, "contact")
            text_teacher, phone_match, email_match = _extract_teacher(tds)
            current_teacher = t_name or text_teacher or ""
            phone_found = RE_PHONE.search(t_phone) if t_phone else None
            current_teacher_phone = phone_found.group(1) if phone_found else (phone_match or "")
            email_found = RE_EMAIL.search(t_email) if t_email else None
            current_teacher_email = email_found.group(1) if email_found else (email_match or "")
            entry = _parse_headerless_row(
                tds, row_str, current_course_code, current_title, default_section
            )
            if entry:
                entries.append(entry)
            continue

        # Header-driven parsing.
        code_cell = _cell(tds, mapping, "course")
        code_match = RE_COURSE_CODE.search(code_cell) or RE_COURSE_CODE.search(row_str)
        if code_match:
            current_course_code = code_match.group(1)
            title_cell = _cell(tds, mapping, "title")
            if title_cell:
                current_title = title_cell
            elif "title" not in mapping and "title" not in warned_missing:
                warned_missing.add("title")
                logger.warning(
                    "Routine table header has no course-title column; "
                    "deriving titles from the cell after the course code."
                )
                for idx, text in enumerate(tds):
                    if text == current_course_code and idx + 1 < len(tds):
                        current_title = tds[idx + 1]
                        break

            # Cache teacher details from the course header row if present
            t_name = _cell(tds, mapping, "teacher")
            t_phone = _cell(tds, mapping, "phone") or _cell(tds, mapping, "contact")
            t_email = _cell(tds, mapping, "email") or _cell(tds, mapping, "contact")
            text_teacher, phone_match, email_match = _extract_teacher(tds)
            current_teacher = t_name or text_teacher or ""
            phone_found = RE_PHONE.search(t_phone) if t_phone else None
            current_teacher_phone = phone_found.group(1) if phone_found else (phone_match or "")
            email_found = RE_EMAIL.search(t_email) if t_email else None
            current_teacher_email = email_found.group(1) if email_found else (email_match or "")

        for key in ("day", "room", "time", "section"):
            if key not in mapping and key not in warned_missing:
                warned_missing.add(key)
                logger.warning(
                    "Routine table header has no %r column; using in-row detection for it.",
                    key,
                )

        day_found = _cell(tds, mapping, "day")
        if day_found not in DAYS_OF_WEEK:
            day_found = next((d for d in DAYS_OF_WEEK if d.lower() == day_found.lower()), None)
        if not day_found:
            day_found = next((d for d in DAYS_OF_WEEK if d in tds), None)

        if not (day_found and current_course_code):
            continue

        room = _cell(tds, mapping, "room")
        if not room:
            room_match = RE_ROOM.search(row_str)
            room = room_match.group(1).strip() if room_match else ""

        time_slot = _cell(tds, mapping, "time")
        if not time_slot:
            time_match = RE_TIME_SLOT.search(row_str)
            time_slot = time_match.group(1).strip() if time_match else ""

        section = _cell(tds, mapping, "section")
        if not section:
            sec_match = RE_SECTION.search(row_str)
            section = sec_match.group(1).strip() if sec_match else default_section

        teacher = _cell(tds, mapping, "teacher")
        teacher_phone = _cell(tds, mapping, "phone") or _cell(tds, mapping, "contact")
        teacher_email = _cell(tds, mapping, "email") or _cell(tds, mapping, "contact")

        # Fill any missing contact fields from the remaining cells.
        text_teacher, phone_match, email_match = _extract_teacher(tds)
        teacher = teacher or text_teacher
        if not RE_PHONE.search(teacher_phone):
            teacher_phone = _cell(tds, mapping, "contact")
        phone_found = RE_PHONE.search(teacher_phone) if teacher_phone else None
        teacher_phone = phone_found.group(1) if phone_found else phone_match
        if not RE_EMAIL.search(teacher_email):
            teacher_email = _cell(tds, mapping, "contact")
        email_found = RE_EMAIL.search(teacher_email) if teacher_email else None
        teacher_email = email_found.group(1) if email_found else email_match

        teacher = teacher or current_teacher
        teacher_phone = teacher_phone or current_teacher_phone
        teacher_email = teacher_email or current_teacher_email

        entries.append({
            "CourseCode": current_course_code,
            "CourseTitle": current_title or "",
            "Teacher": teacher,
            "TeacherPhone": teacher_phone,
            "TeacherEmail": teacher_email,
            "Day": day_found,
            "Room": room,
            "TimeSlot": time_slot,
            "Section": section,
        })

    if not header_seen and rows:
        logger.warning(
            "No recognizable routine table header found; using positional fallback "
            "(Day+1=Room, Day+2=Time, Day+3=Section)."
        )

    return entries


def _derive_attendance(
    total: int, pct: float, attended_source: int | None, missed_source: int | None
) -> tuple[int, int, bool]:
    """Returns (attended, missed, derived).

    Exact source counts always win. Only pct+total falls back to a derived value
    (round-half-to-even via Python's round) and is flagged as derived.
    """
    if attended_source is not None:
        attended = max(attended_source, 0)
        missed = missed_source if missed_source is not None else max(total - attended, 0)
        return attended, max(missed, 0), False
    if missed_source is not None:
        missed = max(missed_source, 0)
        attended = max(total - missed, 0)
        return attended, missed, False
    if total <= 0:
        return 0, 0, False
    attended = round(total * pct / 100.0)
    attended = min(max(attended, 0), total)
    return attended, total - attended, True


def parse_attendance_html(html: str) -> list[dict]:
    """Parses official UCAM attendance table into structured records.

    Exact counts from the portal are preserved. When only a percentage and total
    are available, the derived counts carry ``derived: true``.
    """
    soup = BeautifulSoup(html, "html.parser")
    table = soup.find("table", id=lambda x: x and "gvCourseList" in x)
    if not table:
        return []

    records = []
    rows = table.find_all("tr")
    for r in rows:
        tds = [td.get_text(separator=" ", strip=True) for td in r.find_all(["td", "th"])]
        if len(tds) < 4:
            continue
        row_str = " ".join(tds)
        code_match = RE_COURSE_CODE.search(row_str)
        if not code_match:
            continue
        course_code = code_match.group(1).upper()

        total_m = re.search(r"Total\s*Class(?:es)?\s*[:=]\s*(\d+)", row_str, re.IGNORECASE)
        pct_m = re.search(r"Attendance\s*Percentage\s*[:=]\s*([\d.]+)", row_str, re.IGNORECASE)
        status_m = re.search(r"Attendance\s*Status\s*[:=]\s*([A-Za-z-]+)", row_str, re.IGNORECASE)
        attended_m = re.search(
            r"(?:Attended|Present|Total\s*Present)\s*(?:Class(?:es)?)?\s*[:=]\s*(\d+)",
            row_str,
            re.IGNORECASE,
        )
        missed_m = re.search(
            r"(?:Missed|Absent|Total\s*Absent)\s*(?:Class(?:es)?)?\s*[:=]\s*(\d+)",
            row_str,
            re.IGNORECASE,
        )

        attended_source = int(attended_m.group(1)) if attended_m else None
        missed_source = int(missed_m.group(1)) if missed_m else None
        if total_m:
            total = int(total_m.group(1))
        elif attended_source is not None or missed_source is not None:
            total = (attended_source or 0) + (missed_source or 0)
        else:
            total = 0

        if pct_m:
            pct = float(pct_m.group(1))
        elif total > 0 and attended_source is not None:
            pct = round(attended_source * 100.0 / total, 1)
        else:
            pct = 0.0

        status = status_m.group(1) if status_m else "Collegiate"
        attended, missed, derived = _derive_attendance(
            total, pct, attended_source, missed_source
        )

        records.append({
            "course_code": course_code,
            "total_classes": total,
            "attended": attended,
            "missed": missed,
            "percentage": pct,
            "status": status,
            "derived": derived,
        })

    return records


def prefer_attendance(existing: dict, candidate: dict) -> dict:
    """Prefers exact source counts over derived values when merging duplicates."""
    if existing.get("derived") and not candidate.get("derived"):
        return candidate
    if not existing.get("derived") and candidate.get("derived"):
        return existing
    return candidate


async def extract_account_routine(
    account: dict, chrome_bin: str, headed: bool = False, fetch_attendance: bool = True
) -> tuple[str, list[dict], list[dict]]:
    """Logs into UCAM, bypasses Cloudflare, and extracts routine report for an account."""
    uname = account["username"]
    pwd = account["password"]
    acc_id = account.get("id", "account")
    sec_label = account.get("section_label", "B")

    browser_args = [
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--disable-software-rasterizer",
        "--ozone-platform=x11",
        "--window-size=1920,1080",
    ]
    if not headed:
        browser_args.append("--no-sandbox")

    logger.info("Starting browser session for %s [headed=%s]...", acc_id, headed)
    browser = await nd.start(
        headless=False,
        sandbox=False,
        browser_executable_path=chrome_bin,
        browser_args=browser_args,
    )
    tab = browser.main_tab if hasattr(browser, "main_tab") else browser

    try:
        # Context-building initial visit
        await tab.get("https://www.google.com")
        await wait_for_js(
            tab,
            'document.readyState === "complete"',
            description="initial document readiness",
            timeout=15,
            acc_id=acc_id,
        )

        # Navigate to login
        logger.info("[%s] Accessing UCAM login...", acc_id)
        await tab.get(LOGIN_URL)
        user_input = await wait_for_login_form(tab, acc_id=acc_id, timeout=90)

        pass_input = await wait_for_selector(
            tab, "#logMain_Password", timeout=10, acc_id=acc_id
        )
        login_btn = await wait_for_selector(
            tab, "#logMain_Button1", timeout=10, acc_id=acc_id
        )

        logger.info("[%s] Authenticating student credentials...", acc_id)
        await user_input.send_keys(uname)
        await pass_input.send_keys(pwd)
        await login_btn.click()

        # Wait for the post-login navigation instead of a fixed 8s sleep.
        await wait_for_js(
            tab,
            'document.readyState === "complete" && !location.href.toLowerCase().includes("login.aspx")',
            description="post-login navigation (URL leaves LogIn.aspx)",
            timeout=45,
            acc_id=acc_id,
        )

        # Navigate to Class Routine Report
        logger.info("[%s] Loading Student Class Routine Report...", acc_id)
        await tab.get(ROUTINE_REPORT_URL)
        await wait_for_js(
            tab,
            '!!document.getElementById("ctl00_MainContainer_ReportViewer1_fixedTable") '
            '|| !!document.getElementById("ctl00_MainContainer_ddlHeldIn")',
            description="routine report page (ddlHeldIn or fixedTable)",
            timeout=45,
            acc_id=acc_id,
        )

        # Select latest semester (first non-zero option in ddlHeldIn)
        semester_name = await tab.evaluate("""
            (() => {
                const sel = document.getElementById("ctl00_MainContainer_ddlHeldIn");
                if (!sel) return null;
                // Pick first active semester (value != "0")
                for (let i = 0; i < sel.options.length; i++) {
                    if (sel.options[i].value !== "0") {
                        sel.value = sel.options[i].value;
                        if (sel.onchange) { sel.onchange(); }
                        sel.dispatchEvent(new Event("change", { bubbles: true }));
                        return sel.options[i].text;
                    }
                }
                return null;
            })()
        """)

        logger.info("[%s] Synchronized semester: %s", acc_id, semester_name)

        # The report viewer re-renders the fixed table after the semester postback.
        # Its chrome (toolbar, pager) already contributes rows, so waiting on "tr"
        # alone returns before the report body exists and yields an empty parse.
        # Wait instead for a cell whose text is a weekday.
        try:
            await wait_for_js(
                tab,
                '(() => {'
                '  const t = document.getElementById("ctl00_MainContainer_ReportViewer1_fixedTable");'
                '  if (!t) return false;'
                '  return Array.from(t.querySelectorAll("td"))'
                '    .some(td => /^(sat|sun|mon|tue|wed|thu|fri)$/i.test((td.textContent || "").trim()));'
                '})()',
                description="routine table report rows (weekday cell)",
                timeout=45,
                acc_id=acc_id,
            )
        except TimeoutError:
            # A semester with no published routine legitimately renders no rows;
            # parsing then returns nothing and the caller decides what to do.
            logger.warning(
                "[%s] No weekday rows appeared in the routine report table.", acc_id
            )

        # Retrieve table HTML, re-reading if the postback is still settling.
        report_html = ""
        entries: list[dict] = []
        for attempt in range(1, 4):
            report_html = await tab.evaluate("""
                (() => {
                    const reportTable = document.getElementById("ctl00_MainContainer_ReportViewer1_fixedTable");
                    return reportTable ? reportTable.outerHTML : document.body.innerHTML;
                })()
            """)
            entries = parse_report_html(report_html, default_section=sec_label)
            if entries:
                break
            if attempt < 3:
                logger.warning(
                    "[%s] Report table had no parseable rows (attempt %d/3); re-reading.",
                    acc_id,
                    attempt,
                )
                await asyncio.sleep(3)

        logger.info("[%s] Successfully extracted %d class entries.", acc_id, len(entries))

        # Attempt to extract official attendance dashboard (primary account only)
        attendance_records = []
        if fetch_attendance:
            try:
                logger.info("[%s] Fetching Student Class Attendance Dashboard...", acc_id)
                await tab.get(ATTENDANCE_DASHBOARD_URL)
                await wait_for_js(
                    tab,
                    '(() => {'
                    '  const t = document.getElementById("ctl00_MainContainer_gvCourseList");'
                    '  return !!t || document.readyState === "complete";'
                    '})()',
                    description='attendance table ("#ctl00_MainContainer_gvCourseList")',
                    timeout=30,
                    acc_id=acc_id,
                )
                att_html = await tab.evaluate("""
                    (() => {
                        const t = document.getElementById("ctl00_MainContainer_gvCourseList");
                        return t ? t.outerHTML : "";
                    })()
                """)
                if att_html:
                    attendance_records = parse_attendance_html(att_html)
                    logger.info(
                        "[%s] Successfully extracted attendance for %d courses.",
                        acc_id,
                        len(attendance_records),
                    )
            except Exception as e:
                logger.warning(
                    "[%s] Attendance dashboard extraction notice: %s", acc_id, redact(e)
                )
        else:
            logger.info("[%s] Skipping attendance dashboard (secondary/donor account).", acc_id)

        return semester_name or "Active Semester", entries, attendance_records

    finally:
        browser.stop()


def _atomic_write_json(path: Path, payload: dict) -> None:
    if path.parent:
        path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = path.with_suffix(".tmp")
    with open(tmp_path, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp_path, path)


def existing_entry_count(path: Path) -> int | None:
    """Returns the entry count of an existing output file, or None if unreadable/absent."""
    if not path.exists():
        return None
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        entries = data.get("entries")
        if isinstance(entries, list):
            return len(entries)
    except (OSError, json.JSONDecodeError, AttributeError) as exc:
        logger.warning("Could not read existing output '%s': %s", path, redact(exc))
    return None


def downgrade_guard(path: Path, new_count: int, force: bool) -> tuple[bool, int | None]:
    """Decides whether a smaller extraction may replace the existing file.

    Returns ``(allowed, existing_count)``. A missing/unreadable existing file never blocks.
    """
    old_count = existing_entry_count(path)
    if old_count is not None and new_count < old_count and not force:
        logger.warning(
            "Refusing to downgrade '%s': new extraction has %d rows, existing file has %d. "
            "Re-run with --force to overwrite.",
            path,
            new_count,
            old_count,
        )
        return False, old_count
    if old_count is not None and new_count < old_count and force:
        logger.warning(
            "Forcing downgrade of '%s': new extraction has %d rows, existing file has %d.",
            path,
            new_count,
            old_count,
        )
    elif old_count is not None:
        logger.info(
            "Row-count guard passed for '%s': new=%d, existing=%d.",
            path,
            new_count,
            old_count,
        )
    return True, old_count


def run_parser_from_html(
    html_path: Path,
    output_path: Path,
    *,
    default_section: str = "B",
    dry_run: bool = False,
    force: bool = False,
) -> bool:
    """Parses a saved HTML report without launching a browser."""
    try:
        html = html_path.read_text(encoding="utf-8")
    except OSError as exc:
        logger.error("Could not read HTML file '%s': %s", html_path, redact(exc))
        return False

    entries = parse_report_html(html, default_section=default_section)
    logger.info("Parsed %d entries from '%s'.", len(entries), html_path)

    payload = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "semester": "Offline HTML fixture",
        "accounts_processed": ["html-file"],
        "entries": entries,
    }

    if dry_run:
        json.dump(payload, sys.stdout, indent=2, ensure_ascii=False)
        sys.stdout.write("\n")
        logger.info("Dry run: no output file written.")
        return True

    allowed, _ = downgrade_guard(output_path, len(entries), force)
    if not allowed:
        return False

    _atomic_write_json(output_path, payload)
    logger.info("Wrote %d entries to '%s'.", len(entries), output_path)
    return True


async def run_multi_account_extraction(
    output_path: Path, headed: bool = False, force: bool = False, dry_run: bool = False
) -> bool:
    """Executes routine extraction across all configured accounts and emits JSON."""
    if not headed and "WAYLAND_DISPLAY" in os.environ:
        os.environ.pop("WAYLAND_DISPLAY", None)

    chrome_bin = find_chrome_executable()
    accounts = load_accounts()
    install_resilient_browser_start()

    all_entries = []
    all_attendance = []
    processed_accounts = []
    latest_semester = "Level 4 Term 1"

    for idx, acc in enumerate(accounts):
        if idx > 0:
            await asyncio.sleep(1)  # brief settle pause between browser instances

        # Only the primary account (idx == 0) fetches attendance; secondary accounts are schedule donors
        is_primary = acc.get("primary", idx == 0)
        success = False
        for attempt in range(1, 3):
            try:
                sem, entries, att = await extract_account_routine(
                    acc, chrome_bin, headed=headed, fetch_attendance=is_primary
                )
                if sem:
                    latest_semester = sem
                all_entries.extend(entries)
                if is_primary and att:
                    all_attendance.extend(att)
                processed_accounts.append(acc.get("id", "account"))
                success = True
                break
            except Exception as e:
                logger.warning(
                    "Extraction attempt %d failed for account %s: %s",
                    attempt,
                    acc.get("id"),
                    redact(e),
                )
                if attempt < 2:
                    await asyncio.sleep(2)

        if not success:
            logger.error("Failed to extract routine for account %s after retries.", acc.get("id"))

    if not all_entries:
        raise RuntimeError("No routine entries were successfully extracted from any account.")

    # Deduplicate entries by (CourseCode, Day, TimeSlot, Section)
    unique_entries = []
    seen = set()
    for e in all_entries:
        key = (e["CourseCode"], e["Day"], e["TimeSlot"], e["Section"])
        if key not in seen:
            seen.add(key)
            unique_entries.append(e)

    payload = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "semester": latest_semester,
        "accounts_processed": processed_accounts,
        "entries": unique_entries,
    }

    if dry_run:
        logger.info(
            "Dry run: would write %d unique entries from %d/%d accounts to '%s'.",
            len(unique_entries),
            len(processed_accounts),
            len(accounts),
            output_path,
        )
        return True

    allowed, _ = downgrade_guard(output_path, len(unique_entries), force)
    if not allowed:
        return False

    _atomic_write_json(output_path, payload)
    logger.info(
        "🎉 Extraction complete! Wrote %d unique entries across %d accounts to '%s'.",
        len(unique_entries),
        len(processed_accounts),
        output_path,
    )

    if all_attendance:
        unique_att: dict[str, dict] = {}
        for a in all_attendance:
            code = a["course_code"]
            if code in unique_att:
                unique_att[code] = prefer_attendance(unique_att[code], a)
            else:
                unique_att[code] = a
        att_path = output_path.parent / "attendance_data.json"
        att_payload = {
            "updated_at": datetime.now(timezone.utc).isoformat(),
            "primary_account": accounts[0].get("username", "Unknown"),
            "records": list(unique_att.values()),
        }
        _atomic_write_json(att_path, att_payload)
        logger.info(
            "📊 Wrote %d official attendance records for primary account to '%s'.",
            len(unique_att),
            att_path,
        )

    return True


def main():
    parser = argparse.ArgumentParser(description="Standalone UCAM Routine Scraper")
    parser.add_argument(
        "--output",
        "-o",
        type=Path,
        default=Path("output/routine_data.json"),
        help="Target output JSON path (default: output/routine_data.json)",
    )
    parser.add_argument(
        "--headed",
        action="store_true",
        help="Run browser visibly for debugging (default: false / headless offscreen virtual display)",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Overwrite an existing output file even when the new extraction has fewer rows",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Parse/print results without writing files",
    )
    parser.add_argument(
        "--html-file",
        type=Path,
        default=None,
        help="Parse a saved report HTML file instead of launching a browser",
    )
    parser.add_argument(
        "--section",
        default="B",
        help="Fallback section label when the report omits a section column (default: B)",
    )
    args = parser.parse_args()

    try:
        if args.html_file:
            ok = run_parser_from_html(
                args.html_file,
                args.output,
                default_section=args.section,
                dry_run=args.dry_run,
                force=args.force,
            )
        else:
            ok = asyncio.run(
                run_multi_account_extraction(
                    args.output,
                    headed=args.headed,
                    force=args.force,
                    dry_run=args.dry_run,
                )
            )
    except Exception as e:
        logger.error("Scraper terminated with error: %s", redact(e))
        sys.exit(1)

    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
