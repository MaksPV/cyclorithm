//! Тонкий фасад для встраивания (WASM-плейграунд): один вызов
//! «исходник → JSON §6», без файловой системы.
//!
//! Конвейер повторяет `cyclo run` 1:1 (порядок фаз — по главе ошибок):
//! разбор → импорты (`cannot-read-import`/`schedule-in-import`) → объявления
//! → имена (`duplicate`/`wrong-kind`, затем `unknown-point`/`action-not-allowed`/
//! `unknown-cycle`) → рекурсия (`recursive`) → таблицы (`unknown-table`,
//! границы таблиц и тел) → границы (`invalid-duration`/`invalid-repeat-count`/
//! `cycle-overruns`) → условия (`unknown-name`/`type-mismatch`…) → даты окна
//! (`invalid-datetime`) → развёртка.
//! Тексты ошибок совпадают со stderr CLI дословно.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cond::{check_conditions, resolve_units, Defs, Value};
use crate::datetime::{format_datetime, parse_datetime};
use crate::expand::{expand, next_events, Event};
use crate::imports::{collect_units, ImportError};
use crate::validate::{check_bounds, check_recursion, check_tables, validate_names, NameTables};
use crate::Error;
use cyclorithm_parser::Schedule;

/// Диагностика для редактора: что сломалось и где (если позиция известна).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// `parse` — синтаксис (без слага), `import` — `cannot-read-import`/
    /// `schedule-in-import`/`import-cycle`, `valid` — остальные слаги.
    pub kind: &'static str,
    /// Слаг ситуации; у синтаксиса — `None`.
    pub code: Option<&'static str>,
    /// 1-базные строка/колонка; только у синтаксиса
    /// (позиций в AST нет, ошибки валидации их не несут).
    pub line: Option<usize>,
    pub col: Option<usize>,
    /// Текст как в stderr CLI.
    pub text: String,
}

impl Diag {
    fn parse(text: String, line: Option<usize>, col: Option<usize>) -> Self {
        Self {
            kind: "parse",
            code: None,
            line,
            col,
            text,
        }
    }

    fn valid(e: Error) -> Self {
        Self {
            kind: "valid",
            code: Some(e.code),
            line: None,
            col: None,
            text: e.to_string(),
        }
    }

    fn import(e: ImportError) -> Self {
        match e {
            ImportError::Coded(e) => {
                let kind = match e.code {
                    "cannot-read-import" | "import-cycle" | "schedule-in-import" => "import",
                    _ => "valid",
                };
                Self {
                    kind,
                    code: Some(e.code),
                    line: None,
                    col: None,
                    text: e.to_string(),
                }
            }
            ImportError::Syntax(s) => Self {
                kind: "parse",
                code: None,
                line: None,
                col: None,
                text: s,
            },
        }
    }
}

/// Общий setup фаз главы ошибок для фасадов: разбор → импорты → объявления → решётка.
/// Даты окон и развёртка — в замыкании вызывателя (заимствования живут
/// внутри: вернуть их наружу нельзя, поэтому общий код — через замыкание).
fn with_setup<R>(
    src: &str,
    libs: &[(&str, &str)],
    f: impl FnOnce(&Schedule, &NameTables<'_>, &Defs) -> Result<R, Diag>,
) -> Result<R, Diag> {
    let file = match cyclorithm_parser::parse(src) {
        Ok(f) => f,
        Err(e) => {
            let (line, col) = cyclorithm_parser::error_position(&e);
            return Err(Diag::parse(e.to_string(), Some(line), Some(col)));
        }
    };
    let ast = &file.schedule;
    let mem: HashMap<PathBuf, &str> = libs
        .iter()
        .map(|(name, text)| (PathBuf::from(name), *text))
        .collect();
    let mut groups = collect_units(&file.uses, Path::new(""), &mut |p| {
        mem.get(p)
            .copied()
            .map(str::to_owned)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "нет в памяти"))
    })
    .map_err(Diag::import)?;
    groups.push(file.decls.clone());
    let (defs, reg) = resolve_units(&groups).map_err(Diag::valid)?;
    let tables = validate_names(ast, &reg)
        .and_then(|t| check_recursion(ast, &t).map(|()| t))
        .and_then(|t| check_tables(ast, &t).map(|()| t))
        .and_then(|t| check_bounds(ast, &t).map(|()| t))
        .and_then(|t| check_conditions(ast, &defs, &t).map(|()| t))
        .map_err(Diag::valid)?;
    f(ast, &tables, &defs)
}

/// Общий конвейер `run_schedule`/`run_timeline`: имя расписания и события.
/// Порядок фаз — как в `cyclo run` (глава ошибок).
fn pipeline(
    src: &str,
    start_raw: &str,
    end_raw: &str,
    libs: &[(&str, &str)],
) -> Result<(String, Vec<Event>), Diag> {
    with_setup(src, libs, |ast, tables, defs| {
        let start_ms = parse_datetime(start_raw).map_err(Diag::valid)?;
        let end_ms = parse_datetime(end_raw).map_err(Diag::valid)?;
        let events = expand(ast, tables, defs, start_ms, end_ms).map_err(Diag::valid)?;
        Ok((ast.name.clone(), events))
    })
}

/// Развернуть расписание из строки в JSON §6 (компактный, ключи
/// `schedule,start,end,events`, у событий — `time,action,point`,
/// `point_attrs`,`action_attrs`; без завершающего `\n`, в отличие от stdout CLI).
/// `libs` — содержимое библиотек для `use`: `(путь, текст)`, путь пишется
/// как в `use`, относительно корня (`libs/holidays.cyclo`).
pub fn run_schedule(
    src: &str,
    start_raw: &str,
    end_raw: &str,
    libs: &[(&str, &str)],
) -> Result<String, Diag> {
    let (name, events) = pipeline(src, start_raw, end_raw, libs)?;
    let mut out = String::from("{\"schedule\":");
    out.push_str(&esc(&name));
    out.push_str(",\"start\":");
    out.push_str(&esc(start_raw));
    out.push_str(",\"end\":");
    out.push_str(&esc(end_raw));
    out.push_str(",\"events\":[");
    for (i, e) in events.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        push_event_fields(&mut out, e);
        out.push('}');
    }
    out.push_str("]}");
    Ok(out)
}

/// Первые `n` событий от `from_ms` (включительно) в пределах
/// `[from_ms, from_ms + within_ms)` — JSON §6 (компактный, ключи
/// `schedule,from,within,events`; `from` — резолвленная ISO-строка,
/// `within` — миллисекунды числом; без завершающего `\n`).
/// Пусто — `"events":[]`. Ошибки строк за пределами ответа не срабатывают.
pub fn next_steps(
    src: &str,
    from_ms: i64,
    within_ms: i64,
    n: usize,
    libs: &[(&str, &str)],
) -> Result<String, Diag> {
    let (name, events) = with_setup(src, libs, |ast, tables, defs| {
        let events = next_events(ast, tables, defs, from_ms, within_ms, n).map_err(Diag::valid)?;
        Ok((ast.name.clone(), events))
    })?;
    let mut out = String::from("{\"schedule\":");
    out.push_str(&esc(&name));
    out.push_str(",\"from\":");
    out.push_str(&esc(&format_datetime(from_ms)));
    out.push_str(",\"within\":");
    out.push_str(&within_ms.to_string());
    out.push_str(",\"events\":[");
    for (i, e) in events.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        push_event_fields(&mut out, e);
        out.push('}');
    }
    out.push_str("]}");
    Ok(out)
}

/// Развернуть расписание для таймлайна: как `run_schedule`, плюс
/// `spans` — различные спаны событий окна (имя цикла и границы ISO),
/// упорядочены по `(start, cycle)`. У событий — дополнительное поле
/// `span` с их спаном. CLI не меняется: это API встраивания.
pub fn run_timeline(
    src: &str,
    start_raw: &str,
    end_raw: &str,
    libs: &[(&str, &str)],
) -> Result<String, Diag> {
    let (name, events) = pipeline(src, start_raw, end_raw, libs)?;
    let mut out = String::from("{\"schedule\":");
    out.push_str(&esc(&name));
    out.push_str(",\"start\":");
    out.push_str(&esc(start_raw));
    out.push_str(",\"end\":");
    out.push_str(&esc(end_raw));
    out.push_str(",\"events\":[");
    for (i, e) in events.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        push_event_fields(&mut out, e);
        out.push_str(",\"span\":");
        out.push_str(&span_json(&e.span));
        out.push('}');
    }
    out.push_str("],\"spans\":[");
    let mut spans: Vec<&crate::expand::Span> = events.iter().map(|e| &e.span).collect();
    spans.sort_by(|a, b| (a.start, &a.cycle, a.end).cmp(&(b.start, &b.cycle, b.end)));
    spans.dedup();
    for (i, s) in spans.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&span_json(s));
    }
    out.push_str("]}");
    Ok(out)
}

/// Поля события в JSON-объект без скобок (порядок — §6:
/// `time,action,point,point_attrs,action_attrs`).
fn push_event_fields(out: &mut String, e: &Event) {
    out.push_str("\"time\":");
    out.push_str(&esc(&format_datetime(e.time)));
    out.push_str(",\"action\":");
    out.push_str(&esc(&e.action));
    out.push_str(",\"point\":");
    out.push_str(&esc(&e.point));
    out.push_str(",\"point_attrs\":");
    out.push_str(&attrs_text(&e.point_attrs));
    out.push_str(",\"action_attrs\":");
    out.push_str(&attrs_text(&e.action_attrs));
}

fn span_json(s: &crate::expand::Span) -> String {
    format!(
        "{{\"cycle\":{},\"start\":{},\"end\":{}}}",
        esc(&s.cycle),
        esc(&format_datetime(s.start)),
        esc(&format_datetime(s.end)),
    )
}

/// Словарь атрибутов в JSON-объект (порядок ключей — порядок объявления).
fn attrs_text(pairs: &[(String, Value)]) -> String {
    let mut o = String::from("{");
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&esc(k));
        o.push(':');
        o.push_str(&value_text(v));
    }
    o.push('}');
    o
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Num(n) => n.to_string(),
        Value::Str(s) => esc(s),
        Value::Bool(b) => b.to_string(),
        Value::Map(pairs) => attrs_text(pairs),
        Value::Array(xs) => {
            let mut o = String::from("[");
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                o.push_str(&value_text(x));
            }
            o.push(']');
            o
        }
    }
}

/// JSON-строка с экранированием (без внешних зависимостей — важно для WASM).
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI: &str = r#"schedule "Тест" {
  point DEPOT {
    actions = [depart, arrive];
  }

  cycle HOP
    duration = 20m
  {
    0m: DEPOT.depart();
    20m: DEPOT.arrive();
  }

  root_cycle
    start_time = "2026-01-01T00:00:00",
    duration = 24h
  {
    [not weekend(at)] 6h: HOP();
  }
}"#;

    #[test]
    fn facade_matches_cli_json_shape() {
        let got = run_schedule(MINI, "2026-01-09T00:00:00", "2026-01-10T00:00:00", &[]).unwrap();
        assert_eq!(
            got,
            "{\"schedule\":\"Тест\",\
             \"start\":\"2026-01-09T00:00:00\",\
             \"end\":\"2026-01-10T00:00:00\",\
             \"events\":[\
             {\"time\":\"2026-01-09T06:00:00\",\"action\":\"depart\",\"point\":\"DEPOT\",\
              \"point_attrs\":{},\"action_attrs\":{}},\
             {\"time\":\"2026-01-09T06:20:00\",\"action\":\"arrive\",\"point\":\"DEPOT\",\
              \"point_attrs\":{},\"action_attrs\":{}}]}"
        );
    }

    #[test]
    fn facade_reports_parse_diag_with_position() {
        let d = run_schedule(
            "schedule {",
            "2026-01-09T00:00:00",
            "2026-01-10T00:00:00",
            &[],
        )
        .unwrap_err();
        assert_eq!(d.kind, "parse");
        assert_eq!(d.code, None);
        // Ошибка на `{` (позиция отказа, не начало файла).
        assert_eq!((d.line, d.col), (Some(1), Some(10)));
    }

    #[test]
    fn facade_reports_unknown_name() {
        let src = MINI.replace("[not weekend(at)]", "[banana(at)]");
        let d = run_schedule(&src, "2026-01-09T00:00:00", "2026-01-10T00:00:00", &[]).unwrap_err();
        assert_eq!((d.kind, d.code), ("valid", Some("unknown-name")));
    }

    #[test]
    fn timeline_carries_event_spans_and_distinct_list() {
        // HOP — один экземпляр [06:00,06:20); прямое действие — root_cycle.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle HOP duration = 20m { 0m: A.x(); 20m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 6h: HOP(); 8h: A.x(); } }";
        let got = run_timeline(src, "2026-01-01T00:00:00", "2026-01-02T00:00:00", &[]).unwrap();
        assert!(got.contains(
            "\"point\":\"A\",\"point_attrs\":{},\"action_attrs\":{},\"span\":\
             {\"cycle\":\"HOP\",\"start\":\"2026-01-01T06:00:00\",\"end\":\"2026-01-01T06:20:00\"}"
        ));
        assert!(got.contains(
            "\"spans\":[\
             {\"cycle\":\"root_cycle\",\"start\":\"2026-01-01T00:00:00\",\"end\":\"2026-01-02T00:00:00\"},\
             {\"cycle\":\"HOP\",\"start\":\"2026-01-01T06:00:00\",\"end\":\"2026-01-01T06:20:00\"}]"
        ));
    }

    #[test]
    fn facade_resolves_use_from_memory() {
        let src =
            "use \"lib.cyclo\";\n".to_owned() + &MINI.replace("[not weekend(at)]", "[early(at)]");
        let lib = "pred early(at) = hour(at) < 12;";
        // Без библиотеки — cannot-read-import, с ней — успех.
        let d = run_schedule(&src, "2026-01-09T00:00:00", "2026-01-10T00:00:00", &[]).unwrap_err();
        assert_eq!((d.kind, d.code), ("import", Some("cannot-read-import")));
        let got = run_schedule(
            &src,
            "2026-01-09T00:00:00",
            "2026-01-10T00:00:00",
            &[("lib.cyclo", lib)],
        )
        .unwrap();
        assert!(got.contains("2026-01-09T06:00:00"));
    }
}
