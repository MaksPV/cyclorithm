//! Проверка имён: дубликаты (E04) и разрешение вызовов (E01/E02/E03/E09).
//!
//! Порядок проверок: сначала все объявления (E04, E09 на столкновение
//! имён точки и цикла), затем все вызовы в порядке объявления
//! (циклы по порядку, потом `root_cycle`). Первая ошибка побеждает.
//!
//! Правило общего пространства (§3): одно имя не может обозначать и точку,
//! и цикл — нарушение E09. На вызове: точка как цикл —
//! `point 'X' is not a cycle`, цикл как точка — `cycle 'X' is not a point`.

use std::collections::{HashMap, HashSet};

use cycloritm_parser::{Invocation, Repeat, Schedule, Stmt};

use crate::duration::{duration_ms, effective_offset_ms, format_duration, root_period_ms};
use crate::Error;

/// Таблицы имён после успешной проверки — вход later-фаз ядра.
#[derive(Debug)]
pub struct NameTables<'a> {
    /// Точка → её объявление (список `actions`).
    pub points: HashMap<&'a str, &'a cycloritm_parser::Point>,
    /// Цикл → его объявление.
    pub cycles: HashMap<&'a str, &'a cycloritm_parser::Cycle>,
}

/// Проверить объявления и вызовы. Возвращает таблицы имён либо первую ошибку.
pub fn validate_names(schedule: &Schedule) -> Result<NameTables<'_>, Error> {
    let mut points = HashMap::new();
    for p in &schedule.points {
        if points.contains_key(p.name.as_str()) {
            return Err(Error::e04("point", &p.name));
        }
        points.insert(p.name.as_str(), p);
    }
    let mut cycles = HashMap::new();
    for c in &schedule.cycles {
        if cycles.contains_key(c.name.as_str()) {
            return Err(Error::e04("cycle", &c.name));
        }
        if points.contains_key(c.name.as_str()) {
            return Err(Error::e09_not_cycle(&c.name));
        }
        cycles.insert(c.name.as_str(), c);
    }
    let tables = NameTables { points, cycles };
    for c in &schedule.cycles {
        check_stmts(&tables, &c.stmts)?;
    }
    check_stmts(&tables, &schedule.root.stmts)?;
    Ok(tables)
}

/// Проверить вызовы списка строк в порядке объявления.
fn check_stmts(tables: &NameTables<'_>, stmts: &[cycloritm_parser::Stmt]) -> Result<(), Error> {
    for st in stmts {
        match &st.invocation {
            Invocation::PointAction { point, action } => {
                if let Some(p) = tables.points.get(point.as_str()) {
                    if !p.actions.iter().any(|a| a == action) {
                        return Err(Error::e02(action, point));
                    }
                } else if tables.cycles.contains_key(point.as_str()) {
                    return Err(Error::e09_not_point(point));
                } else {
                    return Err(Error::e01(point));
                }
            }
            Invocation::CycleCall { name } => {
                if tables.cycles.contains_key(name.as_str()) {
                    // ok
                } else if tables.points.contains_key(name.as_str()) {
                    return Err(Error::e09_not_cycle(name));
                } else {
                    return Err(Error::e03(name));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Рекурсия (E06) и границы циклов (E07).
// Вызывать после `validate_names`: обе функции предполагают, что все имена
// разрешены (неизвестных циклов уже нет).
// ---------------------------------------------------------------------------

/// Запрет самовызовов циклов — прямых и через цепочку (E06).
/// Обход в порядке объявления; сообщается повторно вошедший цикл.
pub fn check_recursion(schedule: &Schedule, tables: &NameTables<'_>) -> Result<(), Error> {
    let mut gray: HashSet<&str> = HashSet::new();
    let mut black: HashSet<&str> = HashSet::new();
    for c in &schedule.cycles {
        visit_cycle(c.name.as_str(), tables, &mut gray, &mut black)?;
    }
    Ok(())
}

/// DFS по графу вызовов циклов. Серая вершина при повторном входе — E06.
fn visit_cycle<'a>(
    name: &'a str,
    tables: &NameTables<'a>,
    gray: &mut HashSet<&'a str>,
    black: &mut HashSet<&'a str>,
) -> Result<(), Error> {
    if black.contains(name) {
        return Ok(());
    }
    if !gray.insert(name) {
        return Err(Error::e06(name));
    }
    let cycle = tables.cycles.get(name).expect("имена уже проверены");
    for st in &cycle.stmts {
        if let Invocation::CycleCall { name: callee } = &st.invocation {
            visit_cycle(callee.as_str(), tables, gray, black)?;
        }
    }
    gray.remove(name);
    black.insert(name);
    Ok(())
}

/// Граница циклов (E07, правило 8): `actual(C) ≤ duration(C)` для каждого
/// цикла и `root_cycle`. Отрицательные смещения разрешены заранее
/// (`duration(C) − X`); вылет ниже нуля — тоже E07
/// (`offset '-2h' out of bounds (duration 1h20m)`), проверяется в порядке
/// объявления и побеждает сразу. В сообщении о переполнении — вызов
/// со строки, давшей максимум (при равных концах — первая в порядке объявления).
/// Вызывать после `validate_names` и `check_recursion`.
pub fn check_bounds(schedule: &Schedule, tables: &NameTables<'_>) -> Result<(), Error> {
    for c in &schedule.cycles {
        let limit = duration_ms(&c.duration)?;
        let (end, argmax) = stmts_end(&c.stmts, limit, &c.duration.raw, tables)?;
        if end > limit {
            let row = argmax.expect("конец больше лимита — строка-аргмакс есть");
            return Err(blame(&c.stmts[row], &c.name, end, limit));
        }
    }
    let period = root_period_ms(&schedule.root)?;
    let (end, argmax) = stmts_end(
        &schedule.root.stmts,
        period,
        &schedule.root.duration.raw,
        tables,
    )?;
    if end > period {
        let row = argmax.expect("конец больше лимита — строка-аргмакс есть");
        return Err(blame(&schedule.root.stmts[row], "root_cycle", end, period));
    }
    Ok(())
}

/// Фактическая длительность именованного цикла (§2):
/// `max(o + длина вызова)` по строкам, где `o` — эффективное смещение
/// (отрицательные уже разрешены через длительность цикла); длина — `0`
/// для действия точки, объявленная длительность для вызова цикла;
/// пустой цикл — `0`.
/// Нужна решётке (§4) как горизонт занятости `S`.
/// Рекурсии здесь нет: берутся только объявленные длительности.
pub fn actual_ms(name: &str, tables: &NameTables<'_>) -> Result<i64, Error> {
    let cycle = tables.cycles.get(name).expect("имена уже проверены");
    let limit = duration_ms(&cycle.duration)?;
    Ok(stmts_end(&cycle.stmts, limit, &cycle.duration.raw, tables)?.0)
}

/// Фактическая длительность `root_cycle` — горизонт занятости `S` (§4).
pub fn root_actual_ms(schedule: &Schedule, tables: &NameTables<'_>) -> Result<i64, Error> {
    let period = duration_ms(&schedule.root.duration)?;
    Ok(stmts_end(
        &schedule.root.stmts,
        period,
        &schedule.root.duration.raw,
        tables,
    )?
    .0)
}

/// Конец занятого отрезка списка строк и индекс строки-аргмакса
/// (при равных концах — первой). Пустой список — `(0, None)`.
/// `limit`/`limit_raw` — объявленная длительность непосредственно объемлющего
/// цикла: через неё разрешаются отрицательные смещения и горизонты.
fn stmts_end(
    stmts: &[Stmt],
    limit: i64,
    limit_raw: &str,
    tables: &NameTables<'_>,
) -> Result<(i64, Option<usize>), Error> {
    let mut best: (i64, Option<usize>) = (0, None);
    for (i, st) in stmts.iter().enumerate() {
        let offset = effective_offset_ms(st, limit, limit_raw)?;
        let end = row_end(st, offset, limit, limit_raw, tables)?;
        if best.1.is_none() || end > best.0 {
            best = (end, Some(i));
        }
    }
    Ok(best)
}

/// Конец одной строки: смещение + длина вызова или цепочки.
/// Порядок проверок строки: E10, затем E07 (горизонт, конец цепочки).
fn row_end(
    st: &Stmt,
    offset: i64,
    limit: i64,
    limit_raw: &str,
    tables: &NameTables<'_>,
) -> Result<i64, Error> {
    let (count, step) = chain(st, offset, limit, limit_raw, tables)?;
    Ok(saturating_add_mul(offset, count, step))
}

/// Параметры цепочки строки: число экземпляров и шаг стыковки.
/// `Once` — `(1, длина вызова)`; дальше всё считается одинаково.
/// Та же функция кормит развёртку (`expand`).
pub fn chain(
    st: &Stmt,
    offset: i64,
    limit: i64,
    limit_raw: &str,
    tables: &NameTables<'_>,
) -> Result<(u64, i64), Error> {
    match &st.repeat {
        Repeat::Once => {
            let span = match &st.invocation {
                Invocation::PointAction { .. } => 0,
                Invocation::CycleCall { name } => {
                    duration_ms(&cycle_duration(tables, name).duration)?
                }
            };
            Ok((1, span))
        }
        Repeat::Times(raw) => {
            let n: u64 = raw.parse().map_err(|_| Error::e10_repeat_count(raw))?;
            if n == 0 {
                return Err(Error::e10_repeat_count(raw));
            }
            Ok((n, step_of(&st.invocation, tables)?))
        }
        Repeat::Fill { until } => {
            let step = step_of(&st.invocation, tables)?;
            if step == 0 {
                return Err(match &st.invocation {
                    Invocation::CycleCall { name } => Error::e10_fill_zero(name),
                    Invocation::PointAction { action, .. } => Error::e10_repeat_action(action),
                });
            }
            let horizon = match until {
                None => limit,
                Some(u) => {
                    let t = duration_ms(&u.duration)?;
                    let h = if u.negative {
                        if t > limit {
                            return Err(Error::e07_until(&u.raw(), limit_raw));
                        }
                        limit - t
                    } else {
                        t
                    };
                    if h > limit {
                        return Err(Error::e07_until(&u.raw(), limit_raw));
                    }
                    h
                }
            };
            let n = if horizon - offset >= step {
                ((horizon - offset) / step) as u64
            } else {
                0
            };
            Ok((n, step))
        }
    }
}

/// Длительность шага цепочки: `0` для действия точки,
/// объявленная длительность для вызова цикла.
/// Повтор действия точки — E10.
fn step_of(invocation: &Invocation, tables: &NameTables<'_>) -> Result<i64, Error> {
    match invocation {
        Invocation::PointAction { action, .. } => Err(Error::e10_repeat_action(action)),
        Invocation::CycleCall { name } => duration_ms(&cycle_duration(tables, name).duration),
    }
}

/// Объявление вызываемого цикла (имена уже проверены).
fn cycle_duration<'a>(tables: &NameTables<'a>, name: &str) -> &'a cycloritm_parser::Cycle {
    tables.cycles.get(name).expect("имена уже проверены")
}

/// `offset + n*d` с насыщением: переполнение всё равно больше лимита.
fn saturating_add_mul(offset: i64, n: u64, d: i64) -> i64 {
    let end = offset as i128 + n as i128 * d as i128;
    i64::try_from(end).unwrap_or(i64::MAX)
}

/// Ошибка E07 с виной на вызове строки: цикл — `cycle 'X' overruns ...`,
/// действие точки — симметричное `action 'a' overruns ...`.
fn blame(stmt: &Stmt, outer: &str, end: i64, limit: i64) -> Error {
    let excess = format_duration(end - limit);
    let end_s = format_duration(end);
    let limit_s = format_duration(limit);
    match &stmt.invocation {
        Invocation::PointAction { action, .. } => {
            Error::e07_action(action, outer, &excess, &end_s, &limit_s)
        }
        Invocation::CycleCall { name } => Error::e07_cycle(name, outer, &excess, &end_s, &limit_s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(src: &str) -> cycloritm_parser::Schedule {
        cycloritm_parser::parse(src).expect("фикстура обязана разбираться")
    }

    fn err(src: &str) -> Error {
        let ast = parsed(src);
        validate_names(&ast).expect_err("ожидалась ошибка имён")
    }

    #[test]
    fn accepts_route() {
        let src = include_str!("../../../examples/route.cyclo");
        let ast = parsed(src);
        let tables = validate_names(&ast).expect("route обязан проходить проверку имён");
        assert_eq!(tables.points.len(), 2);
        assert_eq!(tables.cycles.len(), 1);
    }

    #[test]
    fn error_codes_match_fixtures() {
        // (файл, код, сообщение) — дословно по §5.
        for (file, src, code, message) in [
            (
                "bad_e01",
                include_str!("../../../examples/bad_e01.cyclo"),
                "E01",
                "unknown point 'PORT'",
            ),
            (
                "bad_e02",
                include_str!("../../../examples/bad_e02.cyclo"),
                "E02",
                "action 'arrive' not allowed for point 'DEPOT'",
            ),
            (
                "bad_e03",
                include_str!("../../../examples/bad_e03.cyclo"),
                "E03",
                "unknown cycle 'NIGHT_ROUTE'",
            ),
            (
                "bad_e04",
                include_str!("../../../examples/bad_e04.cyclo"),
                "E04",
                "duplicate point 'DEPOT'",
            ),
            (
                "bad_e09",
                include_str!("../../../examples/bad_e09.cyclo"),
                "E09",
                "point 'DEPOT' is not a cycle",
            ),
        ] {
            let e = err(src);
            assert_eq!(e.code, code, "для {file}");
            assert_eq!(e.message, message, "для {file}");
        }
    }

    #[test]
    fn rejects_duplicate_cycle() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            cycle R duration = 2h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let e = err(src);
        assert_eq!((e.code, e.message.as_str()), ("E04", "duplicate cycle 'R'"));
    }

    #[test]
    fn rejects_point_cycle_name_clash() {
        // Общее пространство имён (§3): имя не может быть и точкой, и циклом.
        let src = "schedule \"T\" { point R { actions = [x]; } \
            cycle R duration = 1h { 0m: R.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E09", "point 'R' is not a cycle")
        );
    }

    #[test]
    fn rejects_cycle_used_as_point() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R.depart(); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E09", "cycle 'R' is not a point")
        );
    }

    /// Разобранная фикстура + таблицы имён. `Box::leak` — тестовый приём,
    /// чтобы таблицы жили `'static` рядом со своим AST.
    fn tables(src: &str) -> (&'static cycloritm_parser::Schedule, NameTables<'static>) {
        let ast: &'static cycloritm_parser::Schedule = Box::leak(Box::new(parsed(src)));
        let t = validate_names(ast).expect("имена обязаны проходить");
        (ast, t)
    }

    #[test]
    fn recursion_and_bounds_match_fixtures() {
        for (file, src, code, message) in [
            (
                "bad_e06",
                include_str!("../../../examples/bad_e06.cyclo"),
                "E06",
                "recursive cycle 'A'",
            ),
            (
                "bad_e07",
                include_str!("../../../examples/bad_e07.cyclo"),
                "E07",
                "cycle 'CYCLE2' overruns 'CYCLE1' by 20m (80m > 60m)",
            ),
        ] {
            let (ast, t) = tables(src);
            let e = check_recursion(ast, &t)
                .and_then(|()| check_bounds(ast, &t))
                .expect_err("ожидалась E06/E07");
            assert_eq!(e.code, code, "для {file}");
            assert_eq!(e.message, message, "для {file}");
        }
    }

    #[test]
    fn accepts_route_recursion_and_bounds() {
        let src = include_str!("../../../examples/route.cyclo");
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("route без рекурсии");
        check_bounds(ast, &t).expect("route в границах");
    }

    #[test]
    fn rejects_chain_recursion() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle A1 duration = 1h { 0m: B1(); } \
            cycle B1 duration = 1h { 0m: A1(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: A1(); } }";
        let (ast, t) = tables(src);
        let e = check_recursion(ast, &t).expect_err("цепочка — тоже рекурсия");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E06", "recursive cycle 'A1'")
        );
    }

    #[test]
    fn accepts_nested_bounds() {
        // Вложенность встык валидна: 30m + 1h = 90m ≤ 2h.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle INNER duration = 1h { 0m: A.x(); } \
            cycle OUTER duration = 2h { 30m: INNER(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: OUTER(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_bounds(ast, &t).expect("всё в границах");
    }

    #[test]
    fn rejects_point_action_overrun() {
        // Мгновенное событие за границей периода — тоже E07.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 61m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("событие за границей");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E07", "action 'x' overruns 'R' by 1m (61m > 60m)")
        );
    }

    #[test]
    fn rejects_root_overrun() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 1h { 30m: R(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("вылез за период");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E07", "cycle 'R' overruns 'root_cycle' by 30m (90m > 60m)")
        );
    }

    #[test]
    fn actual_duration_matches_spec_formula() {
        // actual(C) = max(o + D): D = 0 для точки, declared для цикла.
        let (_, t) = tables(include_str!("../../../examples/route.cyclo"));
        assert_eq!(actual_ms("CITY_ROUTE", &t), Ok(4_800_000));
        // Многорядный цикл: max(10m+50m, 90m+5m) = 95m ≤ 100m — валидно.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle BIG duration = 50m { 0m: A.x(); } \
            cycle SMALL duration = 5m { 0m: A.x(); } \
            cycle FOO duration = 100m { 10m: BIG(); 90m: SMALL(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: FOO(); } }";
        let (ast2, t2) = tables(src);
        assert_eq!(actual_ms("FOO", &t2), Ok(5_700_000));
        check_bounds(ast2, &t2).expect("FOO в границах");
    }

    #[test]
    fn actual_of_empty_cycle_is_zero() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle EMPTY duration = 1h { } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: EMPTY(); } }";
        let (_ast, t) = tables(src);
        assert_eq!(actual_ms("EMPTY", &t), Ok(0));
    }

    #[test]
    fn rejects_negative_out_of_bounds() {
        // -2h при duration = 1h20m → эффективное -40m: E07, сообщение по §5.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); -2h: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("вылет ниже нуля");
        assert_eq!(e.code, "E07");
        assert_eq!(
            e.message.as_str(),
            "offset '-2h' out of bounds (duration 1h20m)"
        );
    }

    #[test]
    fn accepts_negative_edges() {
        // -0m ≡ конец (встык валидно), -1h20m ≡ 0m (эффективный ноль валиден).
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { -1h20m: A.x(); -10m: A.x(); -0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_bounds(ast, &t).expect("границы и стыки валидны");
        assert_eq!(actual_ms("R", &t), Ok(4_800_000));
    }

    #[test]
    fn accepts_negative_in_zero_duration_cycle() {
        // Нулевой цикл: -0m даёт эффективное 0 ∈ [0, 0] — валидно без исключений.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle EMPTY duration = 0m { -0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: EMPTY(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_bounds(ast, &t).expect("-0m в нулевом цикле валидно");
        assert_eq!(actual_ms("EMPTY", &t), Ok(0));
    }

    #[test]
    fn negative_overrun_still_blamed() {
        // Эффективное смещение + длина вызова за границей — обычный E07.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle INNER duration = 40m { 0m: A.x(); } \
            cycle OUTER duration = 1h { -10m: INNER(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: OUTER(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("50m + 40m = 90m > 60m");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E07", "cycle 'INNER' overruns 'OUTER' by 30m (90m > 60m)")
        );
    }

    #[test]
    fn blames_argmax_row() {
        // Две вылезающие строки: вина на большей (второй), не на первой.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle A1 duration = 2h { 0m: A.x(); } \
            cycle B1 duration = 2h { 0m: A.x(); } \
            cycle OUTER duration = 1h { 0m: A1(); 10m: B1(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: OUTER(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("обе строки вылезают");
        // Концы: 0+120m=120m и 10m+120m=130m; вина на B1.
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E07", "cycle 'B1' overruns 'OUTER' by 70m (130m > 60m)")
        );
    }

    fn bounds_err(src: &str) -> Error {
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_bounds(ast, &t).expect_err("ожидалась ошибка границ")
    }

    #[test]
    fn rejects_repeat_zero() {
        let e = bounds_err(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: repeat 0 R(); } }",
        );
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E10", "invalid repeat count '0'")
        );
    }

    #[test]
    fn rejects_repeat_of_point_action() {
        let e = bounds_err(
            "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: repeat 3 A.x(); } }",
        );
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E10", "repeat of point action 'x' not allowed")
        );
    }

    #[test]
    fn rejects_fill_of_zero_duration_cycle() {
        let e = bounds_err(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle EMPTY duration = 0m { } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: fill EMPTY(); } }",
        );
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E10", "fill of zero-duration cycle 'EMPTY'")
        );
    }

    #[test]
    fn rejects_repeat_chain_overrun() {
        // 23h + 2*80m = 25:40 > 24h.
        let e = bounds_err(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 23h: repeat 2 R(); } }",
        );
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                "E07",
                "cycle 'R' overruns 'root_cycle' by 100m (1540m > 1440m)"
            )
        );
    }

    #[test]
    fn rejects_until_beyond_parent() {
        let e = bounds_err(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: fill until 30h R(); } }",
        );
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E07", "until '30h' out of bounds (duration 24h)")
        );
    }

    #[test]
    fn rejects_until_below_zero() {
        let e = bounds_err(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: fill until -30h R(); } }",
        );
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E07", "until '-30h' out of bounds (duration 24h)")
        );
    }

    #[test]
    fn accepts_chains_in_bounds() {
        // repeat встык (6h + 3*80m = 10h), fill с хвостом, until встык.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            6h: repeat 3 R(); 0h: fill R(); 0h: fill until 12h R(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_bounds(ast, &t).expect("цепочки в границах");
        assert_eq!(root_actual_ms(ast, &t), Ok(86_400_000));
    }
}
