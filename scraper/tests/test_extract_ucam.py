"""Offline tests for scraper/extract_ucam.py. No browser or network access."""

import json
import sys
from pathlib import Path

import pytest

from scraper import extract_ucam

FIXTURES = Path(__file__).parent / "fixtures"


def fixture_text(name: str) -> str:
    return (FIXTURES / name).read_text(encoding="utf-8")


def test_normal_table_uses_header_columns():
    entries = extract_ucam.parse_report_html(fixture_text("normal.html"))

    assert len(entries) == 3
    first = entries[0]
    assert first["CourseCode"] == "CSE-4101"
    assert first["CourseTitle"] == "Sample Course One"
    assert first["Teacher"] == "AB_Sample Teacher"
    assert first["TeacherPhone"] == "01000000000"
    assert first["TeacherEmail"] == "sample.teacher@example.test"
    assert first["Day"] == "Mon"
    assert first["Room"] == "AC-101"
    assert first["TimeSlot"] == "9:15 AM - 10:30 AM"
    assert first["Section"] == "B"

    second_course = entries[2]
    assert second_course["CourseCode"] == "CSE-4102"
    assert second_course["Room"] == "AC-201"
    assert second_course["Section"] == "B1"


def test_missing_column_falls_back_and_warns(caplog):
    entries = extract_ucam.parse_report_html(
        fixture_text("missing_column.html"), default_section="B2"
    )

    assert len(entries) == 1
    assert entries[0]["Section"] == "B2"
    assert "no 'section' column" in caplog.text


def test_extra_column_is_ignored():
    entries = extract_ucam.parse_report_html(fixture_text("extra_column.html"))

    assert len(entries) == 1
    entry = entries[0]
    assert entry["CourseCode"] == "CSE-4103"
    assert entry["Room"] == "AC-303"
    assert entry["Section"] == "B2"
    assert "Notes" not in entry


def test_empty_table_returns_no_entries():
    assert extract_ucam.parse_report_html(fixture_text("empty.html")) == []


def test_malformed_row_does_not_crash():
    entries = extract_ucam.parse_report_html(fixture_text("malformed.html"))

    assert len(entries) == 1
    assert entries[0]["CourseCode"] == "CSE-4104"
    assert entries[0]["Day"] == "Fri"
    assert entries[0]["Room"] == ""
    assert entries[0]["TimeSlot"] == ""


def test_headerless_table_uses_positional_fallback(caplog):
    entries = extract_ucam.parse_report_html(fixture_text("headerless.html"))

    assert len(entries) == 1
    assert entries[0]["CourseCode"] == "CSE-4101"
    assert entries[0]["CourseTitle"] == "Sample Course One"
    assert entries[0]["Room"] == "AC-101"
    assert entries[0]["TimeSlot"] == "9:15 AM - 10:30 AM"
    assert entries[0]["Section"] == "B"
    assert "positional fallback" in caplog.text


def _write_existing(path: Path, count: int) -> None:
    payload = {
        "generated_at": "2026-01-01T00:00:00+00:00",
        "semester": "Existing",
        "accounts_processed": ["existing"],
        "entries": [{"CourseCode": f"OLD-{i}", "Day": "Mon"} for i in range(count)],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")


def test_downgrade_guard_preserves_larger_existing_file(tmp_path):
    output = tmp_path / "routine_data.json"
    _write_existing(output, 5)

    allowed, old_count = extract_ucam.downgrade_guard(output, 4, force=False)

    assert allowed is False
    assert old_count == 5
    assert len(json.loads(output.read_text())["entries"]) == 5


def test_downgrade_guard_force_overwrites(tmp_path):
    output = tmp_path / "routine_data.json"
    _write_existing(output, 5)

    allowed, old_count = extract_ucam.downgrade_guard(output, 4, force=True)

    assert allowed is True
    assert old_count == 5


def test_run_parser_from_html_respects_force(tmp_path):
    output = tmp_path / "routine_data.json"
    _write_existing(output, 10)

    assert (
        extract_ucam.run_parser_from_html(
            FIXTURES / "normal.html", output, dry_run=False, force=False
        )
        is False
    )
    assert len(json.loads(output.read_text())["entries"]) == 10

    assert (
        extract_ucam.run_parser_from_html(
            FIXTURES / "normal.html", output, dry_run=False, force=True
        )
        is True
    )
    assert len(json.loads(output.read_text())["entries"]) == 3


def test_cli_html_file_dry_run_writes_stdout(tmp_path, monkeypatch, capsys):
    output = tmp_path / "routine_data.json"
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "extract_ucam.py",
            "--html-file",
            str(FIXTURES / "normal.html"),
            "--output",
            str(output),
            "--dry-run",
        ],
    )

    with pytest.raises(SystemExit) as exc:
        extract_ucam.main()

    assert exc.value.code == 0
    payload = json.loads(capsys.readouterr().out)
    assert len(payload["entries"]) == 3
    assert not output.exists()


def test_redaction_removes_password_and_token_sentinels():
    extract_ucam.register_secret("p@ssw0rd-sentinel")

    message = extract_ucam.redact(
        "login failed password=p@ssw0rd-sentinel token=abcdef123456"
    )

    assert "p@ssw0rd-sentinel" not in message
    assert "abcdef123456" not in message
    assert "<redacted>" in message


def test_attendance_percent_only_marks_derived_counts():
    records = extract_ucam.parse_attendance_html(
        fixture_text("attendance_percent_only.html")
    )
    by_code = {r["course_code"]: r for r in records}

    assert by_code["CSE-4101"]["attended"] == 8
    assert by_code["CSE-4101"]["missed"] == 1
    assert by_code["CSE-4101"]["total_classes"] == 9
    assert by_code["CSE-4101"]["percentage"] == 88.9
    assert by_code["CSE-4101"]["derived"] is True

    assert by_code["CSE-4102"]["attended"] == 2
    assert by_code["CSE-4102"]["missed"] == 1
    assert by_code["CSE-4102"]["derived"] is True

    assert by_code["CSE-4103"]["attended"] == 0
    assert by_code["CSE-4103"]["missed"] == 0
    assert by_code["CSE-4103"]["derived"] is False


def test_attendance_exact_counts_are_preserved():
    records = extract_ucam.parse_attendance_html(fixture_text("attendance_exact.html"))

    assert len(records) == 1
    record = records[0]
    assert record["attended"] == 8
    assert record["missed"] == 1
    assert record["derived"] is False


def test_prefer_attendance_keeps_exact_over_derived():
    exact = {"course_code": "CSE-4101", "attended": 8, "derived": False}
    derived = {"course_code": "CSE-4101", "attended": 7, "derived": True}

    assert extract_ucam.prefer_attendance(derived, exact) is exact
    assert extract_ucam.prefer_attendance(exact, derived) is exact


def test_nodriver_version_check_rejects_unsupported_line(monkeypatch):
    monkeypatch.setattr(
        extract_ucam.importlib.metadata, "version", lambda _name: "0.51.0"
    )
    with pytest.raises(RuntimeError, match="Unsupported nodriver version"):
        extract_ucam.assert_supported_nodriver()


def test_nodriver_version_check_accepts_tested_line(monkeypatch):
    monkeypatch.setattr(
        extract_ucam.importlib.metadata, "version", lambda _name: "0.50.6"
    )
    extract_ucam.assert_supported_nodriver()
