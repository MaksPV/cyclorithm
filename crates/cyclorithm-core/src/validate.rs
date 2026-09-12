//! Проверка имён: дубликаты (duplicate) и разрешение вызовов (unknown-point/action-not-allowed/unknown-cycle/wrong-kind).
//!
//! Порядок проверок: сначала все объявления (duplicate, wrong-kind на столкновение
//! имён точки, рутины и цикла; wrong-arguments на пустые параметры рутины; duplicate на дубли
//! меток таблиц), затем все вызовы в порядке объявления (циклы, рутины,
//! `root_cycle`, пожары `->` таблиц). Первая ошибка побеждает.
//!
//! Правило общего пространства (§3): одно имя не может обозначать точку,
//! рутину и цикл одновременно — нарушение wrong-kind. На вызове: точка как цикл —
//! `point 'X' is not a cycle`, цикл как точка — `cycle 'X' is not a point`,
//! рутина как точка — `routine 'X' is not a point`.
//! Вызов `NAME(...)` — это вызов рутины, если имя — рутина (первый аргумент
//! обязателен и является таблицей: проброс своего параметра или литеральное
//! имя из реестра, иначе unknown-table); иначе — обычный вызов цикла.

use std::collections::{HashMap, HashSet};

use cyclorithm_parser::{
    Expr, Invocation, Repeat, Routine, RoutineOffset, Schedule, SlotRow, Stmt,
};

use crate::cond::TableReg;
use crate::duration::{duration_ms, effective_offset_ms, format_duration, root_period_ms};
use crate::Error;

/// Таблицы имён после успешной проверки — вход later-фаз ядра.
#[derive(Debug)]
pub struct NameTables<'a> {
    /// Точка → её объявление (список `actions`).
    pub points: HashMap<&'a str, &'a cyclorithm_parser::Point>,
    /// Цикл → его объявление.
    pub cycles: HashMap<&'a str, &'a cyclorithm_parser::Cycle>,
    /// Рутина → её объявление.
    pub routines: HashMap<&'a str, &'a Routine>,
    /// Реестр таблиц (`time_const`) из объявлений.
    pub tables: &'a TableReg,
}

/// Проверить объявления и вызовы. Возвращает таблицы имён либо первую ошибку.
pub fn validate_names<'a>(
    schedule: &'a Schedule,
    reg: &'a TableReg,
) -> Result<NameTables<'a>, Error> {
    let mut points = HashMap::new();
    for p in &schedule.points {
        if points.contains_key(p.name.as_str()) {
            return Err(Error::duplicate("point", &p.name));
        }
        points.insert(p.name.as_str(), p);
    }
    let mut routines = HashMap::new();
    for r in &schedule.routines {
        if routines.contains_key(r.name.as_str()) {
            return Err(Error::duplicate("routine", &r.name));
        }
        if points.contains_key(r.name.as_str()) {
            return Err(Error::point_not_routine(&r.name));
        }
        // Первый параметр рутины — таблица: без него вызывать нечем.
        if r.params.is_empty() {
            return Err(Error::no_table_parameter(&r.name));
        }
        // Дубли параметров — duplicate, как у циклов.
        let mut seen = HashSet::new();
        for p in &r.params {
            if !seen.insert(p) {
                return Err(Error::duplicate("param", p));
            }
        }
        routines.insert(r.name.as_str(), r);
    }
    let mut cycles = HashMap::new();
    for c in &schedule.cycles {
        if cycles.contains_key(c.name.as_str()) {
            return Err(Error::duplicate("cycle", &c.name));
        }
        if points.contains_key(c.name.as_str()) {
            return Err(Error::point_not_cycle(&c.name));
        }
        if routines.contains_key(c.name.as_str()) {
            return Err(Error::routine_not_cycle(&c.name));
        }
        // Дубли параметров (`cycle C(a, a)`) — duplicate, как дубли объявлений.
        let mut seen = HashSet::new();
        for p in &c.params {
            if !seen.insert(p) {
                return Err(Error::duplicate("param", p));
            }
        }
        cycles.insert(c.name.as_str(), c);
    }
    // Дубли меток таблицы — duplicate, как дубли объявлений.
    for tname in &reg.order {
        let t = reg.get(tname.as_str()).expect("порядок — по реестру");
        let mut seen = HashSet::new();
        for row in &t.rows {
            if !seen.insert(row.label.as_str()) {
                return Err(Error::duplicate("slot", &row.label));
            }
        }
    }
    let tables = NameTables {
        points,
        cycles,
        routines,
        tables: reg,
    };
    for c in &schedule.cycles {
        check_invocations(&tables, c.stmts.iter().map(|st| &st.invocation), None)?;
    }
    for r in &schedule.routines {
        check_invocations(
            &tables,
            r.stmts.iter().map(|st| &st.invocation),
            Some(r.params[0].as_str()),
        )?;
    }
    check_invocations(
        &tables,
        schedule.root.stmts.iter().map(|st| &st.invocation),
        None,
    )?;
    for tname in &reg.order {
        let t = reg.get(tname.as_str()).expect("порядок — по реестру");
        check_invocations(
            &tables,
            t.rows.iter().filter_map(|row| row.firing.as_ref()),
            None,
        )?;
    }
    Ok(tables)
}

/// Проверить вызовы в порядке объявления.
/// `scope` — имя табличного параметра объемлющей рутины (`None` — вне рутин):
/// в позиции таблицы разрешены его проброс и литеральные имена из реестра.
fn check_invocations<'i, I>(
    tables: &NameTables<'_>,
    invocations: I,
    scope: Option<&str>,
) -> Result<(), Error>
where
    I: IntoIterator<Item = &'i Invocation>,
{
    for inv in invocations {
        match inv {
            Invocation::PointAction { point, action, .. } => {
                if let Some(p) = tables.points.get(point.as_str()) {
                    if !p.actions.iter().any(|a| a == action) {
                        return Err(Error::action_not_allowed(action, point));
                    }
                } else if tables.cycles.contains_key(point.as_str()) {
                    return Err(Error::cycle_not_point(point));
                } else if tables.routines.contains_key(point.as_str()) {
                    return Err(Error::routine_not_point(point));
                } else {
                    return Err(Error::unknown_point(point));
                }
            }
            Invocation::CycleCall { name, args } => {
                if let Some(routine) = tables.routines.get(name.as_str()) {
                    // Арность — сразу за существованием (wrong-arguments, прецедент
                    // арности fun/pred); выражения аргументов — позже, со строками.
                    if routine.params.len() != args.len() {
                        return Err(Error::wrong_arguments(name));
                    }
                    match args.first() {
                        Some(Expr::Name(t)) if Some(t.as_str()) == scope => {}
                        Some(Expr::Name(t)) if tables.tables.get(t.as_str()).is_some() => {}
                        Some(Expr::Name(t)) => return Err(Error::unknown_table(t)),
                        _ => return Err(Error::invalid_table_argument(name)),
                    }
                } else if let Some(callee) = tables.cycles.get(name.as_str()) {
                    // Арность — сразу за существованием (wrong-arguments, прецедент
                    // арности fun/pred); выражения аргументов — позже, со строками.
                    if callee.params.len() != args.len() {
                        return Err(Error::wrong_arguments(name));
                    }
                } else if tables.points.contains_key(name.as_str()) {
                    return Err(Error::point_not_cycle(name));
                } else {
                    return Err(Error::unknown_cycle(name));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Рекурсия (recursive) и границы циклов (cycle-overruns).
// Вызывать после `validate_names`: обе функции предполагают, что все имена
// разрешены (неизвестных циклов уже нет).
// ---------------------------------------------------------------------------

/// Запрет самовызовов — прямых и через цепочку (recursive): циклы, рутины
/// и таблицы в одном графе. Пожары `->` таблицы — рёбра таблицы: срабатывают
/// при каждом инстанцировании с ней. Обход в порядке объявления (циклы,
/// рутины, таблицы); сообщается повторно вошедший узел своим сообщением
/// (`recursive cycle/routine/table`). Корень не участвует: в него ничто
/// не возвращается.
/// Вызывать после `validate_names`: неизвестных имён уже нет.
pub fn check_recursion(schedule: &Schedule, tables: &NameTables<'_>) -> Result<(), Error> {
    let pairs = instantiation_pairs(schedule, tables);
    let mut gray: HashSet<(u8, &str)> = HashSet::new();
    let mut black: HashSet<(u8, &str)> = HashSet::new();
    for c in &schedule.cycles {
        visit(
            (KIND_CYCLE, c.name.as_str()),
            tables,
            &pairs,
            &mut gray,
            &mut black,
        )?;
    }
    for r in &schedule.routines {
        visit(
            (KIND_ROUTINE, r.name.as_str()),
            tables,
            &pairs,
            &mut gray,
            &mut black,
        )?;
    }
    for t in &tables.tables.order {
        visit(
            (KIND_TABLE, t.as_str()),
            tables,
            &pairs,
            &mut gray,
            &mut black,
        )?;
    }
    Ok(())
}

/// Виды узлов графа рекурсии (имена таблиц живут отдельно от циклов/рутин).
const KIND_CYCLE: u8 = 0;
const KIND_ROUTINE: u8 = 1;
const KIND_TABLE: u8 = 2;

/// DFS по графу вызовов. Серая вершина при повторном входе — recursive.
fn visit<'a>(
    node: (u8, &'a str),
    tables: &NameTables<'a>,
    pairs: &[(&'a str, &'a str)],
    gray: &mut HashSet<(u8, &'a str)>,
    black: &mut HashSet<(u8, &'a str)>,
) -> Result<(), Error> {
    if black.contains(&node) {
        return Ok(());
    }
    if !gray.insert(node) {
        return Err(match node.0 {
            KIND_ROUTINE => Error::recursive_routine(node.1),
            KIND_TABLE => Error::recursive_table(node.1),
            _ => Error::recursive_cycle(node.1),
        });
    }
    for next in outgoing(node, tables, pairs) {
        visit(next, tables, pairs, gray, black)?;
    }
    gray.remove(&node);
    black.insert(node);
    Ok(())
}

/// Рёбра узла: вызовы тела (для таблицы — её пожары `->`) плюс рёбра
/// в таблицы литеральных вызовов рутин (их пожары срабатывают при развёртке);
/// проброс табличного параметра — во все таблицы пары инстанцирования.
/// Неизвестные имена пропускаются (сообщит `validate_names`).
fn outgoing<'a>(
    node: (u8, &'a str),
    tables: &NameTables<'a>,
    pairs: &[(&'a str, &'a str)],
) -> Vec<(u8, &'a str)> {
    let mut out: Vec<(u8, &'a str)> = Vec::new();
    // Вызов `NAME(...)`: ребро в цикл/рутину; вызов рутины с литеральной
    // таблицей — ещё и ребро в таблицу.
    let call = |out: &mut Vec<(u8, &'a str)>, name: &'a str, args: &'a [Expr]| {
        if tables.cycles.contains_key(name) {
            push_edge(out, KIND_CYCLE, name);
        } else if tables.routines.contains_key(name) {
            push_edge(out, KIND_ROUTINE, name);
            if let Some(Expr::Name(t)) = args.first() {
                if tables.tables.get(t.as_str()).is_some() {
                    push_edge(out, KIND_TABLE, t.as_str());
                }
            }
        }
    };
    match node.0 {
        KIND_CYCLE => {
            if let Some(c) = tables.cycles.get(node.1) {
                for st in &c.stmts {
                    if let Invocation::CycleCall { name, args } = &st.invocation {
                        call(&mut out, name.as_str(), args);
                    }
                }
            }
        }
        KIND_ROUTINE => {
            if let Some(r) = tables.routines.get(node.1) {
                for st in &r.stmts {
                    if let Invocation::CycleCall { name, args } = &st.invocation {
                        call(&mut out, name.as_str(), args);
                        // Проброс табличного параметра: пожары всех таблиц,
                        // с которыми рутина инстанцируется, — тоже рёбра.
                        if let Some(Expr::Name(t)) = args.first() {
                            let is_passthrough = r.params.first().is_some_and(|p| p == t);
                            if is_passthrough {
                                for (_, tt) in pairs.iter().filter(|(rn, _)| *rn == node.1) {
                                    push_edge(&mut out, KIND_TABLE, tt);
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {
            if let Some(t) = tables.tables.get(node.1) {
                for row in &t.rows {
                    if let Some(Invocation::CycleCall { name, args }) = &row.firing {
                        call(&mut out, name.as_str(), args);
                    }
                }
            }
        }
    }
    out
}

/// Добавить ребро без дублей (порядок — порядок объявления).
fn push_edge<'a>(out: &mut Vec<(u8, &'a str)>, kind: u8, name: &'a str) {
    if !out.contains(&(kind, name)) {
        out.push((kind, name));
    }
}

/// Пары `(рутина, таблица)` инстанцирования в порядке первого использования:
/// циклы, рутины, корень, пожары таблиц. Пробросы табличных параметров
/// замыкаются fixpoint-ом (конечен: множество пар ограничено).
/// Нужны recursive (рёбра пожаров) и `check_tables` (проверка каждой пары один раз).
pub fn instantiation_pairs<'a>(
    schedule: &'a Schedule,
    tables: &NameTables<'a>,
) -> Vec<(&'a str, &'a str)> {
    let mut pairs: Vec<(&'a str, &'a str)> = Vec::new();
    let mut site = |routine: &'a str, args: &'a [Expr]| {
        if !tables.routines.contains_key(routine) {
            return;
        }
        if let Some(Expr::Name(t)) = args.first() {
            if tables.tables.get(t.as_str()).is_some() && !pairs.contains(&(routine, t.as_str())) {
                pairs.push((routine, t.as_str()));
            }
        }
    };
    // Пробросы `(вызывающая, вызываемая)`: `R2(TC)` в теле `R1(TC)`.
    let mut passthrough: Vec<(&'a str, &'a str)> = Vec::new();
    for r in &schedule.routines {
        for st in &r.stmts {
            if let Invocation::CycleCall { name, args } = &st.invocation {
                if let Some(Expr::Name(t)) = args.first() {
                    // Пустые параметры рутины бракует `validate_names`;
                    // здесь — аккуратный доступ ради прямых вызовов в тестах.
                    let is_passthrough = r.params.first().is_some_and(|p| p == t);
                    if is_passthrough
                        && tables.routines.contains_key(name.as_str())
                        && !passthrough.contains(&(r.name.as_str(), name.as_str()))
                    {
                        passthrough.push((r.name.as_str(), name.as_str()));
                    }
                }
            }
        }
    }
    for c in &schedule.cycles {
        for st in &c.stmts {
            if let Invocation::CycleCall { name, args } = &st.invocation {
                site(name.as_str(), args);
            }
        }
    }
    for r in &schedule.routines {
        for st in &r.stmts {
            if let Invocation::CycleCall { name, args } = &st.invocation {
                site(name.as_str(), args);
            }
        }
    }
    for st in &schedule.root.stmts {
        if let Invocation::CycleCall { name, args } = &st.invocation {
            site(name.as_str(), args);
        }
    }
    for tname in &tables.tables.order {
        let t = tables
            .tables
            .get(tname.as_str())
            .expect("порядок — по реестру");
        for row in &t.rows {
            if let Some(Invocation::CycleCall { name, args }) = &row.firing {
                site(name.as_str(), args);
            }
        }
    }
    loop {
        let mut added = false;
        for (caller, callee) in &passthrough {
            let inherit: Vec<&'a str> = pairs
                .iter()
                .filter(|(r, _)| r == caller)
                .map(|(_, t)| *t)
                .collect();
            for t in inherit {
                if !pairs.contains(&(*callee, t)) {
                    pairs.push((*callee, t));
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }
    pairs
}

// ---------------------------------------------------------------------------
// Таблицы и границы инстанцирований (unknown-table/invalid-duration/cycle-overruns/invalid-repeat-count).
// Вызывать после `check_recursion` и до `check_bounds`: покрытие меток идёт
// по парам инстанцирования, границы тел — в длительности их таблиц.
// ---------------------------------------------------------------------------

/// Строка таблицы как `Stmt` для переиспользования `stmts_end`:
/// пожар без повторов в смещении слота.
fn firing_stmt(row: &SlotRow, firing: &Invocation) -> Stmt {
    Stmt {
        offset: row.offset.clone(),
        negative: false,
        repeat: Repeat::Once,
        condition: row.condition.clone(),
        invocation: firing.clone(),
    }
}

/// Инстанцирование рутины с таблицей: тело со смещениями и пожары отдельно
/// (пожары выполняются в пустом окружении — данные рутины им недоступны).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub body: Vec<Stmt>,
    pub firings: Vec<Stmt>,
}

/// Инстанцировать рутину с таблицей: метки → смещения слотов, проброс
/// табличного параметра — в литеральное имя, пожары таблицы — отдельно.
/// Неизвестная метка — unknown-slot.
/// Вызывать после `validate_names` (форма вызовов уже проверена).
pub fn instantiate(
    routine: &Routine,
    table_name: &str,
    tables: &NameTables<'_>,
) -> Result<Instance, Error> {
    let table = tables
        .tables
        .get(table_name)
        .expect("таблица уже проверена");
    let table_param = routine.params.first().expect("параметры уже проверены");
    let mut body = Vec::with_capacity(routine.stmts.len());
    for st in &routine.stmts {
        let offset = match &st.offset {
            RoutineOffset::Duration(d) => d.clone(),
            RoutineOffset::Label(label) => match table.rows.iter().find(|r| &r.label == label) {
                Some(slot) => slot.offset.clone(),
                None => return Err(Error::unknown_slot(label)),
            },
        };
        let invocation = match &st.invocation {
            Invocation::CycleCall { name, args } if tables.routines.contains_key(name.as_str()) => {
                let mut resolved = args.clone();
                if let Some(Expr::Name(t)) = resolved.first() {
                    if t == table_param {
                        resolved[0] = Expr::Name(table_name.to_owned());
                    }
                }
                Invocation::CycleCall {
                    name: name.clone(),
                    args: resolved,
                }
            }
            other => other.clone(),
        };
        body.push(Stmt {
            offset,
            negative: st.negative,
            repeat: st.repeat.clone(),
            condition: st.condition.clone(),
            invocation,
        });
    }
    let mut firings = Vec::new();
    for row in &table.rows {
        if let Some(firing) = &row.firing {
            firings.push(firing_stmt(row, firing));
        }
    }
    Ok(Instance { body, firings })
}

/// Проверить таблицы и инстанцирования рутин: длительности таблиц (invalid-duration),
/// строки таблиц в границах (cycle-overruns/invalid-repeat-count, вина — на таблице), покрытие меток
/// каждой пары (unknown-slot) и границы тел (cycle-overruns/invalid-repeat-count, вина — на рутине).
/// Каждая пара `(рутина, таблица)` проверяется один раз, в порядке первого
/// использования. Невызываемые рутины — только статика имён и условий
/// (покрытие без таблицы проверить нечем).
pub fn check_tables(schedule: &Schedule, tables: &NameTables<'_>) -> Result<(), Error> {
    for tname in &tables.tables.order {
        let table = tables
            .tables
            .get(tname.as_str())
            .expect("порядок — по реестру");
        let limit = duration_ms(&table.duration)?;
        let rows: Vec<Stmt> = table
            .rows
            .iter()
            .filter_map(|row| row.firing.as_ref().map(|f| firing_stmt(row, f)))
            .collect();
        let (end, argmax) = stmts_end(&rows, limit, &table.duration.raw, tables)?;
        if end > limit {
            let row = argmax.expect("конец больше лимита — строка-аргмакс есть");
            return Err(blame(&rows[row], &table.name, end, limit));
        }
    }
    for (rname, tname) in instantiation_pairs(schedule, tables) {
        let routine = tables
            .routines
            .get(rname)
            .expect("пары — по проверенным именам");
        let table = tables
            .tables
            .get(tname)
            .expect("пары — по проверенным именам");
        let inst = instantiate(routine, tname, tables)?;
        let body: Vec<Stmt> = inst.body.into_iter().chain(inst.firings).collect();
        let limit = duration_ms(&table.duration)?;
        let (end, argmax) = stmts_end(&body, limit, &table.duration.raw, tables)?;
        if end > limit {
            let row = argmax.expect("конец больше лимита — строка-аргмакс есть");
            return Err(blame(&body[row], rname, end, limit));
        }
    }
    Ok(())
}
/// Граница циклов (cycle-overruns, правило 8): `actual(C) ≤ duration(C)` для каждого
/// цикла и `root_cycle`. Отрицательные смещения разрешены заранее
/// (`duration(C) − X`); вылет ниже нуля — тоже cycle-overruns
/// (`offset '-2h' out of bounds (duration 1h20m)`), проверяется в порядке
/// объявления и побеждает сразу. В сообщении о переполнении — вызов
/// со строки, давшей максимум (при равных концах — первая в порядке объявления).
/// Строки вызова рутин — через длительность их таблиц (`cycle_or_table_ms`);
/// тела рутин — в `check_tables` (длительности таблиц).
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
/// Порядок проверок строки: invalid-repeat-count, затем until-out-of-bounds/cycle-overruns (горизонт, конец цепочки).
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
                Invocation::CycleCall { name, args } => cycle_or_table_ms(name, args, tables)?,
            };
            Ok((1, span))
        }
        Repeat::Times(raw) => {
            let n: u64 = raw.parse().map_err(|_| Error::invalid_repeat_count(raw))?;
            if n == 0 {
                return Err(Error::invalid_repeat_count(raw));
            }
            Ok((n, step_of(&st.invocation, tables)?))
        }
        Repeat::Fill { until } => {
            let step = step_of(&st.invocation, tables)?;
            if step == 0 {
                return Err(match &st.invocation {
                    Invocation::CycleCall { name, .. } => Error::fill_zero_duration(name),
                    Invocation::PointAction { action, .. } => Error::repeat_point_action(action),
                });
            }
            let horizon = match until {
                None => limit,
                Some(u) => {
                    let t = duration_ms(&u.duration)?;
                    let h = if u.negative {
                        if t > limit {
                            return Err(Error::until_out_of_bounds(&u.raw(), limit_raw));
                        }
                        limit - t
                    } else {
                        t
                    };
                    if h > limit {
                        return Err(Error::until_out_of_bounds(&u.raw(), limit_raw));
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
/// объявленная длительность для вызова цикла,
/// длительность таблицы для вызова рутины.
/// Повтор действия точки — repeat-point-action.
fn step_of(invocation: &Invocation, tables: &NameTables<'_>) -> Result<i64, Error> {
    match invocation {
        Invocation::PointAction { action, .. } => Err(Error::repeat_point_action(action)),
        Invocation::CycleCall { name, args } => cycle_or_table_ms(name, args, tables),
    }
}

/// Длина вызова цикла или рутины: объявленная длительность либо длительность
/// таблицы. Таблица — литеральная: пробросы подставляются при инстанцировании
/// (см. `check_tables`), форма вызова проверена в `validate_names`.
fn cycle_or_table_ms(name: &str, args: &[Expr], tables: &NameTables<'_>) -> Result<i64, Error> {
    if tables.routines.contains_key(name) {
        match args.first() {
            Some(Expr::Name(t)) => {
                let table = tables
                    .tables
                    .get(t.as_str())
                    .expect("таблица уже проверена");
                duration_ms(&table.duration)
            }
            _ => unreachable!("форма вызова рутины проверена в validate_names"),
        }
    } else {
        duration_ms(&cycle_duration(tables, name).duration)
    }
}

/// Объявление вызываемого цикла (имена уже проверены).
fn cycle_duration<'a>(tables: &NameTables<'a>, name: &str) -> &'a cyclorithm_parser::Cycle {
    tables.cycles.get(name).expect("имена уже проверены")
}

/// `offset + n*d` с насыщением: переполнение всё равно больше лимита.
fn saturating_add_mul(offset: i64, n: u64, d: i64) -> i64 {
    let end = offset as i128 + n as i128 * d as i128;
    i64::try_from(end).unwrap_or(i64::MAX)
}

/// Ошибка cycle-overruns с виной на вызове строки: цикл — `cycle 'X' overruns ...`,
/// действие точки — симметричное `action 'a' overruns ...`.
fn blame(stmt: &Stmt, outer: &str, end: i64, limit: i64) -> Error {
    let excess = format_duration(end - limit);
    let end_s = format_duration(end);
    let limit_s = format_duration(limit);
    match &stmt.invocation {
        Invocation::PointAction { action, .. } => {
            Error::action_overruns(action, outer, &excess, &end_s, &limit_s)
        }
        Invocation::CycleCall { name, .. } => {
            Error::cycle_overruns(name, outer, &excess, &end_s, &limit_s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(src: &str) -> cyclorithm_parser::Schedule {
        cyclorithm_parser::parse(src)
            .expect("фикстура обязана разбираться")
            .schedule
    }

    fn err(src: &str) -> Error {
        let ast = parsed(src);
        let reg = TableReg::default();
        validate_names(&ast, &reg).expect_err("ожидалась ошибка имён")
    }

    #[test]
    fn accepts_route() {
        let src = include_str!("../../../examples/valid/route.cyclo");
        let ast = parsed(src);
        let reg = TableReg::default();
        let tables = validate_names(&ast, &reg).expect("route обязан проходить проверку имён");
        assert_eq!(tables.points.len(), 2);
        assert_eq!(tables.cycles.len(), 2);
    }

    #[test]
    fn error_codes_match_fixtures() {
        // (файл, код, сообщение) — дословно по главе ошибок.
        for (file, src, code, message) in [
            (
                "bad_unknown-point",
                include_str!("../../../examples/invalid/bad_unknown-point.cyclo"),
                "unknown-point",
                "unknown point 'PORT'",
            ),
            (
                "bad_action-not-allowed",
                include_str!("../../../examples/invalid/bad_action-not-allowed.cyclo"),
                "action-not-allowed",
                "action 'arrive' not allowed for point 'DEPOT'",
            ),
            (
                "bad_unknown-cycle",
                include_str!("../../../examples/invalid/bad_unknown-cycle.cyclo"),
                "unknown-cycle",
                "unknown cycle 'NIGHT_ROUTE'",
            ),
            (
                "bad_duplicate",
                include_str!("../../../examples/invalid/bad_duplicate.cyclo"),
                "duplicate",
                "duplicate point 'DEPOT'",
            ),
            (
                "bad_wrong-kind",
                include_str!("../../../examples/invalid/bad_wrong-kind.cyclo"),
                "wrong-kind",
                "point 'DEPOT' is not a cycle",
            ),
        ] {
            let e = err(src);
            assert_eq!(e.code, code, "для {file}");
            assert_eq!(e.message, message, "для {file}");
        }
    }

    #[test]
    fn rejects_duplicate_cycle_params() {
        // Дубли параметров — duplicate, как дубли объявлений.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle C(a, a) duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: C(1, 2); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("duplicate", "duplicate param 'a'")
        );
    }

    #[test]
    fn rejects_cycle_arity_mismatch() {
        // Арность — сразу за существованием (wrong-arguments, прецедент fun/pred).
        for (row, name) in [("6h: C();", "C"), ("6h: C(1, 2);", "C"), ("6h: R(1);", "R")] {
            let src = format!(
                "schedule \"T\" {{ point A {{ actions = [x]; }} \
                cycle C(a) duration = 1h {{ 0m: A.x(); }} \
                cycle R duration = 1h {{ 0m: A.x(); }} \
                root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ {row} }} }}"
            );
            let e = err(&src);
            assert_eq!(e.code, "wrong-arguments", "для {row}");
            assert_eq!(
                e.message,
                format!("wrong arguments for '{name}'"),
                "для {row}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_cycle() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            cycle R duration = 2h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("duplicate", "duplicate cycle 'R'")
        );
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
            ("wrong-kind", "point 'R' is not a cycle")
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
            ("wrong-kind", "cycle 'R' is not a point")
        );
    }

    /// Разобранная фикстура + таблицы имён. `Box::leak` — тестовый приём,
    /// чтобы таблицы жили `'static` рядом со своим AST.
    fn tables(src: &str) -> (&'static cyclorithm_parser::Schedule, NameTables<'static>) {
        let ast: &'static cyclorithm_parser::Schedule = Box::leak(Box::new(parsed(src)));
        let reg: &'static TableReg = Box::leak(Box::new(TableReg::default()));
        let t = validate_names(ast, reg).expect("имена обязаны проходить");
        (ast, t)
    }

    /// Полный разбор программы с объявлениями: таблицы — через `resolve_units`.
    fn full(src: &str) -> (&'static cyclorithm_parser::Schedule, NameTables<'static>) {
        let file: &'static cyclorithm_parser::SourceFile = Box::leak(Box::new(
            cyclorithm_parser::parse(src).expect("фикстура обязана разбираться"),
        ));
        let (_, reg) = crate::cond::resolve_units(std::slice::from_ref(&file.decls))
            .expect("объявления обязаны проверяться");
        let reg: &'static TableReg = Box::leak(Box::new(reg));
        let t = validate_names(&file.schedule, reg).expect("имена обязаны проходить");
        (&file.schedule, t)
    }

    /// Первая ошибка программы с объявлениями (объявления или имена).
    fn full_err(src: &str) -> Error {
        let file = cyclorithm_parser::parse(src).expect("фикстура обязана разбираться");
        match crate::cond::resolve_units(std::slice::from_ref(&file.decls)) {
            Ok((_, reg)) => validate_names(&file.schedule, &reg).expect_err("ожидалась ошибка"),
            Err(e) => e,
        }
    }

    const DAY: &str = "time_const DAY duration = 24h { 1st: 9h; }";

    #[test]
    fn rejects_unknown_table() {
        // Таблицы с таким именем нет — unknown-table (таблицы живут отдельно от циклов).
        let src = "schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(SHORT); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("unknown-table", "unknown table 'SHORT'")
        );
    }

    #[test]
    fn rejects_non_name_table_arg() {
        // Первый аргумент рутины — таблица, не выражение.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(1 + 1); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("invalid-table-argument", "invalid table argument for 'M'")
        );
    }

    #[test]
    fn rejects_routine_arity_mismatch() {
        // Арность — сразу за существованием (wrong-arguments, как у циклов).
        for row in ["0h: M();", "0h: M(D, 1, 2);"] {
            let src = format!(
                "{DAY} schedule \"T\" {{ point A {{ actions = [x]; }} \
                routine M(TC, subj) {{ 0m: A.x(); }} \
                root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ {row} }} }}"
            );
            let e = full_err(&src);
            assert_eq!(e.code, "wrong-arguments", "для {row}");
            assert_eq!(e.message, "wrong arguments for 'M'", "для {row}");
        }
    }

    #[test]
    fn rejects_routine_name_clashes() {
        // Общее пространство имён: точка/рутина/цикл не делят имя.
        for (src, code, message) in [
            (
                "routine M(TC) { 0m: A.x(); } routine M(TC) { 0m: A.x(); }",
                "duplicate",
                "duplicate routine 'M'",
            ),
            (
                "point M { actions = [x]; } routine M(TC) { 0m: M.x(); }",
                "wrong-kind",
                "point 'M' is not a routine",
            ),
            (
                "routine M(TC) { 0m: A.x(); } cycle M duration = 1h { 0m: A.x(); }",
                "wrong-kind",
                "routine 'M' is not a cycle",
            ),
            (
                "routine M(TC) { 0m: A.x(); } cycle C duration = 1h { 0m: M.x(); }",
                "wrong-kind",
                "routine 'M' is not a point",
            ),
        ] {
            let src = format!(
                "schedule \"T\" {{ point A {{ actions = [x]; }} {src} \
                root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ 0h: A.x(); }} }}"
            );
            let e = err(&src);
            assert_eq!((e.code, e.message.as_str()), (code, message), "для {src}");
        }
    }

    #[test]
    fn rejects_paramless_routine() {
        // Без табличного параметра рутину вызывать нечем.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            routine M { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: A.x(); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("no-table-parameter", "routine 'M' has no table parameter")
        );
    }

    #[test]
    fn rejects_duplicate_routine_params() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            routine M(TC, TC) { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: A.x(); } }";
        let e = err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("duplicate", "duplicate param 'TC'")
        );
    }

    #[test]
    fn rejects_duplicate_slot() {
        // Дубли меток таблицы — duplicate.
        let src = "time_const D duration = 2h { 1st: 0m; 1st: 1h; } \
            schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: A.x(); } }";
        let e = full_err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("duplicate", "duplicate slot '1st'")
        );
    }

    #[test]
    fn accepts_routine_call_and_passthrough() {
        // Литеральная таблица и проброс параметра — валидные имена.
        let src = format!(
            "{DAY} schedule \"T\" {{ point A {{ actions = [x]; }} \
            routine M(TC) {{ 0m: A.x(); }} \
            routine W(TC) {{ 0m: M(TC); }} \
            cycle C duration = 1h {{ 0m: M(DAY); }} \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            {{ 0h: M(DAY); 1h: W(DAY); 2h: C(); }} }}"
        );
        let (ast, t) = full(&src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let pairs = instantiation_pairs(ast, &t);
        assert_eq!(pairs, vec![("M", "DAY"), ("W", "DAY")]);
    }

    #[test]
    fn rejects_routine_cycle_recursion() {
        // Цикл через рутину и обратно — recursive; сообщается повторно вошедший узел.
        let src = format!(
            "{DAY} schedule \"T\" {{ point A {{ actions = [x]; }} \
            routine M(TC) {{ 0m: C(); }} \
            cycle C duration = 1h {{ 0m: M(DAY); }} \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ 0h: C(); }} }}"
        );
        let (ast, t) = full(&src);
        let e = check_recursion(ast, &t).expect_err("цикл через рутину");
        // Обход от циклов: C → M → C, повторно вошёл C.
        assert_eq!(
            (e.code, e.message.as_str()),
            ("recursive", "recursive cycle 'C'")
        );
    }

    #[test]
    fn rejects_self_recursive_routine() {
        let src = format!(
            "{DAY} schedule \"T\" {{ point A {{ actions = [x]; }} \
            routine M(TC) {{ 0m: M(DAY); }} \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ 0h: M(DAY); }} }}"
        );
        let (ast, t) = full(&src);
        let e = check_recursion(ast, &t).expect_err("самовызов рутины");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("recursive", "recursive routine 'M'")
        );
    }

    #[test]
    fn rejects_table_self_fire_recursion() {
        // Пожар `->`, инстанцирующий рутину с той же таблицей, — recursive таблицы.
        let src = "time_const D duration = 2h { tick: 0m -> M(D); } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(D); } }";
        let (ast, t) = full(src);
        let e = check_recursion(ast, &t).expect_err("пожар по кругу");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("recursive", "recursive table 'D'")
        );
    }

    #[test]
    fn passthrough_inherits_tables() {
        // `W(TC) { M(TC); }`, вызванная с двумя таблицами, даёт обе пары для M.
        let src = "time_const D1 duration = 2h { 1st: 0m; } \
            time_const D2 duration = 3h { 1st: 0m; } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 0m: A.x(); } \
            routine W(TC) { 0m: M(TC); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 0h: W(D1); 1h: W(D2); } }";
        let (ast, t) = full(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let pairs = instantiation_pairs(ast, &t);
        assert_eq!(
            pairs,
            vec![("W", "D1"), ("W", "D2"), ("M", "D1"), ("M", "D2")]
        );
    }

    /// Полная проверка до таблиц: имена, рекурсия, затем `check_tables`.
    fn tables_err(src: &str) -> Error {
        let (ast, t) = full(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_tables(ast, &t).expect_err("ожидалась ошибка таблиц")
    }

    /// Полная проверка до границ (включая таблицы).
    fn bounds_full_err(src: &str) -> Error {
        let (ast, t) = full(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_tables(ast, &t).expect("таблицы в порядке");
        check_bounds(ast, &t).expect_err("ожидалась ошибка границ")
    }

    #[test]
    fn rejects_unknown_slot() {
        // Метки нет в таблице вызова — unknown-slot (строгость: не молчаливый пропуск).
        let src = "time_const DAY duration = 24h { 1st: 9h; } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 8th: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(DAY); } }";
        let e = tables_err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("unknown-slot", "unknown slot '8th'")
        );
    }

    #[test]
    fn rejects_table_row_overrun() {
        // Пожар за длительностью таблицы — cycle-overruns с виной на таблице.
        let src = "time_const DAY duration = 1h { 1st: 0m; late: 2h -> A.x(); } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 1st: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(DAY); } }";
        let e = tables_err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                "action-overruns",
                "action 'x' overruns 'DAY' by 60m (120m > 60m)"
            )
        );
    }

    #[test]
    fn rejects_routine_body_overrun() {
        // Тело вылезает из таблицы — cycle-overruns с виной на рутине (светится её имя).
        let src = "time_const DAY duration = 1h { 1st: 0m; } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 1st: C(); } \
            cycle C duration = 2h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(DAY); } }";
        let e = tables_err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                "cycle-overruns",
                "cycle 'C' overruns 'M' by 60m (120m > 60m)"
            )
        );
    }

    #[test]
    fn rejects_routine_chain_overrun() {
        // Шаг цепочки вызова рутины — длительность таблицы.
        let src = "time_const DAY duration = 24h { 1st: 0m; } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 1st: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 0h: repeat 2 M(DAY); } }";
        let e = bounds_full_err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                "cycle-overruns",
                "cycle 'M' overruns 'root_cycle' by 1440m (2880m > 1440m)"
            )
        );
    }

    #[test]
    fn rejects_repeat_zero_in_routine() {
        // Повторы тела проверяются в длительности таблицы (invalid-repeat-count).
        let src = "time_const DAY duration = 24h { 1st: 0m; } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 1st: repeat 0 C(); } \
            cycle C duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(DAY); } }";
        let e = tables_err(src);
        assert_eq!(
            (e.code, e.message.as_str()),
            ("invalid-repeat-count", "invalid repeat count '0'")
        );
    }

    #[test]
    fn uncalled_routine_skipped() {
        // Невызываемую рутину покрыть нечем: только статика имён и условий.
        let src = "time_const DAY duration = 24h { 1st: 0m; } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 8th: A.x(); } \
            routine N(TC) { -10m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: A.x(); } }";
        let (ast, t) = full(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        check_tables(ast, &t).expect("невызываемые рутины не проверяются");
    }

    #[test]
    fn instantiate_resolves_labels_and_passthrough() {
        // Метки → смещения, проброс → литерал, пожары — в конец.
        let src =
            "time_const DAY duration = 24h { 1st: 9h; [workday(at)] lunch: 12h -> LUNCH(); } \
            schedule \"T\" { point A { actions = [x]; } point B { actions = [y]; } \
            routine W(TC) { 0m: A.x(); } \
            routine M(TC) { 1st: A.x(); 45m: B.y(); 0m: W(TC); 0m: C(); } \
            cycle LUNCH duration = 1h { 0m: B.y(); } \
            cycle C duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(DAY); } }";
        let (ast, t) = full(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let routine = t.routines.get("M").expect("рутина есть");
        let inst = instantiate(routine, "DAY", &t).expect("метки покрыты");
        let body = &inst.body;
        let firings = &inst.firings;
        assert_eq!(body.len(), 4);
        assert_eq!(firings.len(), 1);
        let raws: Vec<&str> = body.iter().map(|st| st.offset.raw.as_str()).collect();
        assert_eq!(raws, vec!["9h", "45m", "0m", "0m"]);
        // Проброс подставлен, литералы и циклы не тронуты.
        let table_of = |st: &Stmt| match &st.invocation {
            Invocation::CycleCall { name, args } => (name.clone(), args.clone()),
            inv => panic!("ожидался вызов цикла, получено {inv:?}"),
        };
        assert_eq!(
            table_of(&body[2]),
            ("W".to_owned(), vec![Expr::Name("DAY".to_owned())])
        );
        assert_eq!(table_of(&body[3]).0, "C");
        // Пожар — отдельно, с условием таблицы.
        assert_eq!(firings.len(), 1);
        assert!(firings[0].condition.is_some());
        assert!(matches!(
            firings[0].invocation,
            Invocation::CycleCall { ref name, .. } if name == "LUNCH"
        ));
        assert_eq!(firings[0].offset.raw.as_str(), "12h");
    }

    #[test]
    fn recursion_and_bounds_match_fixtures() {
        for (file, src, code, message) in [
            (
                "bad_recursive",
                include_str!("../../../examples/invalid/bad_recursive.cyclo"),
                "recursive",
                "recursive cycle 'A'",
            ),
            (
                "bad_cycle-overruns",
                include_str!("../../../examples/invalid/bad_cycle-overruns.cyclo"),
                "cycle-overruns",
                "cycle 'CYCLE2' overruns 'CYCLE1' by 20m (80m > 60m)",
            ),
        ] {
            let (ast, t) = tables(src);
            let e = check_recursion(ast, &t)
                .and_then(|()| check_bounds(ast, &t))
                .expect_err("ожидалась recursive/cycle-overruns");
            assert_eq!(e.code, code, "для {file}");
            assert_eq!(e.message, message, "для {file}");
        }
    }

    #[test]
    fn accepts_route_recursion_and_bounds() {
        let src = include_str!("../../../examples/valid/route.cyclo");
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
            ("recursive", "recursive cycle 'A1'")
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
        // Мгновенное событие за границей периода — тоже cycle-overruns.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 61m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("событие за границей");
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                "action-overruns",
                "action 'x' overruns 'R' by 1m (61m > 60m)"
            )
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
            (
                "cycle-overruns",
                "cycle 'R' overruns 'root_cycle' by 30m (90m > 60m)"
            )
        );
    }

    #[test]
    fn actual_duration_matches_spec_formula() {
        // actual(C) = max(o + D): D = 0 для точки, declared для цикла.
        let (_, t) = tables(include_str!("../../../examples/valid/route.cyclo"));
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
        // -2h при duration = 1h20m → эффективное -40m: offset-out-of-bounds, сообщение по главе ошибок.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); -2h: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("вылет ниже нуля");
        assert_eq!(e.code, "offset-out-of-bounds");
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
        // Эффективное смещение + длина вызова за границей — обычный cycle-overruns.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle INNER duration = 40m { 0m: A.x(); } \
            cycle OUTER duration = 1h { -10m: INNER(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: OUTER(); } }";
        let (ast, t) = tables(src);
        check_recursion(ast, &t).expect("рекурсии нет");
        let e = check_bounds(ast, &t).expect_err("50m + 40m = 90m > 60m");
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                "cycle-overruns",
                "cycle 'INNER' overruns 'OUTER' by 30m (90m > 60m)"
            )
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
            (
                "cycle-overruns",
                "cycle 'B1' overruns 'OUTER' by 70m (130m > 60m)"
            )
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
            ("invalid-repeat-count", "invalid repeat count '0'")
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
            (
                "repeat-point-action",
                "repeat of point action 'x' not allowed"
            )
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
            ("fill-zero-duration", "fill of zero-duration cycle 'EMPTY'")
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
                "cycle-overruns",
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
            (
                "until-out-of-bounds",
                "until '30h' out of bounds (duration 24h)"
            )
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
            (
                "until-out-of-bounds",
                "until '-30h' out of bounds (duration 24h)"
            )
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
