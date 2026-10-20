#!/usr/bin/env python3
"""
Standalone UCAM Routine Extraction Engine.
Extracts student timetable directly from the official UCAM Student Class Routine Report.

Features:
- Multi-Account Support (Account 1, Account 2, ... Account N).
- Direct extraction of teacher phone numbers and emails (no external manual JSON required).
- Resilient Cloudflare bypass using Chrome DevTools Protocol (CDP) via nodriver.
- Normalized 12-hour time intervals and room designations.
- Atomic JSON write contract.
"""

import argparse
import asyncio
from datetime import datetime, timezone
import json
import logging
import os
from pathlib import Path
import re
import shutil
import sys

from bs4 import BeautifulSoup
from dotenv import load_dotenv
import nodriver as nd

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("ucam_scraper")

LOGIN_URL = "https://ucam.niter.edu.bd/Security/LogIn.aspx"
ROUTINE_REPORT_URL = (
    "https://ucam.niter.edu.bd/Module/schedular/Report/RptStudentClassRoutine.aspx?mmi=40545e1e42555f594e63"
)

RE_COURSE_CODE = re.compile(r"\b([A-Z]{2,4}-\d{4})\b")
RE_TIME_SLOT = re.compile(
    r"(\d{1,2}:\d{1,2}\s*(?:AM|PM)?\s*-\s*\d{1,2}:\d{1,2}\s*(?:AM|PM)?)",
    re.IGNORECASE,
)
RE_PHONE = re.compile(r"\b(01\d{9})\b")
RE_EMAIL = re.compile(r"\b([\w\.-]+@[\w\.-]+\.\w+)\b")
RE_ROOM = re.compile(r"\b(\d{3}|AC-\d{3}|Lab[-\s]?\d?)\b", re.IGNORECASE)
RE_SECTION = re.compile(r"\b(B|B1|B2|A|A1|A2|C|C1|C2)\b")
DAYS_OF_WEEK = ("Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri")


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
                logger.info("Loaded %d accounts from UCAM_ACCOUNTS env var.", len(accounts))
                return accounts
        except json.JSONDecodeError as e:
            logger.warning("Failed to parse UCAM_ACCOUNTS env var as JSON: %s", e)

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

    # 3. Fallback to local configs (e.g. routine-to-gsheet or configs/ucam_accounts.json)
    fallback_paths = [
        Path("configs/ucam_accounts.json"),
        Path("/home/eyepatch/Documents/projects/routine-to-gsheet/configs_to_edit/ucam_login_credentials.json"),
    ]
    for p in fallback_paths:
        if p.exists():
            try:
                with open(p, encoding="utf-8") as f:
                    data = json.load(f)
                    users = data.get("users", [])
                    if users:
                        logger.info("Loaded %d accounts from fallback file '%s'.", len(users), p)
                        return users
            except Exception as e:
                logger.warning("Error reading config '%s': %s", p, e)

    raise ValueError(
        "No UCAM accounts configured! Set UCAM_ACCOUNTS or UCAM_USERNAME/UCAM_PASSWORD in .env."
    )


def parse_report_html(html: str, default_section: str = "B") -> list[dict]:
    """Parses class routine entries from the UCAM SSRS ReportViewer HTML table."""
    soup = BeautifulSoup(html, "html.parser")
    entries = []

    current_course_code = None
    current_title = None

    rows = soup.find_all("tr")
    for row in rows:
        tds = [td.get_text(separator=" ", strip=True) for td in row.find_all("td")]
        if not tds:
            continue

        row_str = " | ".join(tds)

        # Detect new course row
        code_match = RE_COURSE_CODE.search(row_str)
        if code_match and "Course" not in row_str:
            current_course_code = code_match.group(1)
            for idx, text in enumerate(tds):
                if text == current_course_code and idx + 1 < len(tds):
                    current_title = tds[idx + 1]
                    break

        # Check for day in row
        day_found = next((d for d in DAYS_OF_WEEK if d in tds), None)
        if day_found and current_course_code:
            time_match = RE_TIME_SLOT.search(row_str)
            time_slot = time_match.group(1).strip() if time_match else ""

            room_match = RE_ROOM.search(row_str)
            room = room_match.group(1).strip() if room_match else ""

            sec_match = RE_SECTION.search(row_str)
            section = sec_match.group(1).strip() if sec_match else default_section

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

    return entries


async def extract_account_routine(
    account: dict, chrome_bin: str, headed: bool = False
) -> tuple[str, list[dict]]:
    """Logs into UCAM, bypasses Cloudflare, and extracts routine report for an account."""
    uname = account["username"]
    pwd = account["password"]
    acc_id = account.get("id", uname)
    sec_label = account.get("section_label", "B")

    browser_args = [
        "--ozone-platform=x11",
        "--window-size=1920,1080",
    ]
    if not headed:
        browser_args.append("--no-sandbox")

    logger.info(
        "Starting browser session for %s (%s) [headed=%s]...",
        acc_id,
        uname,
        headed,
    )
    browser = await nd.start(
        headless=False,
        browser_executable_path=chrome_bin,
        browser_args=browser_args,
    )
    tab = browser.main_tab if hasattr(browser, "main_tab") else browser

    try:
        # Context-building initial visit
        await tab.get("https://www.google.com")
        await asyncio.sleep(2)

        # Navigate to login
        logger.info("[%s] Accessing UCAM login...", acc_id)
        await tab.get(LOGIN_URL)
        await asyncio.sleep(6)

        user_input = None
        for attempt in range(1, 12):
            title = await tab.evaluate("document.title") or ""
            if any(w in title.lower() for w in ("just a moment", "cloudflare", "attention required")):
                logger.info("[%s] Cloudflare challenge active (attempt %d). Waiting...", acc_id, attempt)
                await asyncio.sleep(4)
                continue
            user_input = await tab.select("#logMain_UserName", timeout=6)
            if user_input:
                break
            await asyncio.sleep(2)

        if not user_input:
            raise TimeoutError(f"Could not load login fields for {acc_id} (Cloudflare block).")

        pass_input = await tab.select("#logMain_Password", timeout=5)
        login_btn = await tab.select("#logMain_Button1", timeout=5)

        logger.info("[%s] Authenticating student credentials...", acc_id)
        await user_input.send_keys(uname)
        await pass_input.send_keys(pwd)
        await login_btn.click()
        await asyncio.sleep(8)

        # Navigate to Class Routine Report
        logger.info("[%s] Loading Student Class Routine Report...", acc_id)
        await tab.get(ROUTINE_REPORT_URL)
        await asyncio.sleep(8)

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
        await asyncio.sleep(10)

        # Retrieve table HTML
        report_html = await tab.evaluate("""
            (() => {
                const reportTable = document.getElementById("ctl00_MainContainer_ReportViewer1_fixedTable");
                return reportTable ? reportTable.outerHTML : document.body.innerHTML;
            })()
        """)

        entries = parse_report_html(report_html, default_section=sec_label)
        logger.info("[%s] Successfully extracted %d class entries.", acc_id, len(entries))
        return semester_name or "Active Semester", entries

    finally:
        browser.stop()


async def run_multi_account_extraction(output_path: Path, headed: bool = False):
    """Executes routine extraction across all configured accounts and emits JSON."""
    if not headed and "WAYLAND_DISPLAY" in os.environ:
        os.environ.pop("WAYLAND_DISPLAY", None)

    chrome_bin = find_chrome_executable()
    accounts = load_accounts()

    all_entries = []
    processed_accounts = []
    latest_semester = "Level 4 Term 1"

    for acc in accounts:
        try:
            sem, entries = await extract_account_routine(acc, chrome_bin, headed=headed)
            if sem:
                latest_semester = sem
            all_entries.extend(entries)
            processed_accounts.append(acc.get("id", acc["username"]))
        except Exception as e:
            logger.error("Failed to extract routine for account %s: %s", acc.get("id"), e)

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

    # Atomic write
    tmp_path = output_path.with_suffix(".tmp")
    with open(tmp_path, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp_path, output_path)

    logger.info(
        "🎉 Extraction complete! Wrote %d unique entries across %d accounts to '%s'.",
        len(unique_entries),
        len(processed_accounts),
        output_path,
    )


def main():
    parser = argparse.ArgumentParser(description="Standalone UCAM Routine Scraper")
    parser.add_argument(
        "--output",
        "-o",
        type=Path,
        default=Path("routine_data.json"),
        help="Target output JSON path (default: routine_data.json)",
    )
    parser.add_argument(
        "--headed",
        action="store_true",
        help="Run browser visibly for debugging (default: false / headless offscreen virtual display)",
    )
    args = parser.parse_args()

    try:
        asyncio.run(run_multi_account_extraction(args.output, headed=args.headed))
    except Exception as e:
        logger.error("Scraper terminated with error: %s", e)
        sys.exit(1)


if __name__ == "__main__":
    main()
