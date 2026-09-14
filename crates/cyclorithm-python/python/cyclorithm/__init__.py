"""Python-биндинги движка расписаний Cyclorithm.

Ввод — путь к файлу (`*_file`) или текст программы (`*_text`, `use`
резолвится от `base_dir`, по умолчанию — cwd, как stdin в CLI).
Ошибки движка — `CycloError` с полями `code` (слаг главы ошибок)
и `message`; отсутствие файла — штатный `OSError`.
"""

import json as _json
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


def run_file(path, start, end):
    """Окно событий файла: dict как JSON-объект `cyclo run`."""
    text, base = _read_file(path)
    return _json.loads(_run_text(text, start, end, base))


def run_text(text, start, end, base_dir=None):
    """Окно событий текста программы: dict как JSON-объект `cyclo run`."""
    return _json.loads(
        _run_text(text, start, end, "" if base_dir is None else str(base_dir))
    )
