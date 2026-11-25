//! Python-биндинги движка Cyclorithm (PyO3).
//!
//! Тонкий слой над `cyclorithm_core::pipeline`: `check` (валидация)
//! и `run` (окно событий). Контракт API — `docs/ecosystem/python.md`.

use std::path::{Path, PathBuf};

use cyclorithm_core::datetime::parse_cli_datetime_zoned;
use cyclorithm_core::pipeline::{PipelineError, check_source, event_to_json_zoned, expand_window};
use pyo3::prelude::*;

pyo3::create_exception!(_cyclorithm, CycloError, pyo3::exceptions::PyException);

/// Провал конвейера в исключение Python (чужие типы — без `impl From`
/// из-за orphan-правил, только локальная функция).
fn to_pyerr(err: PipelineError) -> PyErr {
    CycloError::new_err((err.code().to_owned(), err.message().to_owned()))
}

/// Текущий момент в системной зоне: `(мс epoch, офсет_минут)` — подпись
/// дефолтного окна (как в CLI). Без tzdata в ОС chrono отдаёт UTC (`Some(0)`).
fn now_local() -> (i64, Option<i16>) {
    let now = chrono::Local::now();
    let ms = now.timestamp_millis();
    let mins = now.offset().local_minus_utc() / 60;
    (ms, i16::try_from(mins).ok())
}

/// Валидация программы.
fn check_inner(text: &str, base: &Path) -> Result<(), PipelineError> {
    check_source(text, base)
}
/// Окно событий: JSON-строка того же объекта, что CLI печатает в stdout
/// (`schedule/start/end/events`). Зона окна — офсет `start`, иначе зона файла.
fn run_inner(
    text: &str,
    base: &Path,
    start_raw: &str,
    end_raw: &str,
) -> Result<String, PipelineError> {
    let (now, now_zone) = now_local();
    let (start_ms, zone) =
        parse_cli_datetime_zoned(start_raw, now, now_zone).map_err(PipelineError::Core)?;
    let (end_ms, _) =
        parse_cli_datetime_zoned(end_raw, start_ms, None).map_err(PipelineError::Core)?;
    let window = expand_window(text, base, start_ms, end_ms, zone)?;
    let effective = zone.or(window.file_zone);
    let out = serde_json::json!({
        "schedule": window.schedule,
        "start": start_raw,
        "end": end_raw,
        "events": window.events.iter().map(|e| event_to_json_zoned(e, effective)).collect::<Vec<_>>(),
    });
    Ok(out.to_string())
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
    py.detach(|| check_inner(text, &base)).map_err(to_pyerr)
}

/// Окно событий текстом программы; возврат — JSON-строка объекта CLI.
#[pyfunction]
#[pyo3(signature = (text, start, end, base))]
fn run_text(py: Python<'_>, text: &str, start: &str, end: &str, base: PathBuf) -> PyResult<String> {
    py.detach(|| run_inner(text, &base, start, end))
        .map_err(to_pyerr)
}

#[pymodule]
fn _cyclorithm(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(core_version, m)?)?;
    m.add_function(wrap_pyfunction!(check_text, m)?)?;
    m.add_function(wrap_pyfunction!(run_text, m)?)?;
    m.add("CycloError", m.py().get_type::<CycloError>())?;
    Ok(())
}
