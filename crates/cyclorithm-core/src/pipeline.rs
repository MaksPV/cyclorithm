//! Общий конвейер parse → reverse → импорты → объявления → решётка → развёртка.
//!
//! Единственная реализация пути, который раньше жил копиями в `setup!`
//! бинаря `cyclo` и Python-биндинга: края (CLI, Python, C++) вызывают
//! эти функции и добавляют только свой ввод/вывод и коды возврата.

use std::fmt;
use std::path::Path;

use crate::Error;
use crate::cond::Value;
use crate::datetime::format_datetime_tz;
use crate::engine::{EngineError, with_validated_fs};
use crate::expand::{Event, expand, next_events};
use crate::imports::ImportError;

/// Провал конвейера: ошибка ядра (со слагом главы ошибок)
/// или ошибка парсера (текст pest / импортированного файла как есть, без слага).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    Core(Error),
    Syntax(String),
}

impl PipelineError {
    /// Слаг ситуации (`"cycle-overruns"`, …; парсер — `"syntax"`).
    pub fn code(&self) -> &str {
        match self {
            PipelineError::Core(e) => e.code,
            PipelineError::Syntax(_) => "syntax",
        }
    }

    /// Текст без слага, дословно по таблице главы ошибок (парсер — текст pest).
    pub fn message(&self) -> &str {
        match self {
            PipelineError::Core(e) => &e.message,
            PipelineError::Syntax(text) => text,
        }
    }
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for PipelineError {}

impl From<ImportError> for PipelineError {
    fn from(err: ImportError) -> Self {
        match err {
            ImportError::Coded(e) => PipelineError::Core(e),
            ImportError::Syntax(text) => PipelineError::Syntax(text),
        }
    }
}

fn engine_to_pipeline(e: EngineError) -> PipelineError {
    match e {
        EngineError::Syntax(text, _) => PipelineError::Syntax(text),
        EngineError::Core(err) => PipelineError::Core(err),
        EngineError::Import(imp) => imp.into(),
    }
}

/// Валидация программы (`base` — директория для `use`).
pub fn check_source(text: &str, base: &Path) -> Result<(), PipelineError> {
    with_validated_fs(text, base, |_ast, _tables, _defs, _file_zone| Ok(()))
        .map_err(engine_to_pipeline)
}

/// Окно развёртки: имя расписания (для конверта CLI) и события.
pub struct Window {
    /// Имя `schedule` программы.
    pub schedule: String,
    /// События окна в порядке `(time, k, порядок объявления)`.
    pub events: Vec<Event>,
    /// Зона файла (`None` — без `timezone`), для fallback'а когда окно наивное.
    pub file_zone: Option<i16>,
}

/// Окно событий в миллисекундах epoch (наивных, как внутри движка).
/// `query_zone` — зона окна (`start`/`from`): кадр наивных дат файла,
/// когда в файле нет `timezone` (см. docs/reference/semantics.md).
pub fn expand_window(
    text: &str,
    base: &Path,
    start_ms: i64,
    end_ms: i64,
    query_zone: Option<i16>,
) -> Result<Window, PipelineError> {
    with_validated_fs(text, base, |ast, tables, defs, file_zone| {
        let events = expand(ast, tables, defs, start_ms, end_ms, query_zone)?;
        Ok((ast.name.clone(), events, file_zone))
    })
    .map(|(schedule, events, file_zone)| Window {
        schedule,
        events,
        file_zone,
    })
    .map_err(engine_to_pipeline)
}

/// Первые `n` событий от `from_ms` в пределах `within_ms`.
/// `query_zone` — зона окна (`from`): кадр наивных дат файла,
/// когда в файле нет `timezone` (см. docs/reference/semantics.md).
pub fn next_window(
    text: &str,
    base: &Path,
    from_ms: i64,
    within_ms: i64,
    n: usize,
    query_zone: Option<i16>,
) -> Result<Window, PipelineError> {
    with_validated_fs(text, base, |ast, tables, defs, file_zone| {
        let events = next_events(ast, tables, defs, from_ms, within_ms, n, query_zone)?;
        Ok((ast.name.clone(), events, file_zone))
    })
    .map(|(schedule, events, file_zone)| Window {
        schedule,
        events,
        file_zone,
    })
    .map_err(engine_to_pipeline)
}

/// Событие в JSON-объект (ключи — как в `docs/reference/output.md`;
/// порядок ключей словарей — порядок объявления). Без зоны — наивное.
pub fn event_to_json(e: &Event) -> serde_json::Value {
    event_to_json_zoned(e, None)
}

/// Событие в JSON-объект с зоной окна (`None` — наивное, `Some(0)` → `Z`).
pub fn event_to_json_zoned(e: &Event, zone: Option<i16>) -> serde_json::Value {
    serde_json::json!({
        "time": format_datetime_tz(e.time, zone),
        "action": e.action,
        "point": e.point,
        "point_attrs": attrs_json(&e.point_attrs),
        "action_attrs": attrs_json(&e.action_attrs),
    })
}

/// Словарь атрибутов в JSON-объект (порядок ключей — порядок объявления:
/// `preserve_order` в `Cargo.toml` сохраняет порядок вставки).
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
