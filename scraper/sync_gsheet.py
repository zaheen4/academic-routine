#!/usr/bin/env python3
"""
Google Sheets Routine Synchronizer for academic-routine.

Synchronizes scraped academic routine data directly to Google Spreadsheet
(updating both 'backend' and 'NewMain' worksheets).
"""

import argparse
from datetime import datetime, timedelta, timezone
import json
import logging
import os
from pathlib import Path
import re
import sys

from dotenv import load_dotenv
import gspread

load_dotenv()

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("sync_gsheet")

# Layout Constants matching routine-to-gsheet
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
    """Formats full slot (e.g. '8:0 AM - 9:15 AM' or '11:45 - 1:0') into clean 12h range and returns start minute."""
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


def get_or_create_worksheet(spreadsheet: gspread.Spreadsheet, sheet_name: str, rows=100, cols=20):
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


def grow_worksheet(ws: gspread.Worksheet, min_rows: int, min_cols: int):
    """Grows a worksheet if row/column capacity is insufficient."""
    cur_rows = ws.row_count
    cur_cols = ws.col_count
    target_rows = max(cur_rows, min_rows)
    target_cols = max(cur_cols, min_cols)
    if target_rows > cur_rows or target_cols > cur_cols:
        ws.resize(rows=target_rows, cols=target_cols)
        logger.info("Resized '%s' from %dx%d to %dx%d.", ws.title, cur_rows, cur_cols, target_rows, target_cols)


def sync_to_google_sheet(json_path: Path, key_path: Path, spreadsheet_name: str) -> bool:
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
        backend_rows = build_sheet_data(entries)
        ws_backend.clear()
        ws_backend.update(values=backend_rows, range_name="A1")
        # Format contact column with text wrap
        ws_backend.format("H2:H" + str(len(backend_rows)), {"wrapStrategy": "WRAP"})
        logger.info("✅ Successfully updated '%s' (%d rows).", BACKEND_SHEET_NAME, len(backend_rows))
    except Exception as e:
        logger.error("Error updating '%s': %s", BACKEND_SHEET_NAME, e)
        return False

    # 2. Update 'NewMain' worksheet
    try:
        ws_main = get_or_create_worksheet(sh, NEW_MAIN_SHEET_NAME)
        grow_worksheet(ws_main, len(entries) + 15, DATA_START_COL + DATA_WIDTH + 1)

        # Seed or refresh header at B3:I3
        ws_main.update(values=[SHEET_HEADERS], range_name="B3")
        ws_main.format("B3:I3", {"textFormat": {"bold": True}})

        newmain_rows = build_newmain_rows(entries)
        if newmain_rows:
            ws_main.update(values=newmain_rows, range_name="B4", value_input_option="RAW")
            last_data_row = DATA_START_ROW - 1 + len(newmain_rows)
        else:
            ws_main.update(values=[[NO_DATA_MESSAGE]], range_name="B4")
            last_data_row = DATA_START_ROW

        # Clear stale tail below data and any stray columns beyond I
        stale_from = last_data_row + 1
        clear_ranges = []
        if stale_from <= ws_main.row_count:
            clear_ranges.append(f"B{stale_from}:Z{ws_main.row_count}")
        if ws_main.col_count > DATA_START_COL + DATA_WIDTH - 1:
            clear_ranges.append(f"J4:Z{last_data_row}")
        if clear_ranges:
            ws_main.batch_clear(clear_ranges)

        # Write footer stamp
        stamp_row = last_data_row + 2
        stamp_col = column_letter(STAMP_COLUMN)
        now_dt = datetime.now(timezone.utc)
        ws_main.update(values=[[format_stamp(now_dt)]], range_name=f"{stamp_col}{stamp_row}")
        ws_main.update(values=[[SIGNATURE]], range_name=f"{stamp_col}{stamp_row + 1}")
        ws_main.format(f"{stamp_col}{stamp_row}", {"horizontalAlignment": "LEFT"})
        ws_main.format(f"{stamp_col}{stamp_row + 1}", {"horizontalAlignment": "RIGHT"})

        # Wrap contact column in NewMain (Column I is stamp, Column I in B-based is Col 9 => Col H in 0-based is B+7 = I)
        contact_col_idx = DATA_START_COL + SHEET_HEADERS.index("Teacher Phone and Email")
        contact_letter = column_letter(contact_col_idx)
        ws_main.format(f"{contact_letter}4:{contact_letter}{last_data_row}", {"wrapStrategy": "WRAP"})

        logger.info("✅ Successfully updated '%s' (%d sorted rows).", NEW_MAIN_SHEET_NAME, len(newmain_rows))
    except Exception as e:
        logger.error("Error updating '%s': %s", NEW_MAIN_SHEET_NAME, e)
        return False

    return True


def main():
    parser = argparse.ArgumentParser(description="Sync routine to Google Spreadsheet")
    parser.add_argument("--data", type=Path, default=Path("output/routine_data.json"))
    parser.add_argument("--key", type=Path, default=None)
    parser.add_argument("--sheet", type=str, default=None)
    args = parser.parse_args()

    # Resolve routine data path
    data_path = args.data
    if not data_path.exists():
        fallback_root = Path("routine_data.json")
        if fallback_root.exists():
            data_path = fallback_root
        else:
            fallback_legacy = Path("output_of_fetched_routine/final_combined_routine.json")
            if fallback_legacy.exists():
                data_path = fallback_legacy

    # Resolve key path
    key_path = (
        args.key
        or Path(os.getenv("GOOGLE_SERVICE_ACCOUNT_KEY", "google_cloud_keys/service_account_key.json"))
    )
    if not key_path.exists():
        fallback = Path("/home/eyepatch/Documents/projects/routine-to-gsheet/google_cloud_keys/service_account_key.json")
        if fallback.exists():
            key_path = fallback

    sheet_name = args.sheet or os.getenv("SPREADSHEET_NAME", "CSE-03_B_ClassRoutine")

    success = sync_to_google_sheet(data_path, key_path, sheet_name)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()
