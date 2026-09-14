//! Python-биндинги движка Cyclorithm (PyO3).
//!
//! Тонкий слой над `cyclorithm-core`: `check` (валидация) и `run`
//! (окно событий). Контракт API — `docs/ecosystem/python.md`.

use pyo3::prelude::*;

/// Версия ядра (синхронна с версией workspace).
#[pyfunction]
fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pymodule]
fn _cyclorithm(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(core_version, m)?)?;
    Ok(())
}
