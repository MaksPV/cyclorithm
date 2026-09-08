//! Бинарь `cyclo` — тонкая обёртка над парсером и ядром (контракт §1 спеки).
//!
//! - `cyclo run FILE --start T --end T` → один JSON-объект в stdout, код 0;
//! - любая ошибка ввода/валидации → текст в stderr, в stdout ничего, код 1;
//! - неверные аргументы → usage в stderr, код 2.

use cyclorithm_core::cond::{check_conditions, resolve_units};
use cyclorithm_core::datetime::{format_datetime, parse_datetime};
use cyclorithm_core::expand::expand;
use cyclorithm_core::imports::collect_units;
use cyclorithm_core::validate::{check_bounds, check_recursion, validate_names};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file, start_raw, end_raw) = match parse_args(&args) {
        Ok(v) => v,
        Err(usage) => {
            eprintln!("{usage}");
            return 2;
        }
    };
    let src = match std::fs::read_to_string(&file) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("cannot read '{file}': {e}");
            return 1;
        }
    };
    // Ошибка парсера — без E-кода (§5): текст pest как есть.
    let src = match cyclorithm_parser::parse(&src) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let ast = &src.schedule;
    // Объявления — сверху файла: их ошибки (E04/E11/E12) раньше проверок решётки.
    // Импорты (E13/E14) — раньше объявлений: склейка «импорты → программа».
    let mut groups = match collect_units(
        &src.uses,
        std::path::Path::new(&file)
            .parent()
            .unwrap_or(std::path::Path::new("")),
        &mut |p| std::fs::read_to_string(p),
    ) {
        Ok(groups) => groups,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    groups.push(src.decls.clone());
    let defs = match resolve_units(&groups) {
        Ok(defs) => defs,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let tables = match validate_names(ast)
        .and_then(|t| check_recursion(ast, &t).map(|()| t))
        .and_then(|t| check_bounds(ast, &t).map(|()| t))
        .and_then(|t| check_conditions(ast, &defs).map(|()| t))
    {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    // `--start`/`--end`: битые значения — E08; в выводе — эхо как передали.
    let start_ms = match parse_datetime(&start_raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let end_ms = match parse_datetime(&end_raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let events = match expand(ast, &tables, &defs, start_ms, end_ms) {
        Ok(events) => events,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let out = serde_json::json!({
        "schedule": ast.name,
        "start": start_raw,
        "end": end_raw,
        "events": events
            .iter()
            .map(|e| serde_json::json!({
                "time": format_datetime(e.time),
                "action": e.action,
                "point": e.point,
            }))
            .collect::<Vec<_>>(),
    });
    println!("{out}");
    0
}

/// `cyclo run FILE --start T --end T`; `--start`/`--end` в любом порядке.
fn parse_args(args: &[String]) -> Result<(String, String, String), String> {
    let usage = || "usage: cyclo run FILE --start DATETIME --end DATETIME".to_owned();
    if args.first().map(String::as_str) != Some("run") {
        return Err(usage());
    }
    let file = args.get(1).ok_or_else(usage)?.clone();
    let mut start = None;
    let mut end = None;
    let mut rest = args.get(2..).ok_or_else(usage)?.iter().peekable();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(usage)?;
        match flag.as_str() {
            "--start" => start = Some(value.clone()),
            "--end" => end = Some(value.clone()),
            _ => return Err(usage()),
        }
    }
    match (start, end) {
        (Some(s), Some(e)) => Ok((file, s, e)),
        _ => Err(usage()),
    }
}
