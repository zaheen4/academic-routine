"""Offline tests for scraper/sync_gsheet.py using a recording fake worksheet."""

import pytest

from scraper import sync_gsheet


class FakeWorksheet:
    """Minimal gspread worksheet double that records the call order."""

    def __init__(self, values=None, title="ws", row_count=100, col_count=12):
        self.values = values or []
        self.title = title
        self.row_count = row_count
        self.col_count = col_count
        self.calls = []
        self.fail_on_update_range = None

    def get_all_values(self):
        return [list(row) for row in self.values]

    def update(self, *, values=None, range_name=None, value_input_option=None):
        if self.fail_on_update_range == range_name:
            raise RuntimeError("simulated update failure")
        self.calls.append(("update", range_name, values, value_input_option))

    def batch_clear(self, ranges):
        self.calls.append(("batch_clear", list(ranges)))

    def format(self, range_name, spec):
        self.calls.append(("format", range_name, spec))

    def resize(self, rows, cols):
        self.row_count = rows
        self.col_count = cols
        self.calls.append(("resize", rows, cols))


def sample_entries(count=2):
    return [
        {
            "CourseCode": f"CSE-410{i}",
            "CourseTitle": f"Sample Course {i}",
            "Section": "B",
            "Day": "Mon" if i % 2 else "Tue",
            "Room": f"AC-{100 + i}",
            "TimeSlot": "9:15 AM - 10:30 AM",
            "Teacher": "AB_Sample Teacher",
            "TeacherPhone": "01000000000",
            "TeacherEmail": "sample.teacher@example.test",
        }
        for i in range(1, count + 1)
    ]


def call_names(ws):
    return [call[0] for call in ws.calls]


def test_backend_updates_before_clearing_tail():
    old_values = [[f"old-{r}"] + [""] * 7 for r in range(5)]
    ws = FakeWorksheet(old_values, title="backend", col_count=12)

    plan = sync_gsheet.sync_backend_worksheet(ws, sample_entries(2))

    names = call_names(ws)
    assert names.index("update") < names.index("batch_clear")
    assert plan["new_rows"] == 3
    assert plan["old_rows"] == 5
    clear_call = next(call for call in ws.calls if call[0] == "batch_clear")
    assert clear_call[1] == ["A4:H5"]


def test_backend_failed_update_never_clears():
    old_values = [[f"old-{r}"] + [""] * 7 for r in range(5)]
    ws = FakeWorksheet(old_values, title="backend", col_count=12)
    ws.fail_on_update_range = "A1"

    with pytest.raises(RuntimeError, match="simulated update failure"):
        sync_gsheet.sync_backend_worksheet(ws, sample_entries(2))

    assert "batch_clear" not in call_names(ws)


def test_backend_longer_new_data_does_not_clear():
    old_values = [["Course"] + [""] * 7]
    ws = FakeWorksheet(old_values, title="backend", col_count=12)

    plan = sync_gsheet.sync_backend_worksheet(ws, sample_entries(2))

    assert plan["clear_ranges"] == []
    assert "batch_clear" not in call_names(ws)


def test_newmain_updates_data_before_clearing_tail():
    old_values = [[""] * 9 for _ in range(10)]
    for row_idx in range(3, 10):
        old_values[row_idx][1] = f"old-{row_idx}"
    ws = FakeWorksheet(old_values, title="NewMain", col_count=9)

    plan = sync_gsheet.sync_newmain_worksheet(ws, sample_entries(2))

    names = call_names(ws)
    data_update_index = next(
        i
        for i, call in enumerate(ws.calls)
        if call[0] == "update" and call[1] == "B4"
    )
    clear_index = names.index("batch_clear")
    assert data_update_index < clear_index
    assert plan["new_rows"] == 2
    assert plan["last_data_row"] == 5
    clear_call = next(call for call in ws.calls if call[0] == "batch_clear")
    assert "B6:Z10" in clear_call[1]


def test_newmain_failed_data_update_never_clears():
    old_values = [[""] * 9 for _ in range(10)]
    for row_idx in range(3, 10):
        old_values[row_idx][1] = f"old-{row_idx}"
    ws = FakeWorksheet(old_values, title="NewMain", col_count=9)
    ws.fail_on_update_range = "B4"

    with pytest.raises(RuntimeError, match="simulated update failure"):
        sync_gsheet.sync_newmain_worksheet(ws, sample_entries(2))

    assert "batch_clear" not in call_names(ws)


def test_newmain_longer_new_data_does_not_clear_tail():
    old_values = [[""] * 9 for _ in range(6)]
    for row_idx in range(3, 6):
        old_values[row_idx][1] = f"old-{row_idx}"
    ws = FakeWorksheet(old_values, title="NewMain", col_count=9)

    plan = sync_gsheet.sync_newmain_worksheet(ws, sample_entries(4))

    assert plan["clear_ranges"] == []
    assert "batch_clear" not in call_names(ws)


def test_newmain_dry_run_only_reads():
    old_values = [[""] * 9 for _ in range(10)]
    old_values[9][1] = "stale-row-10"
    ws = FakeWorksheet(old_values, title="NewMain", col_count=9)

    plan = sync_gsheet.sync_newmain_worksheet(ws, sample_entries(2), dry_run=True)

    assert plan["clear_ranges"] == ["B6:Z10"]
    assert ws.calls == []


def test_preview_sync_plan_reports_ranges():
    plan = sync_gsheet.preview_sync_plan(sample_entries(2))

    assert plan["backend"]["new_rows"] == 3
    assert plan["newmain"]["new_rows"] == 2
    assert plan["newmain"]["write_range"] == "B4"
    assert plan["newmain"]["last_data_row"] == 5
