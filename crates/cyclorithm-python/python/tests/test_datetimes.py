"""Тесты конвертации границ окна (слой Python, не движок)."""

from datetime import date, datetime, timedelta, timezone

import pytest

import cyclorithm as c
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
VALID = REPO / "examples" / "valid" / "route.cyclo"

START = "2026-09-07T00:00:00"
END = "2026-09-08T00:00:00"


def test_naive_datetime_matches_string():
    assert c.run_file(VALID, datetime(2026, 9, 7), datetime(2026, 9, 8)) == c.run_file(
        VALID, START, END
    )


def test_aware_window_inherits_naive_walls():
    # Наивный файл + aware-окно: стены стоят (06:00 остаётся 06:00 в зоне
    # окна), инстанты = стена − зона окна (см. docs/reference/semantics.md).
    tz = REPO / "examples" / "valid" / "tz_offsets.cyclo"
    msk = timezone(timedelta(hours=3))
    aware = c.run_file(
        tz,
        datetime(2026, 1, 9, 0, 0, tzinfo=msk),
        datetime(2026, 1, 10, 0, 0, tzinfo=msk),
    )
    naive = c.run_file(tz, "2026-01-09T00:00:00", "2026-01-10T00:00:00")
    assert [e["time"] for e in aware["events"]] == [
        "2026-01-09T06:00:00+03:00",
        "2026-01-09T07:00:00+03:00",
        "2026-01-09T18:00:00+03:00",
        "2026-01-09T19:00:00+03:00",
    ]
    assert [e["time"] for e in naive["events"]] == [
        "2026-01-09T06:00:00",
        "2026-01-09T07:00:00",
        "2026-01-09T18:00:00",
        "2026-01-09T19:00:00",
    ]


def test_date_means_midnight():
    assert c.run_file(VALID, date(2026, 9, 7), date(2026, 9, 8)) == c.run_file(
        VALID, START, END
    )


def test_timedelta_end_matches_plus_duration():
    assert c.run_file(VALID, START, timedelta(days=1)) == c.run_file(
        VALID, START, "+86400000"
    )


def test_microseconds_truncate_to_milliseconds():
    out = c.run_file(VALID, datetime(2026, 9, 7, 0, 0, 0, 999999), END)
    assert out["start"] == "2026-09-07T00:00:00.999"


def test_bad_type_is_type_error():
    with pytest.raises(TypeError) as exc:
        c.run_file(VALID, 20260907, END)
    assert "start" in str(exc.value)


def test_negative_timedelta_is_type_error():
    with pytest.raises(TypeError):
        c.run_file(VALID, START, timedelta(days=-1))
