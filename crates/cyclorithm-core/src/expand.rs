//! Развёртка решётки `root_cycle` в плоский список событий (§4 спеки).
//!
//! Экземпляры `T(k) = T0 + k·P`, `k = 0, 1, …`; строки выполняются
//! в момент «запуск объемлющего + смещение», вызовы циклов разворачиваются
//! рекурсивно с накоплением смещений. В вывод попадают события
//! с `time ∈ [start, end)`; сортировка — `(time, k, порядок объявления)`,
//! дубликаты сохраняются.
//!
//! Строится с `k_min = max(0, ⌈(start − S − T0)/P⌉)`, пока `T(k) < end`,
//! где `S` — фактическая длительность корня. Вызывать после полной
//! валидации (`validate_names`, `check_recursion`, `check_bounds`):
//! рекурсивные цепочки здесь зациклили бы развёртку.

use cyclorithm_parser::{Invocation, Schedule};

use crate::cond::{eval_cond, Defs};
use crate::datetime::parse_datetime;
use crate::duration::{duration_ms, effective_offset_ms, root_period_ms};
use crate::validate::{chain, root_actual_ms, NameTables};
use crate::Error;

/// Событие вывода (§2, §6): время — `i64` мс epoch, остальное — имена
/// из исходника. Вектор от `expand` уже упорядочен.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub time: i64,
    pub point: String,
    pub action: String,
}

/// Развернуть расписание на окне `[start_ms, end_ms)`.
/// `end <= start` — не ошибка: пустой вектор.
pub fn expand(
    schedule: &Schedule,
    tables: &NameTables<'_>,
    defs: &Defs,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<Event>, Error> {
    let t0 = parse_datetime(&schedule.root.start_time)?;
    let period = root_period_ms(&schedule.root)?;
    let horizon = root_actual_ms(schedule, tables)?;

    // i128: около лимита i64 разности платежа не должны паниковать.
    let start = start_ms as i128;
    let end = end_ms as i128;
    let t0 = t0 as i128;
    let period = period as i128;
    let horizon = horizon as i128;

    // k_min = max(0, ceil((start − S − T0)/P)).
    let mut k = 0.max(ceil_div(start - horizon - t0, period));
    let mut raw: Vec<RawEvent> = Vec::new();
    let mut seq: usize = 0;
    // Период влезает в i64: пришёл из root_period_ms.
    let period_ms = period as i64;
    while t0 + k * period < end {
        let base = t0 + k * period;
        for st in &schedule.root.stmts {
            let offset = effective_offset_ms(st, period_ms, &schedule.root.duration.raw)?;
            unfold_stmt(
                st,
                Frame {
                    base,
                    offset,
                    limit: period_ms,
                    limit_raw: &schedule.root.duration.raw,
                    k,
                },
                tables,
                defs,
                &mut raw,
                &mut seq,
            )?;
        }
        k += 1;
    }
    raw.retain(|e| e.time >= start && e.time < end);
    raw.sort_by_key(|a| (a.time, a.k, a.seq));
    Ok(raw
        .into_iter()
        .map(|e| Event {
            // Время внутри окна из i64-дат — преобразование точно.
            time: e.time as i64,
            point: e.point,
            action: e.action,
        })
        .collect())
}

/// Потолок деления при положительном делителе.
fn ceil_div(a: i128, p: i128) -> i128 {
    debug_assert!(p > 0);
    -((-a).div_euclid(p))
}

/// Сырое событие до фильтра и сортировки: `k` — экземпляр корня,
/// `seq` — глобальный порядок объявления при обходе.
struct RawEvent {
    time: i128,
    k: i128,
    seq: usize,
    point: String,
    action: String,
}

/// Кадр развёртки строки: база родителя, эффективное смещение,
/// длительность родителя для цепочек и номер экземпляра корня.
struct Frame<'a> {
    base: i128,
    offset: i64,
    limit: i64,
    limit_raw: &'a str,
    k: i128,
}

/// Развёртка строки: цепочка экземпляров по `chain` (валидация уже прошла,
/// счёт конечен). Порядок обхода задаёт `seq` для сортировки.
fn unfold_stmt(
    stmt: &cyclorithm_parser::Stmt,
    frame: Frame<'_>,
    tables: &NameTables<'_>,
    defs: &Defs,
    out: &mut Vec<RawEvent>,
    seq: &mut usize,
) -> Result<(), Error> {
    let (count, step) = chain(stmt, frame.offset, frame.limit, frame.limit_raw, tables)?;
    for i in 0..count {
        let base = frame.base + frame.offset as i128 + i as i128 * step as i128;
        if let Some(cond) = &stmt.condition {
            let at = i64::try_from(base).unwrap_or(i64::MAX);
            if !eval_cond(cond, at, defs)? {
                continue;
            }
        }
        unfold(&stmt.invocation, base, frame.k, tables, defs, out, seq)?;
    }
    Ok(())
}

/// Рекурсивная развёртка вызова с накопленной базой времени.
fn unfold(
    invocation: &Invocation,
    base: i128,
    k: i128,
    tables: &NameTables<'_>,
    defs: &Defs,
    out: &mut Vec<RawEvent>,
    seq: &mut usize,
) -> Result<(), Error> {
    match invocation {
        Invocation::PointAction { point, action } => {
            out.push(RawEvent {
                time: base,
                k,
                seq: *seq,
                point: point.clone(),
                action: action.clone(),
            });
            *seq += 1;
            Ok(())
        }
        Invocation::CycleCall { name } => {
            let cycle = tables
                .cycles
                .get(name.as_str())
                .expect("имена уже проверены");
            let limit = duration_ms(&cycle.duration)?;
            for st in &cycle.stmts {
                let offset = effective_offset_ms(st, limit, &cycle.duration.raw)?;
                unfold_stmt(
                    st,
                    Frame {
                        base,
                        offset,
                        limit,
                        limit_raw: &cycle.duration.raw,
                        k,
                    },
                    tables,
                    defs,
                    out,
                    seq,
                )?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cond::{check_conditions, resolve_units, Defs};
    use crate::datetime::{format_datetime, parse_datetime};
    use crate::validate::{check_bounds, check_recursion, validate_names};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn setup(
        src: &str,
    ) -> (
        &'static cyclorithm_parser::Schedule,
        NameTables<'static>,
        &'static Defs,
    ) {
        let file: &'static cyclorithm_parser::SourceFile =
            Box::leak(Box::new(cyclorithm_parser::parse(src).unwrap()));
        let ast: &'static cyclorithm_parser::Schedule = &file.schedule;
        let t = validate_names(ast).unwrap();
        check_recursion(ast, &t).unwrap();
        check_bounds(ast, &t).unwrap();
        // Импорты — из памяти: route_lib.cyclo лежит рядом с route.cyclo.
        // Без `use` чтение не вызывается, остальные фикстуры не меняются.
        let libs: HashMap<PathBuf, String> = HashMap::from([(
            PathBuf::from("route_lib.cyclo"),
            include_str!("../../../examples/valid/route_lib.cyclo").to_owned(),
        )]);
        let mut groups = crate::imports::collect_units(&file.uses, Path::new(""), &mut |p| {
            libs.get(p)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "нет в памяти"))
        })
        .expect("импорты тестов обязаны разрешаться");
        groups.push(file.decls.clone());
        let d: &'static Defs = Box::leak(Box::new(resolve_units(&groups).unwrap()));
        check_conditions(ast, d).unwrap();
        (ast, t, d)
    }

    fn window(start: &str, end: &str) -> (i64, i64) {
        (parse_datetime(start).unwrap(), parse_datetime(end).unwrap())
    }

    fn times(events: &[Event]) -> Vec<String> {
        events.iter().map(|e| format_datetime(e.time)).collect()
    }

    #[test]
    fn expands_route_like_expected_json() {
        // Контракт §1: пятница 09.01 — полное расписание, 18 событий как в JSON.
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-09T00:00:00", "2026-01-10T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        let got: Vec<(String, String, String)> = events
            .iter()
            .map(|ev| {
                (
                    format_datetime(ev.time),
                    ev.action.clone(),
                    ev.point.clone(),
                )
            })
            .collect();
        let day =
            |t: &str, a: &str, p: &str| (format!("2026-01-09T{t}"), a.to_owned(), p.to_owned());
        assert_eq!(
            got,
            vec![
                day("06:00:00", "depart", "DEPOT"),
                day("06:40:00", "arrive", "AIRPORT"),
                day("06:50:00", "depart", "AIRPORT"),
                day("07:20:00", "arrive", "DEPOT"),
                day("10:00:00", "depart", "DEPOT"),
                day("10:20:00", "arrive", "DEPOT"),
                day("10:20:00", "depart", "DEPOT"),
                day("10:40:00", "arrive", "DEPOT"),
                day("14:00:00", "depart", "DEPOT"),
                day("14:20:00", "arrive", "DEPOT"),
                day("14:20:00", "depart", "DEPOT"),
                day("14:40:00", "arrive", "DEPOT"),
                day("14:40:00", "depart", "DEPOT"),
                day("15:00:00", "arrive", "DEPOT"),
                day("18:00:00", "depart", "DEPOT"),
                day("18:40:00", "arrive", "AIRPORT"),
                day("18:50:00", "depart", "AIRPORT"),
                day("19:20:00", "arrive", "DEPOT"),
            ]
        );
    }

    #[test]
    fn empty_window_and_window_before_anchor() {
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        // end <= start — пусто без ошибки.
        let (s, e) = window("2026-01-11T00:00:00", "2026-01-10T00:00:00");
        assert_eq!(expand(ast, &t, d, s, e).unwrap(), vec![]);
        // Окно целиком до start_time — пусто.
        let (s, e) = window("2025-12-30T00:00:00", "2025-12-31T00:00:00");
        assert_eq!(expand(ast, &t, d, s, e).unwrap(), vec![]);
        // Окно встык к границе экземпляра: событие на end не входит.
        let (s, e) = window("2026-01-10T06:00:00", "2026-01-10T06:00:00");
        assert_eq!(expand(ast, &t, d, s, e).unwrap(), vec![]);
    }

    #[test]
    fn keeps_duplicates_and_declaration_order() {
        // Две одинаковые строки: дубликаты сохраняются, порядок — объявления.
        let src = "schedule \"T\" { point A { actions = [x, y]; } \
            cycle R duration = 1h { 0m: A.x(); 0m: A.y(); 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(times(&events), vec!["2026-01-01T06:00:00"; 3]);
        let actions: Vec<&str> = events.iter().map(|ev| ev.action.as_str()).collect();
        assert_eq!(actions, vec!["x", "y", "x"]);
    }

    #[test]
    fn orders_by_time_then_instance() {
        // Событие на стыке: конец экземпляра k (смещение = периоду, встык
        // валидно) и старт экземпляра k+1 — одно время, решает k.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 1h { 0m: A.x(); 60m: A.x(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-01T02:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        // k=0: 00:00, 01:00; k=1: 01:00, 02:00(исключено концом окна).
        assert_eq!(
            times(&events),
            vec![
                "2026-01-01T00:00:00",
                "2026-01-01T01:00:00",
                "2026-01-01T01:00:00",
            ]
        );
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn negative_offsets_match_positive_equivalents() {
        // `-10m ≡ 70m`, `-0m ≡ 80m`: развёртка негативной программы дословно
        // совпадает с позитивной (порядок — по объявлению, сортировка та же).
        let neg = "schedule \"T\" { point DEPOT { actions = [depart, arrive]; } \
            point AIRPORT { actions = [arrive, depart]; } \
            cycle CITY_ROUTE duration = 1h20m { 0m: DEPOT.depart(); 40m: AIRPORT.arrive(); -10m: AIRPORT.depart(); -0m: DEPOT.arrive(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: CITY_ROUTE(); } }";
        let pos = neg
            .replace("-10m: AIRPORT.depart()", "70m: AIRPORT.depart()")
            .replace("-0m: DEPOT.arrive()", "80m: DEPOT.arrive()");
        let (an, tn, dn) = setup(neg);
        let (ap, tp, dp) = setup(Box::leak(pos.into_boxed_str()));
        let (s, e) = window("2026-01-10T00:00:00", "2026-01-11T00:00:00");
        assert_eq!(
            expand(an, &tn, dn, s, e).unwrap(),
            expand(ap, &tp, dp, s, e).unwrap()
        );
        assert_eq!(
            times(&expand(an, &tn, dn, s, e).unwrap()),
            vec![
                "2026-01-10T06:00:00",
                "2026-01-10T06:40:00",
                "2026-01-10T07:10:00",
                "2026-01-10T07:20:00",
            ]
        );
    }

    #[test]
    fn expands_repeat_chains() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); 80m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: repeat 2 R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e).unwrap()),
            vec![
                "2026-01-01T06:00:00",
                "2026-01-01T07:20:00",
                "2026-01-01T07:20:00",
                "2026-01-01T08:40:00",
            ]
        );
    }

    #[test]
    fn expands_fill_until_horizon() {
        // Горизонт 12h, шаг 80m: 9 экземпляров, следующий (12:00) не влез.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: fill until 12h R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(events.len(), 9);
        assert_eq!(times(&events)[8], "2026-01-01T10:40:00");
    }

    #[test]
    fn fill_until_minus_zero_matches_fill() {
        let base = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); 40m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { REPL } }";
        let fill = base.replace("REPL", "0h: fill R();");
        let until = base.replace("REPL", "0h: fill until -0m R();");
        let (af, tf, df) = setup(Box::leak(fill.into_boxed_str()));
        let (au, tu, du) = setup(Box::leak(until.into_boxed_str()));
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let fe = expand(af, &tf, df, s, e).unwrap();
        assert_eq!(fe, expand(au, &tu, du, s, e).unwrap());
        assert_eq!(fe.len(), 36);
    }

    #[test]
    fn skips_rows_with_false_condition() {
        // 2026-01-01T06:00:00 = 1767247200000 мс epoch.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [at >= 1767247200000] 6h: R(); [at < 1767290400000] 18h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e).unwrap()),
            vec!["2026-01-01T06:00:00"]
        );
    }

    #[test]
    fn false_parent_kills_nested_events() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle INNER duration = 1h { 0m: A.x(); } \
            cycle OUTER duration = 2h { 0m: INNER(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [at < 0] 6h: OUTER(); 6h: OUTER(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e).unwrap()),
            vec!["2026-01-01T06:00:00"]
        );
    }

    #[test]
    fn condition_filters_repeat_instances() {
        // `fill` без условия дал бы 18 экземпляров; условие режет все после 01:20.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [at < 1767231000000] 0h: fill R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e).unwrap()),
            vec!["2026-01-01T00:00:00", "2026-01-01T01:20:00"]
        );
    }

    #[test]
    fn user_predicate_in_cycle_table_filters_launches() {
        // Свой предикат внутри таблицы: `at` — абсолютное время строки,
        // поэтому рейс в 6:00 теряет строку, а рейс в 8:00 — нет.
        let src = "pred rush(at) = hour(at) == 8; \
            schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); [rush(at)] 30m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            6h: R(); 8h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e).unwrap()),
            vec![
                "2026-01-01T06:00:00",
                "2026-01-01T08:00:00",
                "2026-01-01T08:30:00"
            ]
        );
    }

    #[test]
    fn builds_only_covering_instances() {
        // Окно внутри второго периода: строится ровно экземпляр k=1.
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-02T06:30:00", "2026-01-02T07:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(
            times(&events),
            vec!["2026-01-02T06:40:00", "2026-01-02T06:50:00"]
        );
    }
}
