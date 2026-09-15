//! C ABI поверх `cyclorithm_core::pipeline` для C/C++ потребителей.
//!
//! Граница — строки: вход/выход UTF-8, окно — JSON-объект как в `cyclo run`.
//! Контракт API — `docs/ecosystem/cpp.md` и `include/cyclorithm.h`.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

use cyclorithm_core::datetime::parse_cli_datetime_zoned;
use cyclorithm_core::pipeline::{PipelineError, check_source, event_to_json_zoned, expand_window};

/// Результат вызова (см. `cyclorithm.h`): успех — `json != NULL`;
/// ошибка — `json == NULL`, `code`/`message` заполнены.
/// Всё не-NULL освобождается одним `cyclo_result_free`.
#[repr(C)]
pub struct CycloResult {
    pub json: *mut c_char,
    pub code: *mut c_char,
    pub message: *mut c_char,
}

/// Указатель-владелец Rust-строки для C (освобождение — `cyclo_result_free`).
fn into_c(text: &str) -> *mut c_char {
    CString::new(text)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn ok_json(value: serde_json::Value) -> CycloResult {
    CycloResult {
        json: into_c(&value.to_string()),
        code: std::ptr::null_mut(),
        message: std::ptr::null_mut(),
    }
}

fn err(code: &str, message: &str) -> CycloResult {
    CycloResult {
        json: std::ptr::null_mut(),
        code: into_c(code),
        message: into_c(message),
    }
}

fn err_pipeline(e: PipelineError) -> CycloResult {
    err(e.code(), e.message())
}

/// C-строка в `&str`: NULL или не-UTF-8 — `None`.
fn c_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: контракт заголовка — валидный NUL-терминированный указатель,
    // живёт весь вызов; NULL отсечён выше.
    unsafe { CStr::from_ptr(ptr).to_str().ok() }
}

/// Текущий момент (UTC, наивный): мс epoch. Якорь относительных дат
/// (`+1d` в старте — от now, в конце — от старта), как в CLI.
fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// Валидация текста программы (`base` — директория для `use`).
///
/// # Safety
///
/// Указатели обязаны быть не-NULL и указывать на валидные NUL-терминированные
/// UTF-8 C-строки, живые весь вызов.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cyclo_check(text: *const c_char, base: *const c_char) -> CycloResult {
    let (text, base) = match c_str(text).zip(c_str(base)) {
        Some((t, b)) => (t, Path::new(b)),
        None => return err("syntax", "null or non-utf8 input"),
    };
    match check_source(text, base) {
        Ok(()) => ok_json(serde_json::json!({"ok": true})),
        Err(e) => err_pipeline(e),
    }
}

/// Окно событий: JSON-строка объекта CLI (`schedule/start/end/events`).
/// Даты — короткие формы CLI.
///
/// # Safety
///
/// Как в `cyclo_check`: все четыре указателя — живые NUL-терминированные строки.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cyclo_run(
    text: *const c_char,
    base: *const c_char,
    start: *const c_char,
    end: *const c_char,
) -> CycloResult {
    let (text, base, start_raw, end_raw) = match c_str(text)
        .zip(c_str(base))
        .zip(c_str(start))
        .zip(c_str(end))
    {
        Some((((t, b), s), e)) => (t, Path::new(b), s, e),
        None => return err("syntax", "null or non-utf8 input"),
    };
    let (start_ms, zone) = match parse_cli_datetime_zoned(start_raw, now_ms()) {
        Ok(v) => v,
        Err(e) => return err_pipeline(PipelineError::Core(e)),
    };
    let (end_ms, _) = match parse_cli_datetime_zoned(end_raw, start_ms) {
        Ok(v) => v,
        Err(e) => return err_pipeline(PipelineError::Core(e)),
    };
    match expand_window(text, base, start_ms, end_ms) {
        Ok(window) => {
            let effective = zone.or(window.file_zone);
            ok_json(serde_json::json!({
                "schedule": window.schedule,
                "start": start_raw,
                "end": end_raw,
                "events": window.events.iter().map(|e| event_to_json_zoned(e, effective)).collect::<Vec<_>>(),
            }))
        }
        Err(e) => err_pipeline(e),
    }
}

/// Освобождение всех не-NULL полей результата (и только их).
///
/// # Safety
///
/// Каждое не-NULL поле обязано быть указателем, выданным `into_c`
/// (владеющий `CString`), и освобождаться ровно один раз здесь.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cyclo_result_free(r: CycloResult) {
    for ptr in [r.json, r.code, r.message] {
        if !ptr.is_null() {
            // SAFETY: указатель выдан `into_c` (владеющий `CString`),
            // освобождается ровно один раз здесь.
            unsafe {
                drop(CString::from_raw(ptr));
            }
        }
    }
}

/// Версия ядра (синхронна с версией workspace). Статическая NUL-строка,
/// освобождать не нужно.
#[unsafe(no_mangle)]
pub extern "C" fn cyclo_version() -> *const c_char {
    static VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
    VERSION.as_ptr().cast()
}
