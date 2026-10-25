#!/usr/bin/env python3
"""
Google Sheets Routine Synchronizer for academic-routine.

Synchronizes scraped academic routine data directly to Google Spreadsheet
(updating both 'backend' and 'NewMain' worksheets) using update-first,
clear-tail semantics so a failed sync never leaves the sheet blank.
"""

import argparse
import json
import logging
import os
import re
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

from dotenv import load_dotenv

try:
    import gspread
except ImportError:  # pragma: no cover - only hit when sheets deps are absent
    gspread = None

load_dotenv()

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("sync_gsheet")

# Layout Constants for Google Spreadsheet
SHEET_HEADERS = [
    "Course",
    "Course Title",
    "Sect",
    "Day",
    "Room",
    "Time Slot",
    "Teacher",
    "Teacher Phone and Email",
]
NEW_MAIN_SHEET_NAME = "NewMain"
BACKEND_SHEET_NAME = "backend"

DATA_START_ROW = 4
DATA_START_COL = 2  # Column B
DATA_WIDTH = len(SHEET_HEADERS)

DAY_ORDER = {"SAT": 1, "SUN": 2, "MON": 3, "TUE": 4, "WED": 5, "THU": 6, "FRI": 7}
MISSING_DAY = 998
UNKNOWN_DAY = 999
UNPARSEABLE_TIME = 99998

STAMP_COLUMN = DATA_START_COL + DATA_WIDTH - 1  # Column I
STAMP_PREFIX = "Last Updated: "
SIGNATURE = "Made by Z  :)"
NO_DATA_MESSAGE = "No data found."

REPORT_TIMEZONE = timezone(timedelta(hours=6))  # GMT+6 (Dhaka)

MONTHS = (
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
)

_TIME_RE = re.compile(r"^\s*(\d{1,2}):(\d{1,2})(?::\d{1,2})?(?:\s*(AM|PM))?", re.IGNORECASE)


def column_letter(col_idx: int) -> str:
    """1-based column index to its A1 letter(s): 1 -> 'A', 2 -> 'B'."""
    letters = ""
    while col_idx > 0:
        col_idx, remainder = divmod(col_idx - 1, 26)
        letters = chr(ord("A") + remainder) + letters
    return letters


def format_stamp(dt: datetime) -> str:
    """Formats datetime as 'Last Updated: Month DD, YYYY' in GMT+6."""
    local_dt = dt.astimezone(REPORT_TIMEZONE)
    month_name = MONTHS[local_dt.month - 1]
    return f"{STAMP_PREFIX}{month_name} {local_dt.day:02d}, {local_dt.year}"


def clean_teacher_name(name: str) -> str:
    """Strips leading initials prefix like 'UKD_' or 'MdAM_' if present."""
    if not name:
        return ""
    return re.sub(r"^[A-Za-z0-9]{2,6}_", "", name.strip())


def combine_contact_info(phone: str, email: str) -> str:
    """Combines teacher phone and email into a single multiline cell value."""
    p = phone.strip() if phone else ""
    e = email.strip() if email else ""
    if p and e:
        return f"{p}\n{e}"
    return p or e


def format_one_time(text: str) -> tuple[str, int]:
    """Parses a time token and returns (formatted 12h string, sortable minute integer)."""
    text = text.strip()
    match = _TIME_RE.match(text)
    if not match:
        return text, UNPARSEABLE_TIME

    hour = int(match.group(1))
    minute = int(match.group(2))
    period = match.group(3).upper() if match.group(3) else ("AM" if 7 <= hour <= 11 else "PM")

    if period == "PM" and hour < 12:
        hour += 12
    elif period == "AM" and hour == 12:
        hour = 0

    sortable = hour * 60 + minute
    display_hour = hour % 12 or 12
    return f"{display_hour}:{minute:02d} {period}", sortable


def parse_and_format_time_slot(slot: str) -> tuple[str, int]:
    """Formats full slot (e.g. '8:0 AM - 9:15 AM') and returns start minute."""
    if not slot or (isinstance(slot, str) and not slot.strip()):
        return "", 99999

    parts = re.split(r"\s*[-–—]\s*", str(slot).strip())
    start_fmt, start_sort = format_one_time(parts[0])
    end_text = parts[1] if len(parts) > 1 else ""
    if end_text:
        end_fmt, _ = format_one_time(end_text)
        formatted = f"{start_fmt} - {end_fmt}" if end_fmt else start_fmt
    else:
        formatted = start_fmt
    return formatted, start_sort


def normalize_day(day_str: str) -> int:
    """Maps a day string to university week sort index (1..7)."""
    if isinstance(day_str, str) and day_str.strip():
        return DAY_ORDER.get(day_str.strip()[:3].upper(), UNKNOWN_DAY)
    return MISSING_DAY


def build_sheet_data(entries: list[dict]) -> list[list]:
    """Builds standard 2D list of rows including the header."""
    rows = [list(SHEET_HEADERS)]
    for item in entries:
        rows.append([
            item.get("CourseCode", ""),
            item.get("CourseTitle", ""),
            item.get("Section", ""),
            item.get("Day", ""),
            item.get("Room", ""),
            item.get("TimeSlot", ""),
            clean_teacher_name(item.get("Teacher", "")),
            combine_contact_info(item.get("TeacherPhone", ""), item.get("TeacherEmail", "")),
        ])
    return rows


def build_newmain_rows(entries: list[dict]) -> list[list]:
    """Builds sorted, 12-hour normalized data rows for NewMain (without header)."""
    raw_data = build_sheet_data(entries)
    if not raw_data or len(raw_data) <= 1:
        return []

    decorated = []
    for row in raw_data[1:]:
        values = list(row)
        formatted_slot, sortable_time = parse_and_format_time_slot(values[5])
        values[5] = formatted_slot
        day_rank = normalize_day(values[3])
        decorated.append((day_rank, sortable_time, values))

    # Sort stably by (day_rank, sortable_time)
    decorated.sort(key=lambda t: (t[0], t[1]))
    return [t[2] for t in decorated]


def _used_last_row(ws, *, min_row: int = 1, min_col: int = 1, max_col: int | None = None) -> int:
    """Returns the last row containing data in the selected column window."""
    values = ws.get_all_values()
    last = 0
    for row_idx, row in enumerate(values, start=1):
        if row_idx < min_row:
            continue
        end = min(len(row), max_col) if max_col else len(row)
        for col_idx in range(min_col, end + 1):
            if str(row[col_idx - 1]).strip():
                last = row_idx
                break
    return last


def get_or_create_worksheet(spreadsheet, sheet_name: str, rows=100, cols=20):
    """Retrieves or creates a worksheet, seeding headers if creating NewMain."""
    try:
        ws = spreadsheet.worksheet(sheet_name)
    except gspread.exceptions.WorksheetNotFound:
        logger.info("Worksheet '%s' not found. Creating...", sheet_name)
        ws = spreadsheet.add_worksheet(title=sheet_name, rows=rows, cols=cols)
        if sheet_name == NEW_MAIN_SHEET_NAME:
            ws.update(values=[SHEET_HEADERS], range_name="B3")
            ws.format("B3:I3", {"textFormat": {"bold": True}})
            logger.info("Seeded header to B3:I3 on '%s'.", sheet_name)
    return ws


def grow_worksheet(ws, min_rows: int, min_cols: int):
    """Grows a worksheet if row/column capacity is insufficient."""
    cur_rows = ws.row_count
    cur_cols = ws.col_count
    target_rows = max(cur_rows, min_rows)
    target_cols = max(cur_cols, min_cols)
    if target_rows > cur_rows or target_cols > cur_cols:
        ws.resize(rows=target_rows, cols=target_cols)
        logger.info(
            "Resized '%s' from %dx%d to %dx%d.",
            ws.title,
            cur_rows,
            cur_cols,
            target_rows,
            target_cols,
        )


def sync_backend_worksheet(ws, entries: list[dict], dry_run: bool = False) -> dict:
    """Updates 'backend' first, then clears only the stale tail rows.

    The update is issued before any clear call. If the update raises, the sheet is
    left with its previous contents instead of being blanked first.
    """
    rows = build_sheet_data(entries)
    old_rows = _used_last_row(ws, min_row=1, min_col=1, max_col=len(SHEET_HEADERS))
    last_col = column_letter(len(SHEET_HEADERS))
    clear_ranges = []
    if old_rows > len(rows):
        clear_ranges.append(f"A{len(rows) + 1}:{last_col}{old_rows}")

    plan = {
        "worksheet": BACKEND_SHEET_NAME,
        "new_rows": len(rows),
        "old_rows": old_rows,
        "write_range": "A1",
        "clear_ranges": clear_ranges,
    }
    logger.info(
        "[%s] rows old=%d new=%d; write %s before clearing %s",
        BACKEND_SHEET_NAME,
        old_rows,
        len(rows),
        plan["write_range"],
        clear_ranges or "nothing",
    )
    if dry_run:
        return plan

    ws.update(values=rows, range_name="A1")
    if clear_ranges:
        ws.batch_clear(clear_ranges)
    ws.format(f"H2:H{len(rows)}", {"wrapStrategy": "WRAP"})
    logger.info(
        "✅ Successfully updated '%s' (%d rows, cleared tail %s).",
        BACKEND_SHEET_NAME,
        len(rows),
        clear_ranges or "none",
    )
    return plan


def sync_newmain_worksheet(ws, entries: list[dict], dry_run: bool = False) -> dict:
    """Updates 'NewMain' data first, then clears stale tails and stray columns."""
    newmain_rows = build_newmain_rows(entries)
    old_last_row = _used_last_row(
        ws,
        min_row=DATA_START_ROW,
        min_col=DATA_START_COL,
        max_col=DATA_START_COL + DATA_WIDTH - 1,
    )

    if newmain_rows:
        last_data_row = DATA_START_ROW - 1 + len(newmain_rows)
        write_values = newmain_rows
    else:
        last_data_row = DATA_START_ROW
        write_values = [[NO_DATA_MESSAGE]]

    clear_ranges = []
    if old_last_row > last_data_row:
        # Clear the whole stale tail (including an old stamp) in one shot.
        clear_ranges.append(f"B{last_data_row + 1}:Z{old_last_row}")
    if ws.col_count > DATA_START_COL + DATA_WIDTH - 1:
        # Drop stray columns beyond the I column for the live data rows.
        clear_ranges.append(f"J{DATA_START_ROW}:Z{last_data_row}")

    stamp_row = last_data_row + 2
    stamp_col = column_letter(STAMP_COLUMN)
    plan = {
        "worksheet": NEW_MAIN_SHEET_NAME,
        "new_rows": len(newmain_rows),
        "old_rows": max(old_last_row - DATA_START_ROW + 1, 0),
        "write_range": "B4",
        "last_data_row": last_data_row,
        "clear_ranges": clear_ranges,
        "stamp_range": f"{stamp_col}{stamp_row}",
    }
    logger.info(
        "[%s] rows old=%d new=%d; write %s before clearing %s",
        NEW_MAIN_SHEET_NAME,
        plan["old_rows"],
        len(newmain_rows),
        plan["write_range"],
        clear_ranges or "nothing",
    )
    if dry_run:
        return plan

    # Header is static, then the data overwrite, then the tail clear.
    ws.update(values=[SHEET_HEADERS], range_name="B3")
    ws.format("B3:I3", {"textFormat": {"bold": True}})
    ws.update(values=write_values, range_name="B4", value_input_option="RAW")
    if clear_ranges:
        ws.batch_clear(clear_ranges)

    # Write footer stamp
    now_dt = datetime.now(timezone.utc)
    ws.update(values=[[format_stamp(now_dt)]], range_name=f"{stamp_col}{stamp_row}")
    ws.update(values=[[SIGNATURE]], range_name=f"{stamp_col}{stamp_row + 1}")
    ws.format(f"{stamp_col}{stamp_row}", {"horizontalAlignment": "LEFT"})
    ws.format(f"{stamp_col}{stamp_row + 1}", {"horizontalAlignment": "RIGHT"})

    # Wrap contact column (Teacher Phone and Email) in NewMain.
    contact_col_idx = DATA_START_COL + SHEET_HEADERS.index("Teacher Phone and Email")
    contact_letter = column_letter(contact_col_idx)
    ws.format(f"{contact_letter}4:{contact_letter}{last_data_row}", {"wrapStrategy": "WRAP"})

    logger.info(
        "✅ Successfully updated '%s' (%d sorted rows, cleared tail %s).",
        NEW_MAIN_SHEET_NAME,
        len(newmain_rows),
        clear_ranges or "none",
    )
    return plan


def preview_sync_plan(entries: list[dict]) -> dict:
    """Builds the intended ranges without authenticating to Google."""
    backend_rows = build_sheet_data(entries)
    newmain_rows = build_newmain_rows(entries)
    last_data_row = (
        DATA_START_ROW - 1 + len(newmain_rows) if newmain_rows else DATA_START_ROW
    )
    stamp_col = column_letter(STAMP_COLUMN)
    return {
        "backend": {
            "new_rows": len(backend_rows),
            "write_range": "A1",
            "tail_cleared_after_update_if_needed": f"A{len(backend_rows) + 1}:H",
        },
        "newmain": {
            "new_rows": len(newmain_rows),
            "write_range": "B4",
            "last_data_row": last_data_row,
            "tail_cleared_after_update_if_needed": f"B{last_data_row + 1}:Z",
            "stamp_range": f"{stamp_col}{last_data_row + 2}",
        },
    }


def sync_to_google_sheet(
    json_path: Path,
    key_path: Path,
    spreadsheet_name: str,
    dry_run: bool = False,
) -> bool:
    """Executes full synchronization of routine_data.json to the Google Spreadsheet."""
    if not json_path.exists():
        logger.error("Data JSON not found at '%s'.", json_path)
        return False

    with open(json_path, encoding="utf-8") as f:
        data = json.load(f)

    entries = data.get("entries", [])
    if not entries:
        logger.warning("No entries in routine data to write.")
        return False

    logger.info("Loaded %d routine entries from '%s'.", len(entries), json_path)

    if dry_run:
        plan = preview_sync_plan(entries)
        logger.info("Dry run plan (no Google API calls): %s", json.dumps(plan, indent=2))
        return True

    if gspread is None:
        logger.error("gspread is not installed; run 'uv sync --locked'.")
        return False

    # Resolve service account key
    if not key_path.exists():
        logger.error("Service account key not found at '%s'.", key_path)
        return False

    try:
        gc = gspread.service_account(filename=str(key_path))
        sh = gc.open(spreadsheet_name)
        logger.info("Opened Google Spreadsheet: '%s'", spreadsheet_name)
    except Exception as e:
        logger.error("Failed to authenticate or open spreadsheet: %s", e)
        return False

    # 1. Update 'backend' worksheet
    try:
        ws_backend = get_or_create_worksheet(sh, BACKEND_SHEET_NAME)
        grow_worksheet(ws_backend, len(entries) + 10, len(SHEET_HEADERS) + 2)
        sync_backend_worksheet(ws_backend, entries)
    except Exception as e:
        logger.error("Error updating '%s' (old contents preserved): %s", BACKEND_SHEET_NAME, e)
        return False

    # 2. Update 'NewMain' worksheet
    try:
        ws_main = get_or_create_worksheet(sh, NEW_MAIN_SHEET_NAME)
        grow_worksheet(ws_main, len(entries) + 15, DATA_START_COL + DATA_WIDTH + 1)
        sync_newmain_worksheet(ws_main, entries)
    except Exception as e:
        logger.error("Error updating '%s' (old contents preserved): %s", NEW_MAIN_SHEET_NAME, e)
        return False

    return True


def main():
    parser = argparse.ArgumentParser(description="Sync routine to Google Spreadsheet")
    parser.add_argument("--data", type=Path, default=Path("output/routine_data.json"))
    parser.add_argument("--key", type=Path, default=None)
    parser.add_argument("--sheet", type=str, default=None)
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print computed ranges and row counts without calling the Google API",
    )
    args = parser.parse_args()

    data_path = args.data
    if not data_path.exists():
        fallback_root = Path("routine_data.json")
        if fallback_root.exists():
            data_path = fallback_root

    key_path = (
        args.key
        or Path(os.getenv("GOOGLE_SERVICE_ACCOUNT_KEY", "google_cloud_keys/service_account_key.json"))
    )

    sheet_name = args.sheet or os.getenv("SPREADSHEET_NAME", "CSE-03_B_ClassRoutine")

    success = sync_to_google_sheet(data_path, key_path, sheet_name, dry_run=args.dry_run)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()
