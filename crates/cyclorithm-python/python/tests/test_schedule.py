"""Тесты класса Schedule (обёртка поверх функций)."""

from datetime import datetime, timedelta
from pathlib import Path

import pytest

import cyclorithm as c

REPO = Path(__file__).resolve().parents[4]
VALID = REPO / "examples" / "valid" / "route.cyclo"
INVALID = REPO / "examples" / "invalid"

START = "2026-09-07T00:00:00"
END = "2026-09-08T00:00:00"


def test_from_file_check_and_run_match_functions():
    sched = c.Schedule(VALID)
    assert sched.check() is None
    assert sched.run(START, END) == c.run_file(VALID, START, END)


def test_from_text_needs_base_dir_for_use():
    text = VALID.read_text(encoding="utf-8")
    with pytest.raises(c.CycloError) as exc:
        c.Schedule.from_text(text).check()
    assert exc.value.code == "cannot-read-import"
    sched = c.Schedule.from_text(text, base_dir=VALID.parent)
    assert sched.check() is None
    # Эхо start/end в выходе зависит от формы входа, события — нет.
    assert sched.run(START, END) == c.run_file(VALID, START, END)
    assert (
        sched.run(datetime(2026, 9, 7), timedelta(days=1))["events"]
        == c.run_file(VALID, START, END)["events"]
    )


def test_run_error_propagates_slug():
    sched = c.Schedule(INVALID / "bad_cycle-overruns-repeat.cyclo")
    with pytest.raises(c.CycloError) as exc:
        sched.run(START, END)
    assert exc.value.code == "cycle-overruns"


def test_missing_file_is_oserror():
    with pytest.raises(OSError):
        c.Schedule(INVALID / "nope.cyclo")
