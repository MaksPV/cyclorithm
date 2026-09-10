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

use std::collections::HashMap;

use cyclorithm_parser::{Expr, Invocation, Schedule};

use crate::cond::{eval_cond_with_env, eval_expr_with_env, resolve_point_attrs, Defs, Value};
use crate::datetime::parse_datetime;
use crate::duration::{duration_ms, effective_offset_ms, root_period_ms};
use crate::validate::{chain, instantiate, root_actual_ms, NameTables};
use crate::Error;

/// Спан экземпляра цикла для таймлайна: имя цикла и границы
/// `[start, end)` в мс epoch (конец — по объявленной длительности).
/// У действий напрямую в `root_cycle` — `cycle: "root_cycle"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub cycle: String,
    pub start: i64,
    pub end: i64,
}

/// Событие вывода (§2, §6): время — `i64` мс epoch, остальное — имена
/// из исходника. Словари — копии атрибутов точки и значений блока строки
/// (порядок ключей — порядок объявления). Вектор от `expand` уже упорядочен.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub time: i64,
    pub point: String,
    pub action: String,
    pub point_attrs: Vec<(String, Value)>,
    pub action_attrs: Vec<(String, Value)>,
    /// Ближайший экземпляр цикла, породивший событие.
    pub span: Span,
}

/// Развернуть один экземпляр корня (`base = T0 + k·P`): строки корня
/// в свои события. Общий кусок `expand` и `next_events`: порядок обхода
/// (`seq` сквозной) и ошибки строк совпадают 1:1.
#[allow(clippy::too_many_arguments)]
fn unfold_root_instance(
    schedule: &Schedule,
    tables: &NameTables<'_>,
    defs: &Defs,
    point_attrs: &HashMap<String, Vec<(String, Value)>>,
    k: i128,
    base: i128,
    period_ms: i64,
    out: &mut Vec<RawEvent>,
    seq: &mut usize,
) -> Result<(), Error> {
    let root_span = Span {
        cycle: "root_cycle".to_owned(),
        start: clamp_i64(base),
        end: clamp_i64(base + period_ms as i128),
    };
    // Корень параметров не имеет: окружение строк — пустое.
    let root_env: HashMap<String, Value> = HashMap::new();
    let mut ctx = Ctx {
        tables,
        defs,
        point_attrs,
        out,
        seq,
    };
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
            &root_span,
            &mut ctx,
            &root_env,
        )?;
    }
    Ok(())
}

/// Сырое событие в событие вывода (время из окна i64-дат — точно).
fn finalize_event(e: RawEvent) -> Event {
    Event {
        time: e.time as i64,
        point: e.point,
        action: e.action,
        point_attrs: e.point_attrs,
        action_attrs: e.action_attrs,
        span: e.span,
    }
}

/// Удерживаемый кандидат `next_events`: худший сверху max-кучи.
struct Top {
    time: i128,
    k: i128,
    seq: usize,
    ev: RawEvent,
}

impl PartialEq for Top {
    fn eq(&self, other: &Self) -> bool {
        (self.time, self.k, self.seq) == (other.time, other.k, other.seq)
    }
}

impl Eq for Top {}

impl PartialOrd for Top {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Top {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.time, self.k, self.seq).cmp(&(other.time, other.k, other.seq))
    }
}

/// Первые `n` событий от `from_ms` (включительно) в пределах
/// `[from_ms, from_ms + within_ms)` — однопроходно по экземплярам корня
/// от того же `k_min`, что у `expand`. Каждый экземпляр разворачивается
/// ровно один раз; стоп — когда `base` превзошёл худшее удерживаемое
/// (позже стартующие события только позже) или кап. Связки одного момента
/// не склеиваются: `n` считает события. Порядок — как у `expand`.
/// Пусто (нет событий, `within < 0`, `n == 0`) — пустой вектор без ошибки.
/// Ленивость: ошибки строк за пределами ответа не срабатывают.
pub fn next_events(
    schedule: &Schedule,
    tables: &NameTables<'_>,
    defs: &Defs,
    from_ms: i64,
    within_ms: i64,
    n: usize,
) -> Result<Vec<Event>, Error> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let t0 = parse_datetime(&schedule.root.start_time)? as i128;
    let period = root_period_ms(&schedule.root)? as i128;
    let horizon = root_actual_ms(schedule, tables)? as i128;
    let point_attrs = resolve_point_attrs(schedule, defs)?;
    // Период влезает в i64: пришёл из root_period_ms.
    let period_ms = period as i64;

    let from = from_ms as i128;
    let cap = from + within_ms as i128;
    let mut k = 0.max(ceil_div(from - horizon - t0, period));
    let mut seq: usize = 0;
    let mut heap: std::collections::BinaryHeap<Top> = std::collections::BinaryHeap::new();
    let mut buf: Vec<RawEvent> = Vec::new();
    loop {
        let base = t0 + k * period;
        if base >= cap {
            break;
        }
        if heap.len() >= n {
            let worst = heap.peek().expect("куча полна").time;
            if base > worst {
                break;
            }
        }
        buf.clear();
        unfold_root_instance(
            schedule,
            tables,
            defs,
            &point_attrs,
            k,
            base,
            period_ms,
            &mut buf,
            &mut seq,
        )?;
        for ev in buf.drain(..) {
            if ev.time < from || ev.time >= cap {
                continue;
            }
            let top = Top {
                time: ev.time,
                k: ev.k,
                seq: ev.seq,
                ev,
            };
            if heap.len() < n {
                heap.push(top);
            } else if let Some(worst) = heap.peek() {
                if top < *worst {
                    heap.pop();
                    heap.push(top);
                }
            }
        }
        k += 1;
    }
    Ok(heap
        .into_sorted_vec()
        .into_iter()
        .map(|t| finalize_event(t.ev))
        .collect())
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
    // Атрибуты точек — после всех проверок §5, до первой строки.
    let point_attrs = resolve_point_attrs(schedule, defs)?;

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
    // Корень разворачивается поэкземплярно общим хелпером.
    while t0 + k * period < end {
        let base = t0 + k * period;
        unfold_root_instance(
            schedule,
            tables,
            defs,
            &point_attrs,
            k,
            base,
            period_ms,
            &mut raw,
            &mut seq,
        )?;
        k += 1;
    }
    raw.retain(|e| e.time >= start && e.time < end);
    raw.sort_by_key(|a| (a.time, a.k, a.seq));
    Ok(raw.into_iter().map(finalize_event).collect())
}

/// Потолок деления при положительном делителе.
fn ceil_div(a: i128, p: i128) -> i128 {
    debug_assert!(p > 0);
    -((-a).div_euclid(p))
}

/// Кламп i128 → i64 для границ спанов (вне окна точности не нужно).
fn clamp_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Сырое событие до фильтра и сортировки: `k` — экземпляр корня,
/// `seq` — глобальный порядок объявления при обходе.
struct RawEvent {
    time: i128,
    k: i128,
    seq: usize,
    point: String,
    action: String,
    point_attrs: Vec<(String, Value)>,
    action_attrs: Vec<(String, Value)>,
    span: Span,
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

/// Общее состояние обхода: таблицы, определения, атрибуты точек и аккумуляторы.
struct Ctx<'a, 'n, 'o, 'm> {
    tables: &'a NameTables<'n>,
    defs: &'a Defs,
    point_attrs: &'m HashMap<String, Vec<(String, Value)>>,
    out: &'o mut Vec<RawEvent>,
    seq: &'o mut usize,
}
/// Развёртка строки: цепочка экземпляров по `chain` (валидация уже прошла,
/// счёт конечен). Порядок обхода задаёт `seq` для сортировки.
/// `env` — динамическое окружение параметров цепочки вызовов.
fn unfold_stmt(
    stmt: &cyclorithm_parser::Stmt,
    frame: Frame<'_>,
    parent: &Span,
    ctx: &mut Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
) -> Result<(), Error> {
    let (count, step) = chain(stmt, frame.offset, frame.limit, frame.limit_raw, ctx.tables)?;
    for i in 0..count {
        let base = frame.base + frame.offset as i128 + i as i128 * step as i128;
        if let Some(cond) = &stmt.condition {
            let at = i64::try_from(base).unwrap_or(i64::MAX);
            if !eval_cond_with_env(cond, at, ctx.defs, env)? {
                continue;
            }
        }
        unfold(&stmt.invocation, base, frame.k, parent, ctx, env)?;
    }
    Ok(())
}

/// Рекурсивная развёртка вызова с накопленной базой времени.
/// `parent` — спан ближайшего цикла (для корня — `root_cycle`).
/// Аргументы вычисляются в окружении вызывающего (`at` — время экземпляра),
/// параметры связываются поверх него (вложенный вызов перетирает целиком);
/// вызов без аргументов окружение не меняет (течёт вниз как есть).
fn unfold(
    invocation: &Invocation,
    base: i128,
    k: i128,
    parent: &Span,
    ctx: &mut Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
) -> Result<(), Error> {
    let at = i64::try_from(base).unwrap_or(i64::MAX);
    match invocation {
        Invocation::PointAction {
            point,
            action,
            block,
        } => {
            let mut action_attrs = Vec::with_capacity(block.len());
            for (key, value) in block {
                action_attrs.push((key.clone(), eval_expr_with_env(value, at, ctx.defs, env)?));
            }
            ctx.out.push(RawEvent {
                time: base,
                k,
                seq: *ctx.seq,
                point: point.clone(),
                action: action.clone(),
                // Копия атрибутов своей точки (порядок — порядок объявления).
                point_attrs: ctx.point_attrs.get(point).cloned().unwrap_or_default(),
                action_attrs,
                span: parent.clone(),
            });
            *ctx.seq += 1;
            Ok(())
        }
        Invocation::CycleCall { name, args } => {
            if ctx.tables.routines.contains_key(name.as_str()) {
                unfold_routine(name, args, base, k, ctx, env)
            } else {
                let cycle = ctx
                    .tables
                    .cycles
                    .get(name.as_str())
                    .expect("имена уже проверены");
                // Арность уже проверена (E12): длины совпадают.
                debug_assert_eq!(cycle.params.len(), args.len());
                let mut child = env.clone();
                for (param, arg) in cycle.params.iter().zip(args.iter()) {
                    child.insert(param.clone(), eval_expr_with_env(arg, at, ctx.defs, env)?);
                }
                let limit = duration_ms(&cycle.duration)?;
                let child_span = Span {
                    cycle: name.clone(),
                    start: clamp_i64(base),
                    end: clamp_i64(base + limit as i128),
                };
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
                        &child_span,
                        ctx,
                        &child,
                    )?;
                }
                Ok(())
            }
        }
    }
}

/// Развёртка вызова рутины: инстанцирование с таблицей; тело — в окружении
/// вызывающего плюс данные, пожары — в пустом окружении (данные рутины им
/// недоступны статически, а чужое окружение затирало бы глобальные имена:
/// резолв идёт `env` раньше `defs`).
/// Табличный параметр из окружения затирается (в условиях он невидим — E11).
/// Спан именуется рутиной (§6: в спанах светится её имя).
fn unfold_routine(
    name: &str,
    args: &[Expr],
    base: i128,
    k: i128,
    ctx: &mut Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
) -> Result<(), Error> {
    let at = i64::try_from(base).unwrap_or(i64::MAX);
    let routine = ctx.tables.routines.get(name).expect("имена уже проверены");
    // Таблица — литеральная: пробросы подставлены при инстанцировании
    // родительской рутины, в циклах/корне — только литералы по валидации.
    // Арность уже проверена (E12): длины совпадают.
    debug_assert_eq!(routine.params.len(), args.len());
    let table_name = match args.first() {
        Some(Expr::Name(t)) => t.as_str(),
        _ => unreachable!("форма вызова проверена в validate_names"),
    };
    let table_param = routine.params.first().expect("параметры проверены");
    let mut child = env.clone();
    child.remove(table_param);
    for (param, arg) in routine.params.iter().skip(1).zip(args.iter().skip(1)) {
        child.insert(param.clone(), eval_expr_with_env(arg, at, ctx.defs, env)?);
    }
    let table = ctx
        .tables
        .tables
        .get(table_name)
        .expect("таблица проверена");
    let limit = duration_ms(&table.duration)?;
    let child_span = Span {
        cycle: name.to_owned(),
        start: clamp_i64(base),
        end: clamp_i64(base + limit as i128),
    };
    let inst = instantiate(routine, table_name, ctx.tables)?;
    for st in &inst.body {
        let offset = effective_offset_ms(st, limit, &table.duration.raw)?;
        unfold_stmt(
            st,
            Frame {
                base,
                offset,
                limit,
                limit_raw: &table.duration.raw,
                k,
            },
            &child_span,
            ctx,
            &child,
        )?;
    }
    let fresh: HashMap<String, Value> = HashMap::new();
    for st in &inst.firings {
        let offset = effective_offset_ms(st, limit, &table.duration.raw)?;
        unfold_stmt(
            st,
            Frame {
                base,
                offset,
                limit,
                limit_raw: &table.duration.raw,
                k,
            },
            &child_span,
            ctx,
            &fresh,
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cond::{check_conditions, resolve_units, Defs, TableReg};
    use crate::datetime::{format_datetime, parse_datetime};
    use crate::validate::{check_bounds, check_recursion, check_tables, validate_names};
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
        // Импорты — из памяти: route_lib.cyclo лежит в libs/ рядом с route.cyclo.
        // Без `use` чтение не вызывается, остальные фикстуры не меняются.
        let libs: HashMap<PathBuf, String> = HashMap::from([(
            PathBuf::from("libs/route_lib.cyclo"),
            include_str!("../../../examples/valid/libs/route_lib.cyclo").to_owned(),
        )]);
        let mut groups = crate::imports::collect_units(&file.uses, Path::new(""), &mut |p| {
            libs.get(p)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "нет в памяти"))
        })
        .expect("импорты тестов обязаны разрешаться");
        groups.push(file.decls.clone());
        let (defs, reg) = resolve_units(&groups).unwrap();
        let d: &'static Defs = Box::leak(Box::new(defs));
        let reg: &'static TableReg = Box::leak(Box::new(reg));
        let t = validate_names(ast, reg).unwrap();
        check_recursion(ast, &t).unwrap();
        check_tables(ast, &t).unwrap();
        check_bounds(ast, &t).unwrap();
        check_conditions(ast, d, &t).unwrap();
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
    fn next_matches_expand_prefix() {
        // next_events(from, within, n) == первые n развёртки [from, from+within).
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        let day = 86_400_000i64;
        for (from_raw, within, n) in [
            ("2026-01-09T00:00:00", 2 * day, 5),
            ("2026-01-09T06:00:00", day, 1),
            ("2026-01-09T06:00:01", day, 3),
            ("2026-01-09T10:20:00", day, 4),
            ("2026-01-09T00:00:00", 2 * day, 100),
            ("2026-01-09T00:00:00", 0, 10),
            ("2026-01-09T00:00:00", day, 0),
        ] {
            let from = parse_datetime(from_raw).unwrap();
            let full = expand(ast, &t, d, from, from + within).unwrap();
            let want: Vec<Event> = full.into_iter().take(n).collect();
            let got = next_events(ast, &t, d, from, within, n).unwrap();
            assert_eq!(got, want, "для {from_raw} +{within} n={n}");
        }
    }

    #[test]
    fn next_finds_sparse_event_within_cap() {
        // Одно событие в году: поиск дотягивается через пустые месяцы.
        let src = r#"
schedule "Редкое" {
  point BELL { actions = [ring]; }
  cycle DAY duration = 24h {
    [datestr(at) == "2026-12-31"] 12h: BELL.ring();
  }
  root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {
    0h: DAY();
  }
}"#;
        let (ast, t, d) = setup(src);
        let from = parse_datetime("2026-01-05T00:00:00").unwrap();
        let got = next_events(ast, &t, d, from, 366 * 86_400_000, 3).unwrap();
        assert_eq!(times(&got), vec!["2026-12-31T12:00:00"]);
        // Капа не хватает — пусто без ошибки.
        let got = next_events(ast, &t, d, from, 30 * 86_400_000, 3).unwrap();
        assert_eq!(got, vec![]);
    }

    #[test]
    fn next_counts_events_not_instants() {
        // n считает события: связка 10:20 (arrive+depart) режется пополам.
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        let from = parse_datetime("2026-01-09T10:20:00").unwrap();
        let got = next_events(ast, &t, d, from, 86_400_000, 1).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(format_datetime(got[0].time), "2026-01-09T10:20:00");
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
    fn params_flow_into_blocks_and_conditions() {
        // Параметр виден и в блоках, и в условиях; условие режет строки.
        let src = "const LEC = {\"name\": \"БЖД\", \"type\": \"лек\"}; \
            const PR = {\"name\": \"БЖД\", \"type\": \"прак\"}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle LESSON(subj) duration = 1h { \
            0m: B.ring() { subject = subj.name, event = \"start\" }; \
            [subj.type == \"лек\"] 30m: B.ring() { subject = subj.name, event = \"extra\" }; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 9h: LESSON(LEC); 13h: LESSON(PR); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(
            times(&events),
            vec![
                "2026-01-01T09:00:00",
                "2026-01-01T09:30:00",
                "2026-01-01T13:00:00",
            ]
        );
        let str_ = |s: &str| crate::cond::Value::Str(s.to_owned());
        assert_eq!(
            events[0].action_attrs,
            vec![
                ("subject".to_owned(), str_("БЖД")),
                ("event".to_owned(), str_("start")),
            ]
        );
        assert_eq!(
            events[1].action_attrs,
            vec![
                ("subject".to_owned(), str_("БЖД")),
                ("event".to_owned(), str_("extra")),
            ]
        );
        // У практики нет extra-строки: третье событие — снова start.
        assert_eq!(
            events[2].action_attrs[1],
            ("event".to_owned(), str_("start"))
        );
    }

    #[test]
    fn inner_cycle_sees_outer_params() {
        // Динамический скоуп: безаргументный INNER видит subj вызывающего.
        let src = "const LEC = {\"name\": \"БЖД\", \"type\": \"лек\"}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle INNER duration = 30m { [subj.type == \"лек\"] 0m: B.ring() { subject = subj.name }; } \
            cycle LESSON(subj) duration = 1h { 0m: INNER(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: LESSON(LEC); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(times(&events), vec!["2026-01-01T09:00:00"]);
        assert_eq!(
            events[0].action_attrs,
            vec![(
                "subject".to_owned(),
                crate::cond::Value::Str("БЖД".to_owned())
            )]
        );
    }

    #[test]
    fn inner_call_shadows_outer_param() {
        // Вложенный вызов со своим аргументом перетирает параметр целиком.
        let src = "const LEC = {\"name\": \"Лек\", \"type\": \"лек\"}; \
            const PR = {\"name\": \"Прак\", \"type\": \"прак\"}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle INNER(subj) duration = 30m { 0m: B.ring() { subject = subj.name }; } \
            cycle OUTER(subj) duration = 1h { 0m: INNER(PR); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: OUTER(LEC); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(times(&events), vec!["2026-01-01T09:00:00"]);
        assert_eq!(
            events[0].action_attrs,
            vec![(
                "subject".to_owned(),
                crate::cond::Value::Str("Прак".to_owned())
            )]
        );
    }

    #[test]
    fn unbound_param_is_runtime_e11() {
        // Статика пропускает (имя — параметр LESSON), строка без связывания — E11.
        let src = "const LEC = {\"name\": \"БЖД\"}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle LESSON(subj) duration = 1h { 0m: B.ring() { subject = subj.name }; } \
            cycle INNER duration = 30m { 0m: B.ring() { subject = subj.name }; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: INNER(); } }";
        let file = Box::leak(Box::new(cyclorithm_parser::parse(src).unwrap()));
        let ast = &file.schedule;
        let groups = vec![file.decls.clone()];
        let (defs, reg) = resolve_units(&groups).unwrap();
        let d = Box::leak(Box::new(defs));
        let reg = Box::leak(Box::new(reg));
        let t = validate_names(ast, reg).unwrap();
        check_recursion(ast, &t).unwrap();
        check_tables(ast, &t).unwrap();
        check_bounds(ast, &t).unwrap();
        check_conditions(ast, d, &t).expect("статика видит имя параметра");
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let err = expand(ast, &t, d, s, e).expect_err("несвязанный параметр — ошибка");
        assert_eq!(
            (err.code, err.message.as_str()),
            ("E11", "unknown name 'subj'")
        );
    }

    #[test]
    fn point_attrs_copy_to_events() {
        // Литерал, ссылка на константу, отсутствие — три точки, три словаря.
        let src = "const CORPUS = {\"building\": \"Л\"}; \
            schedule \"T\" { point A { actions = [x]; attrs = {\"gps\": \"1,2\"}; } \
            point B { actions = [x]; attrs = CORPUS; } \
            point C { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); 10m: B.x(); 20m: C.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(times(&events).len(), 3);
        let str_ = |s: &str| crate::cond::Value::Str(s.to_owned());
        assert_eq!(events[0].point_attrs, vec![("gps".to_owned(), str_("1,2"))]);
        assert_eq!(
            events[1].point_attrs,
            vec![("building".to_owned(), str_("Л"))]
        );
        assert!(events[2].point_attrs.is_empty());
        assert!(events.iter().all(|ev| ev.action_attrs.is_empty()));
    }

    #[test]
    fn bad_point_attrs_fail_expand() {
        // Ошибки атрибутов — в начале развёртки (после всех проверок §5).
        for (decls, point, code, message) in [
            (
                "",
                "point A { actions = [x]; attrs = {\"a\": 1, \"a\": 2}; }",
                "E15",
                "duplicate attribute 'a'",
            ),
            (
                "",
                "point A { actions = [x]; attrs = NOPE; }",
                "E11",
                "unknown name 'NOPE'",
            ),
            (
                "const N = 5;",
                "point A { actions = [x]; attrs = N; }",
                "E12",
                "type mismatch: cannot mix number and string",
            ),
            (
                "fun f(t) = t;",
                "point A { actions = [x]; attrs = f; }",
                "E12",
                "type mismatch: cannot mix number and string",
            ),
        ] {
            let src = format!(
                "{decls} schedule \"T\" {{ {point} \
                root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ 6h: A.x(); }} }}"
            );
            let file = Box::leak(src.into_boxed_str());
            let parsed = Box::leak(Box::new(cyclorithm_parser::parse(file).unwrap()));
            let ast = &parsed.schedule;
            let groups = vec![parsed.decls.clone()];
            let (defs, reg) = resolve_units(&groups).unwrap();
            let d = Box::leak(Box::new(defs));
            let reg = Box::leak(Box::new(reg));
            let t = validate_names(ast, reg).unwrap();
            check_recursion(ast, &t).unwrap();
            check_tables(ast, &t).unwrap();
            check_bounds(ast, &t).unwrap();
            check_conditions(ast, d, &t).unwrap();
            let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
            let err = expand(ast, &t, d, s, e).expect_err("атрибуты обязаны браковаться");
            assert_eq!(err.code, code, "для {point}");
            assert_eq!(err.message.as_str(), message, "для {point}");
        }
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
    fn events_carry_innermost_span() {
        // Спан — ближайший экземпляр цикла; у прямых действий — root_cycle.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); 60m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); 8h: A.x(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        let spans: Vec<(&str, String, String)> = events
            .iter()
            .map(|ev| {
                (
                    ev.span.cycle.as_str(),
                    format_datetime(ev.span.start),
                    format_datetime(ev.span.end),
                )
            })
            .collect();
        assert_eq!(
            spans,
            vec![
                (
                    "R",
                    "2026-01-01T06:00:00".to_owned(),
                    "2026-01-01T07:00:00".to_owned()
                ),
                (
                    "R",
                    "2026-01-01T06:00:00".to_owned(),
                    "2026-01-01T07:00:00".to_owned()
                ),
                (
                    "root_cycle",
                    "2026-01-01T00:00:00".to_owned(),
                    "2026-01-02T00:00:00".to_owned()
                ),
            ]
        );
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

    /// Сквозная рутина: метки из таблицы, обед-пожар только по будням,
    /// данные текут в условия и блоки, спаны именованы рутиной.
    /// 2026-09-07 — понедельник, 2026-09-12 — суббота.
    fn routine_src() -> &'static str {
        "time_const DAY duration = 24h { \
            1st: 9h; \
            [workday(at)] lunch: 12h -> LUNCH(); \
        } \
        schedule \"T\" { point A { actions = [x]; } point B { actions = [y]; } \
        routine M(TC, subj) { [subj == 1] 1st: A.x() { n = subj }; } \
        cycle LUNCH duration = 30m { 0m: B.y(); } \
        root_cycle start_time = \"2026-09-07T00:00:00\", duration = 24h { \
        [day_of_week(at) == 1] 0h: M(DAY, 1); \
        [day_of_week(at) == 6] 0h: M(DAY, 1); } }"
    }

    #[test]
    fn expands_routine_with_labels_and_firing() {
        let (ast, t, d) = setup(routine_src());
        let (s, e) = window("2026-09-07T00:00:00", "2026-09-13T00:00:00");
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
        // Понедельник: пара в 9:00 и обед в 12:00; суббота: только пара.
        assert_eq!(
            got,
            vec![
                (
                    "2026-09-07T09:00:00".to_owned(),
                    "x".to_owned(),
                    "A".to_owned()
                ),
                (
                    "2026-09-07T12:00:00".to_owned(),
                    "y".to_owned(),
                    "B".to_owned()
                ),
                (
                    "2026-09-12T09:00:00".to_owned(),
                    "x".to_owned(),
                    "A".to_owned()
                ),
            ]
        );
        // Спаны: строки рутины — её именем, пожар — внутренним циклом
        // (ближайший экземпляр, как у вложенных циклов).
        let spans: Vec<&str> = events.iter().map(|ev| ev.span.cycle.as_str()).collect();
        assert_eq!(spans, vec!["M", "LUNCH", "M"]);
        // Данные — в action_attrs пары.
        assert_eq!(
            events[0].action_attrs,
            vec![("n".to_owned(), Value::Num(1))]
        );
        assert!(events[1].action_attrs.is_empty());
    }

    #[test]
    fn routine_data_filters_rows() {
        // `subj == 1` режет строки: с subj=2 пара не выходит, обед — да.
        let src = routine_src().replace("M(DAY, 1)", "M(DAY, 2)");
        let (ast, t, d) = setup(&src);
        let (s, e) = window("2026-09-07T00:00:00", "2026-09-08T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(times(&events), vec!["2026-09-07T12:00:00"]);
    }

    #[test]
    fn day_of_week_matches_calendar() {
        // 2026-09-07 — понедельник (=1), 2026-09-13 — воскресенье (=7).
        let (ast, t, d) = setup(routine_src());
        let (s, e) = window("2026-09-07T00:00:00", "2026-09-08T00:00:00");
        let events = expand(ast, &t, d, s, e).unwrap();
        assert_eq!(events.len(), 2);
        for case in [("2026-09-07T00:00:00", 1), ("2026-09-13T00:00:00", 7)] {
            let at = parse_datetime(case.0).unwrap();
            let v = eval_expr_with_env(
                &cyclorithm_parser::Expr::Call {
                    name: "day_of_week".to_owned(),
                    args: vec![cyclorithm_parser::Expr::At],
                },
                at,
                d,
                &HashMap::new(),
            )
            .unwrap();
            assert_eq!(v, Value::Num(case.1), "для {}", case.0);
        }
    }
}
