"""Тесты слоя биндинга (ввод/вывод/ошибки).

Семантику движка не дублируют — она покрыта Rust-тестами и e2e;
здесь только: формы ввода, форма выхода, маппинг ошибок.
"""

import json
import subprocess
from pathlib import Path

import pytest

import cyclorithm as c

REPO = Path(__file__).resolve().parents[4]
VALID = REPO / "examples" / "valid" / "route.cyclo"
INVALID = REPO / "examples" / "invalid"


def test_version_matches_core():
    assert c.__version__ == c.core_version()


def test_check_file_ok():
    assert c.check_file(VALID) is None


def test_check_text_ok():
    assert (
        c.check_text(VALID.read_text(encoding="utf-8"), base_dir=VALID.parent) is None
    )


def test_check_core_error_has_slug():
    with pytest.raises(c.CycloError) as exc:
        c.check_file(INVALID / "bad_cycle-overruns-repeat.cyclo")
    assert exc.value.code == "cycle-overruns"
    assert str(exc.value).startswith("cycle-overruns: ")


def test_check_syntax_error_has_syntax_code():
    # bad_syntax.cyclo теперь missing-argument (отсутствие root_cycle —
    # поле шапки, а не голый синтаксис); чистый синтаксис проверяем строкой.
    with pytest.raises(c.CycloError) as exc:
        c.check_text('schedule "T" { point A { actions = [x] } cycle R duration = 1h { 0m: A.x(); }')
    assert exc.value.code == "syntax"


def test_check_missing_file_is_oserror():
    with pytest.raises(OSError):
        c.check_file(INVALID / "nope.cyclo")


def test_run_matches_cli_byte_for_byte():
    start, end = "2026-09-07T00:00:00", "2026-09-08T00:00:00"
    cli = subprocess.run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "cyclorithm-cli",
            "--",
            "run",
            str(VALID),
            "--start",
            start,
            "--end",
            end,
        ],
        capture_output=True,
        text=True,
        cwd=REPO,
        check=True,
    )
    assert c.run_file(VALID, start, end) == json.loads(cli.stdout)


def test_run_text_uses_base_dir_for_use():
    # route.cyclo начинается с use "libs/route_lib.cyclo" — без base_dir
    # импорт не резолвится, с ним — тот же результат, что у run_file.
    text = VALID.read_text(encoding="utf-8")
    start, end = "2026-09-07T00:00:00", "2026-09-08T00:00:00"
    with pytest.raises(c.CycloError) as exc:
        c.run_text(text, start, end)
    assert exc.value.code == "cannot-read-import"
    assert c.run_text(text, start, end, base_dir=VALID.parent) == c.run_file(
        VALID, start, end
    )


def test_run_error_propagates_slug():
    with pytest.raises(c.CycloError) as exc:
        c.run_file(
            INVALID / "bad_integer-out-of-range.cyclo", "2026-01-01", "2026-01-02"
        )
    assert exc.value.code == "integer-out-of-range"
