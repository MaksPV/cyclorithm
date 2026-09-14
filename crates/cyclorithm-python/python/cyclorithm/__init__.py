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
    "check_file",
    "check_text",
    "run_file",
    "run_text",
    "__version__",
    "core_version",
]

DateLike = str | _datetime | _date | _timedelta


def _as_datetime_str(value: DateLike, name: str) -> str:
    """Граница окна в каноническую строку ядра `YYYY-MM-DDTHH:MM:SS[.mmm]`.

    `str` — как есть (короткие формы и `+DURATION` разбирает ядро);
    aware-`datetime` — в UTC без зоны; микросекунды усекаются до
    миллисекунд (точность движка); отрицательный `timedelta` —
    `TypeError` (якоря-строки Python разрешить не может).
    """
    if isinstance(value, str):
        return value
    if isinstance(value, _datetime):
        if value.tzinfo is not None:
            value = value.astimezone(_timezone.utc).replace(tzinfo=None)
        ms = f".{value.microsecond // 1000:03d}" if value.microsecond else ""
        return (
            f"{value.year:04d}-{value.month:02d}-{value.day:02d}"
            f"T{value.hour:02d}:{value.minute:02d}:{value.second:02d}{ms}"
        )
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
