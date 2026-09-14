//! Python-биндинги движка Cyclorithm (PyO3).
//!
//! Тонкий слой над `cyclorithm-core`: `check` (валидация) и `run`
//! (окно событий). Контракт API — `docs/ecosystem/python.md`.
//!
//! Конвейер дословно повторяет `setup!` бинаря `cyclo`
//! (`crates/cyclorithm-cli/src/main.rs`).

use std::path::{Path, PathBuf};

use cyclorithm_core::cond::{check_conditions, resolve_units, Value};
use cyclorithm_core::datetime::{format_datetime, parse_cli_datetime};
use cyclorithm_core::expand::{Event, expand};
use cyclorithm_core::imports::{ImportError, collect_units};
use cyclorithm_core::validate::{check_bounds, check_recursion, check_tables, validate_names};
use pyo3::prelude::*;

pyo3::create_exception!(_cyclorithm, CycloError, pyo3::exceptions::PyException);

/// Внутренний провал конвейера: ошибка ядра (со слагом)
/// или ошибка парсера (текст pest / импортированного файла как есть).
enum Fail {
    Core(cyclorithm_core::Error),
    Syntax(String),
}

impl From<ImportError> for Fail {
    fn from(err: ImportError) -> Self {
        match err {
            ImportError::Coded(e) => Fail::Core(e),
            ImportError::Syntax(text) => Fail::Syntax(text),
        }
    }
}

impl From<Fail> for PyErr {
    fn from(fail: Fail) -> PyErr {
        match fail {
            Fail::Core(e) => CycloError::new_err((e.code.to_owned(), e.message)),
            Fail::Syntax(text) => CycloError::new_err(("syntax".to_owned(), text)),
        }
    }
}

/// Общий проход parse → reverse → импорты → объявления → решётка главы ошибок.
/// Продолжение `$then` выполняется в той же области видимости (таблицы
/// заимствуют локальные данные — вернуть их наружу нельзя, как в `setup!` CLI).
macro_rules! setup {
    ($text:expr, $base:expr, $ast:ident, $tables:ident, $defs:ident, $then:block) => {{
        let mut src =
            cyclorithm_parser::parse($text).map_err(|e| Fail::Syntax(e.to_string()))?;
        cyclorithm_core::reverse::materialize_reverse(&mut src.schedule)
            .map_err(Fail::Core)?;
        let $ast = &src.schedule;
        let mut groups =
            collect_units(&src.uses, $base, &mut |p| std::fs::read_to_string(p))
                .map_err(Fail::from)?;
        groups.push(src.decls.clone());
        let ($defs, reg) = resolve_units(&groups).map_err(Fail::Core)?;
        let $tables = validate_names($ast, &reg)
            .and_then(|t| check_recursion($ast, &t).map(|()| t))
            .and_then(|t| check_tables($ast, &t).map(|()| t))
            .and_then(|t| check_bounds($ast, &t).map(|()| t))
            .and_then(|t| check_conditions($ast, &$defs, &t).map(|()| t))
            .map_err(Fail::Core)?;
        $then
    }};
}

/// Текущий момент (UTC, наивный): мс epoch. Якорь относительных дат CLI
/// (`+1d` в `--start` — от now, в `--end` — от старта).
fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// Валидация программы: молча OK или `Fail`.
fn check_inner(text: &str, base: &Path) -> Result<(), Fail> {
    setup!(text, base, _ast, _tables, _defs, { Ok(()) })
}

/// Окно событий: JSON-строка того же объекта, что CLI печатает в stdout
/// (`schedule/start/end/events`).
fn run_inner(text: &str, base: &Path, start_raw: &str, end_raw: &str) -> Result<String, Fail> {
    setup!(text, base, ast, tables, defs, {
        let start_ms = parse_cli_datetime(start_raw, now_ms()).map_err(Fail::Core)?;
        let end_ms = parse_cli_datetime(end_raw, start_ms).map_err(Fail::Core)?;
        let events = expand(ast, &tables, &defs, start_ms, end_ms).map_err(Fail::Core)?;
        let out = serde_json::json!({
            "schedule": ast.name,
            "start": start_raw,
            "end": end_raw,
            "events": events.iter().map(event_json).collect::<Vec<_>>(),
        });
        Ok(out.to_string())
    })
}

/// Событие в JSON-объект (ключи — как в `docs/reference/output.md`).
fn event_json(e: &Event) -> serde_json::Value {
    serde_json::json!({
        "time": format_datetime(e.time),
        "action": e.action,
        "point": e.point,
        "point_attrs": attrs_json(&e.point_attrs),
        "action_attrs": attrs_json(&e.action_attrs),
    })
}

/// Словарь атрибутов в JSON-объект (порядок ключей — порядок объявления).
fn attrs_json(pairs: &[(String, Value)]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert(k.clone(), value_json(v));
    }
    serde_json::Value::Object(m)
}

fn value_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Num(n) => (*n).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Str(s) => s.clone().into(),
        Value::Bool(b) => (*b).into(),
        Value::Map(pairs) => attrs_json(pairs),
        Value::Array(xs) => xs.iter().map(value_json).collect(),
    }
}

/// Версия ядра (синхронна с версией workspace).
#[pyfunction]
fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Валидация текста программы (`base` — директория для `use`).
#[pyfunction]
#[pyo3(signature = (text, base))]
fn check_text(py: Python<'_>, text: &str, base: PathBuf) -> PyResult<()> {
    py.detach(|| check_inner(text, &base))
        .map_err(PyErr::from)
}

/// Окно событий текстом программы; возврат — JSON-строка объекта CLI.
#[pyfunction]
#[pyo3(signature = (text, start, end, base))]
fn run_text(
    py: Python<'_>,
    text: &str,
    start: &str,
    end: &str,
    base: PathBuf,
) -> PyResult<String> {
    py.detach(|| run_inner(text, &base, start, end))
        .map_err(PyErr::from)
}

#[pymodule]
fn _cyclorithm(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(core_version, m)?)?;
    m.add_function(wrap_pyfunction!(check_text, m)?)?;
    m.add_function(wrap_pyfunction!(run_text, m)?)?;
    m.add("CycloError", m.py().get_type::<CycloError>())?;
    Ok(())
}
