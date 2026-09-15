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


def test_aware_datetime_converts_to_utc():
    msk = timezone(timedelta(hours=3))
    aware = c.run_file(VALID, datetime(2026, 9, 7, 9, 0, tzinfo=msk), END)
    naive = c.run_file(VALID, "2026-09-07T06:00:00", END)
    # Один инстант, но окно aware форматируется в своей зоне (+03:00).
    assert aware["events"][0]["time"].endswith("+03:00")
    assert naive["events"][0]["time"] == "2026-09-07T06:00:00"
    # Инстанты совпадают: 09:00+03:00 == 06:00Z.
    from datetime import timezone as _tz

    def to_utc(s):
        # s вида YYYY-MM-DDTHH:MM:SS[.mmm][Z|±HH:MM]; наивное — трактуем как UTC.
        dt = datetime.fromisoformat(s.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=_tz.utc)
        return dt.astimezone(_tz.utc)

    assert to_utc(aware["events"][0]["time"]) == to_utc(naive["events"][0]["time"])


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
