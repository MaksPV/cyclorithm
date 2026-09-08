//! Резолвер `use`-импортов (§4.16 спеки). Без файловой системы: чтение
//! подставляет вызыватель (CLI), здесь — только обход, циклы и склейка.
//!
//! Порядок оверлея — обход вглубь: зависимости раньше зависимых, тело
//! программы — последним. Дубль внутри одного файла — позже отдаст `E04`,
//! одно имя в разных файлах — молча побеждает последнее.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use cycloritm_parser::Decl;

use crate::Error;

/// Неудача резолвера: код (E13/E14) или сырая ошибка разбора с путём.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Coded(Error),
    /// Синтаксис импортированного файла: `"path: <текст pest>"`, без кода.
    Syntax(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Coded(e) => write!(f, "{e}"),
            ImportError::Syntax(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Собрать группы объявлений импортов в порядке оверлея (тело программы
/// вызыватель добавляет последним сам). `read` читает файл целиком.
pub fn collect_units(
    uses: &[String],
    importer_dir: &Path,
    read: &mut dyn FnMut(&Path) -> Result<String, std::io::Error>,
) -> Result<Vec<Vec<Decl>>, ImportError> {
    let mut out = Vec::new();
    let mut done = HashSet::new();
    let mut stack = Vec::new();
    for u in uses {
        visit(u, importer_dir, read, &mut done, &mut stack, &mut out)?;
    }
    Ok(out)
}

fn visit(
    use_str: &str,
    importer_dir: &Path,
    read: &mut dyn FnMut(&Path) -> Result<String, std::io::Error>,
    done: &mut HashSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
    out: &mut Vec<Vec<Decl>>,
) -> Result<(), ImportError> {
    let full = clean_join(importer_dir, use_str);
    if stack.contains(&full) {
        return Err(ImportError::Coded(Error::e13_cycle(use_str)));
    }
    if !done.insert(full.clone()) {
        return Ok(());
    }
    let src = read(&full).map_err(|_| ImportError::Coded(Error::e13_read(use_str)))?;
    let unit = cycloritm_parser::parse_unit(&src)
        .map_err(|e| ImportError::Syntax(format!("{}: {e}", full.display())))?;
    if unit.has_schedule {
        return Err(ImportError::Coded(Error::e14_schedule(use_str)));
    }
    stack.push(full.clone());
    let dir = full.parent().unwrap_or(Path::new(""));
    for u in &unit.uses {
        visit(u, dir, read, done, stack, out)?;
    }
    stack.pop();
    out.push(unit.decls);
    Ok(())
}

/// Склейка с базовым каталогом + лексическая чистка (`.`/`..`/двойные
/// разделители). Без обращения к ФС: для детекта циклов достаточно.
fn clean_join(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    out.push(rel);
    let mut clean = PathBuf::new();
    for c in out.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            _ => clean.push(c.as_os_str()),
        }
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn mem(files: &[(&str, &str)]) -> HashMap<PathBuf, String> {
        files
            .iter()
            .map(|(k, v)| (PathBuf::from(k), v.to_string()))
            .collect()
    }

    fn collect(
        uses: &[&str],
        dir: &str,
        files: &HashMap<PathBuf, String>,
    ) -> Result<Vec<Vec<Decl>>, ImportError> {
        let owned: Vec<String> = uses.iter().map(|s| s.to_string()).collect();
        let mut files = files.clone();
        collect_units(&owned, Path::new(dir), &mut |p| {
            files
                .remove(p)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "нет"))
        })
    }

    fn names(groups: &[Vec<Decl>]) -> Vec<String> {
        groups
            .iter()
            .flat_map(|g| g.iter())
            .map(|d| match d {
                Decl::Const { name, .. } => name.clone(),
                Decl::Fun { name, .. } => name.clone(),
                Decl::Pred { name, .. } => name.clone(),
            })
            .collect()
    }

    #[test]
    fn overlays_depth_first() {
        // A → [B, C], B → [D]: D, B, C.
        let files = mem(&[
            ("/lib/b.cyclo", "use \"d.cyclo\"; const B = 1;"),
            ("/lib/d.cyclo", "const D = 1;"),
            ("/lib/c.cyclo", "const C = 1;"),
        ]);
        let groups = collect(&["b.cyclo", "c.cyclo"], "/lib", &files).unwrap();
        assert_eq!(names(&groups), vec!["D", "B", "C"]);
    }

    #[test]
    fn diamond_resolves_once() {
        let files = mem(&[
            ("/lib/b.cyclo", "use \"d.cyclo\"; const B = 1;"),
            ("/lib/c.cyclo", "use \"d.cyclo\"; const C = 1;"),
            ("/lib/d.cyclo", "const D = 1;"),
        ]);
        let groups = collect(&["b.cyclo", "c.cyclo"], "/lib", &files).unwrap();
        assert_eq!(names(&groups), vec!["D", "B", "C"]);
    }

    #[test]
    fn rejects_cycle_and_missing() {
        let files = mem(&[
            ("/lib/a.cyclo", "use \"b.cyclo\";"),
            ("/lib/b.cyclo", "use \"a.cyclo\";"),
        ]);
        let e = collect(&["a.cyclo"], "/lib", &files).expect_err("цикл — E13");
        assert_eq!(e, ImportError::Coded(Error::e13_cycle("a.cyclo")));
        let files = mem(&[]);
        let e = collect(&["nope.cyclo"], "/lib", &files).expect_err("нет файла — E13");
        assert_eq!(e, ImportError::Coded(Error::e13_read("nope.cyclo")));
    }

    #[test]
    fn rejects_schedule_inside() {
        let files = mem(&[(
            "/lib/s.cyclo",
            "const K = 1; schedule \"X\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }",
        )]);
        let e = collect(&["s.cyclo"], "/lib", &files).expect_err("расписание — E14");
        assert_eq!(e, ImportError::Coded(Error::e14_schedule("s.cyclo")));
    }

    #[test]
    fn normalizes_dot_segments() {
        // `./d.cyclo` и `sub/../d.cyclo` — один файл, ромб без дубля.
        let files = mem(&[
            ("/lib/b.cyclo", "use \"./d.cyclo\"; const B = 1;"),
            ("/lib/c.cyclo", "use \"sub/../d.cyclo\"; const C = 1;"),
            ("/lib/d.cyclo", "const D = 1;"),
        ]);
        let groups = collect(&["b.cyclo", "c.cyclo"], "/lib", &files).unwrap();
        assert_eq!(names(&groups), vec!["D", "B", "C"]);
    }
}
