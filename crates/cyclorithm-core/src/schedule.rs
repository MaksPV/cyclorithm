//! Тонкий фасад для встраивания (WASM-плейграунд): один вызов
//! «исходник → JSON §6», без файловой системы.
//!
//! Конвейер повторяет `cyclo run` 1:1 (порядок фаз — по §5):
//! разбор → импорты (`E13`/`E14`) → объявления → имена (`E04`/`E09`,
//! затем `E01`/`E02`/`E03`) → рекурсия (`E06`) → границы (`E05`/`E10`/`E07`)
//! → условия (`E11`/`E12`) → даты окна (`E08`) → развёртка.
//! Тексты ошибок совпадают со stderr CLI дословно.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cond::{check_conditions, resolve_units};
use crate::datetime::{format_datetime, parse_datetime};
use crate::expand::expand;
use crate::imports::{collect_units, ImportError};
use crate::validate::{check_bounds, check_recursion, validate_names};
use crate::Error;

/// Диагностика для редактора: что сломалось и где (если позиция известна).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// `parse` — синтаксис (без E-кода), `import` — `E13`/`E14`,
    /// `valid` — остальные коды §5.
    pub kind: &'static str,
    /// Код из §5; у синтаксиса — `None`.
    pub code: Option<&'static str>,
    /// 1-базные строка/колонка; только у синтаксиса
    /// (позиций в AST нет, E-ошибки их не несут).
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
                    "E13" | "E14" => "import",
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

/// Общий конвейер `run_schedule`/`run_timeline`: имя расписания и события.
/// Порядок фаз — как в `cyclo run` (§5).
fn pipeline(
    src: &str,
    start_raw: &str,
    end_raw: &str,
    libs: &[(&str, &str)],
) -> Result<(String, Vec<crate::expand::Event>), Diag> {
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
    let defs = resolve_units(&groups).map_err(Diag::valid)?;
    let tables = validate_names(ast)
        .and_then(|t| check_recursion(ast, &t).map(|()| t))
        .and_then(|t| check_bounds(ast, &t).map(|()| t))
        .and_then(|t| check_conditions(ast, &defs).map(|()| t))
        .map_err(Diag::valid)?;
    let start_ms = parse_datetime(start_raw).map_err(Diag::valid)?;
    let end_ms = parse_datetime(end_raw).map_err(Diag::valid)?;
    let events = expand(ast, &tables, &defs, start_ms, end_ms).map_err(Diag::valid)?;
    Ok((ast.name.clone(), events))
}

/// Развернуть расписание из строки в JSON §6 (компактный, ключи
/// `schedule,start,end,events`, у событий — `time,action,point`;
/// без завершающего `\n`, в отличие от stdout CLI).
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
        out.push_str("{\"time\":");
        out.push_str(&esc(&format_datetime(e.time)));
        out.push_str(",\"action\":");
        out.push_str(&esc(&e.action));
        out.push_str(",\"point\":");
        out.push_str(&esc(&e.point));
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
        out.push_str("{\"time\":");
        out.push_str(&esc(&format_datetime(e.time)));
        out.push_str(",\"action\":");
        out.push_str(&esc(&e.action));
        out.push_str(",\"point\":");
        out.push_str(&esc(&e.point));
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

fn span_json(s: &crate::expand::Span) -> String {
    format!(
        "{{\"cycle\":{},\"start\":{},\"end\":{}}}",
        esc(&s.cycle),
        esc(&format_datetime(s.start)),
        esc(&format_datetime(s.end)),
    )
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
             {\"time\":\"2026-01-09T06:00:00\",\"action\":\"depart\",\"point\":\"DEPOT\"},\
             {\"time\":\"2026-01-09T06:20:00\",\"action\":\"arrive\",\"point\":\"DEPOT\"}]}"
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
    fn facade_reports_e11() {
        let src = MINI.replace("[not weekend(at)]", "[banana(at)]");
        let d = run_schedule(&src, "2026-01-09T00:00:00", "2026-01-10T00:00:00", &[]).unwrap_err();
        assert_eq!((d.kind, d.code), ("valid", Some("E11")));
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
            "\"point\":\"A\",\"span\":\
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
        // Без библиотеки — E13, с ней — успех.
        let d = run_schedule(&src, "2026-01-09T00:00:00", "2026-01-10T00:00:00", &[]).unwrap_err();
        assert_eq!((d.kind, d.code), ("import", Some("E13")));
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
