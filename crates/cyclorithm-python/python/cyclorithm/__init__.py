"""Python-биндинги движка расписаний Cyclorithm.

Ввод — путь к файлу (`*_file`) или текст программы (`*_text`, `use`
резолвится от `base_dir`, по умолчанию — cwd, как stdin в CLI).
Границы окна (`start`/`end`) — `str` (короткие формы CLI), `datetime`,
`date` (полночь) или `timedelta` (дельта: для `start` — от now,
для `end` — от `start`, как `+DURATION` в CLI).
Ошибки движка — `CycloError` с полями `code` (слаг главы ошибок)
и `message`; отсутствие файла — штатный `OSError`.
"""

import json as _json
from datetime import date as _date
from datetime import datetime as _datetime
from datetime import timedelta as _timedelta
from datetime import timezone as _timezone
from pathlib import Path as _Path

from cyclorithm._cyclorithm import (
    CycloError,
    check_text as _check_text,
    core_version,
    run_text as _run_text,
)

__version__ = core_version()


def _error_code(self):
    """Слаг главы ошибок (`cycle-overruns`, …; парсер — `syntax`)."""
    return self.args[0]


def _error_message(self):
    """Текст без слага, дословно по таблице главы ошибок."""
    return self.args[1]


def _error_str(self):
    return f"{self.args[0]}: {self.args[1]}"


# create_exception! не умеет в поля (pyo3 0.29 запрещает наследовать
# PyException через pyclass), поэтому code/message — свойства поверх args.
CycloError.code = property(_error_code)
CycloError.message = property(_error_message)
CycloError.__str__ = _error_str

__all__ = [
    "CycloError",
    "Schedule",
    "check_file",
    "check_text",
    "run_file",
    "run_text",
    "__version__",
    "core_version",
]

DateLike = str | _datetime | _date | _timedelta


def _as_datetime_str(value: DateLike, name: str) -> str:
    """Граница окна в каноническую строку ядра `YYYY-MM-DDTHH:MM:SS[.mmm][Z|±HH:MM]`.

    `str` — как есть (короткие формы и `+DURATION` разбирает ядро);
    `datetime` — наивное как есть, aware — wall + офсет (`Z`/±HH:MM);
    микросекунды усекаются до миллисекунд (точность движка);
    отрицательный `timedelta` — `TypeError`.
    """
    if isinstance(value, str):
        return value
    if isinstance(value, _datetime):
        ms = f".{value.microsecond // 1000:03d}" if value.microsecond else ""
        wall = (
            f"{value.year:04d}-{value.month:02d}-{value.day:02d}"
            f"T{value.hour:02d}:{value.minute:02d}:{value.second:02d}{ms}"
        )
        if value.tzinfo is not None:
            off = value.utcoffset()
            if off is None:
                return wall
            total_min = int(off.total_seconds() // 60)
            if total_min == 0:
                return wall + "Z"
            sign = "+" if total_min >= 0 else "-"
            abs_min = abs(total_min)
            hh = abs_min // 60
            mm = abs_min % 60
            return wall + f"{sign}{hh:02d}:{mm:02d}"
        return wall
    if isinstance(value, _date):
        return f"{value.year:04d}-{value.month:02d}-{value.day:02d}T00:00:00"
    if isinstance(value, _timedelta):
        total_ms = (
            value.days * 86_400 + value.seconds
        ) * 1000 + value.microseconds // 1000
        if total_ms < 0:
            raise TypeError(f"{name}: отрицательный timedelta без якоря")
        return f"+{total_ms}"
    raise TypeError(
        f"{name}: ожидаются str, datetime, date или timedelta,"
        f" получен {type(value).__name__}"
    )


def _read_file(path):
    """Текст файла и директория для `use`."""
    p = _Path(path)
    return p.read_text(encoding="utf-8"), str(p.parent)


def check_file(path):
    """Валидация файла: молча OK или `CycloError`."""
    text, base = _read_file(path)
    _check_text(text, base)


def check_text(text, base_dir=None):
    """Валидация текста программы: молча OK или `CycloError`."""
    _check_text(text, "" if base_dir is None else str(base_dir))


def run_file(path, start: DateLike, end: DateLike):
    """Окно событий файла: dict как JSON-объект `cyclo run`."""
    text, base = _read_file(path)
    return _json.loads(
        _run_text(
            text, _as_datetime_str(start, "start"), _as_datetime_str(end, "end"), base
        )
    )


def run_text(text, start: DateLike, end: DateLike, base_dir=None):
    """Окно событий текста программы: dict как JSON-объект `cyclo run`."""
    return _json.loads(
        _run_text(
            text,
            _as_datetime_str(start, "start"),
            _as_datetime_str(end, "end"),
            "" if base_dir is None else str(base_dir),
        )
    )


class Schedule:
    """Расписание как объект: текст читается один раз, дальше — методы.

    Тонкая обёртка поверх функций (тот же выход и те же ошибки);
    конструктор лёгкий и не валидирует — ошибка только в `check`/`run`.
    """

    def __init__(self, path):
        """Программа из файла; `use` — от директории файла."""
        self._text, self._base = _read_file(path)

    @classmethod
    def from_text(cls, text, base_dir=None):
        """Программа строкой; `use` — от `base_dir` (по умолчанию cwd)."""
        sched = cls.__new__(cls)
        sched._text = text
        sched._base = "" if base_dir is None else str(base_dir)
        return sched

    def check(self):
        """Валидация: молча OK или `CycloError`."""
        _check_text(self._text, self._base)

    def run(self, start: DateLike, end: DateLike):
        """Окно событий: dict как JSON-объект `cyclo run`."""
        return _json.loads(
            _run_text(
                self._text,
                _as_datetime_str(start, "start"),
                _as_datetime_str(end, "end"),
                self._base,
            )
        )
