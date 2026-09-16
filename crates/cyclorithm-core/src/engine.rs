//! Единый конвейер валидации (parse → reverse → imports → resolve → validate).
//! Используется всеми краями (CLI, Python, C++, WASM) — без дублирования.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::Error;
use crate::cond::{Defs, check_conditions, resolve_units};
use crate::datetime::parse_timezone;
use crate::imports::{ImportError, collect_units};
use crate::parser::{self, Schedule, SourceFile};
use crate::validate::{NameTables, check_bounds, check_recursion, check_tables, validate_names};

/// Ошибка валидации с сохранением типа для маппинга в `PipelineError`/`Diag`.
#[derive(Debug, Clone)]
pub enum EngineError {
    Syntax(String, Option<(usize, usize)>),
    Core(Error),
    Import(ImportError),
}

impl From<ImportError> for EngineError {
    fn from(e: ImportError) -> Self {
        EngineError::Import(e)
    }
}

/// Парсинг + reverse + file_zone (без импортов/валидации) — для `check` без FS.
fn parse_and_file_zone(text: &str) -> Result<(SourceFile, Option<i16>), EngineError> {
    let mut src = parser::parse(text).map_err(|e| match e {
        parser::ParseError::Syntax(se) => {
            let (l, c) = parser::error_position(&se);
            EngineError::Syntax(se.to_string(), Some((l, c)))
        }
        parser::ParseError::Coded(ce) => EngineError::Core(ce),
    })?;
    crate::reverse::materialize_reverse(&mut src.schedule).map_err(EngineError::Core)?;
    let fz = match &src.schedule.timezone {
        Some(raw) => Some(parse_timezone(raw).map_err(EngineError::Core)?),
        None => None,
    };
    Ok((src, fz))
}

/// Общий путь для FS (CLI/Python/C++): `base` — директория для `use`, чтение — `std::fs`.
pub fn with_validated_fs<F, R>(text: &str, base: &Path, f: F) -> Result<R, EngineError>
where
    F: FnOnce(&Schedule, &NameTables<'_>, &Defs, Option<i16>) -> Result<R, Error>,
{
    let (src, file_zone) = parse_and_file_zone(text)?;
    let ast = &src.schedule;
    let mut groups = collect_units(&src.uses, base, &mut |p| std::fs::read_to_string(p))?;
    groups.push(src.decls.clone());
    let (defs, reg) = resolve_units(&groups).map_err(EngineError::Core)?;
    let tables = validate_names(ast, &reg)
        .and_then(|t| check_recursion(ast, &t).map(|()| t))
        .and_then(|t| check_tables(ast, &t).map(|()| t))
        .and_then(|t| check_bounds(ast, &t).map(|()| t))
        .and_then(|t| check_conditions(ast, &defs, &t).map(|()| t))
        .map_err(EngineError::Core)?;
    f(ast, &tables, &defs, file_zone).map_err(EngineError::Core)
}

/// Общий путь для памяти (WASM/schedule): `libs` — `(&str,&str)` в памяти, база — `""`.
pub fn with_validated_mem<F, R>(text: &str, libs: &[(&str, &str)], f: F) -> Result<R, EngineError>
where
    F: FnOnce(&Schedule, &NameTables<'_>, &Defs, Option<i16>) -> Result<R, Error>,
{
    let (src, file_zone) = parse_and_file_zone(text)?;
    let ast = &src.schedule;
    let mem: HashMap<PathBuf, &str> = libs.iter().map(|(n, t)| (PathBuf::from(n), *t)).collect();
    let mut groups = collect_units(&src.uses, Path::new(""), &mut |p| {
        mem.get(p)
            .copied()
            .map(str::to_owned)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "нет в памяти"))
    })?;
    groups.push(src.decls.clone());
    let (defs, reg) = resolve_units(&groups).map_err(EngineError::Core)?;
    let tables = validate_names(ast, &reg)
        .and_then(|t| check_recursion(ast, &t).map(|()| t))
        .and_then(|t| check_tables(ast, &t).map(|()| t))
        .and_then(|t| check_bounds(ast, &t).map(|()| t))
        .and_then(|t| check_conditions(ast, &defs, &t).map(|()| t))
        .map_err(EngineError::Core)?;
    f(ast, &tables, &defs, file_zone).map_err(EngineError::Core)
}
