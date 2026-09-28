//! Развёртка решётки `root_cycle` в плоский список событий (см. docs/reference/semantics.md).
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

use crate::parser::{DepKind, Expr, Invocation, MomentArg, MomentExpr, Routine, Schedule};

use crate::Error;
use crate::cond::{
    AtFrame, Defs, TimeTable, Value, eval_cond_with_env, eval_expr_with_env, resolve_point_attrs,
};
use crate::datetime::{parse_in_frame, parse_timezone};
use crate::duration::{duration_ms, format_duration, root_period_ms};
use crate::validate::{NameTables, instantiate, plan_stmts, plan_stmts_with, root_actual_ms};

/// Зона кадра в мс (`file.or(query)`, минуты → мс): стена = абсолют + зона.
/// Тот же кадр, что у `parse_in_frame` для наивных дат файла.
fn frame_zone_ms(file_zone: Option<i16>, query_zone: Option<i16>) -> i64 {
    file_zone.or(query_zone).map_or(0, |z| z as i64 * 60_000)
}

/// Кадр стека `here.stack`: имя цикла/рутины и его параметры.
/// Пустые параметры (`root_cycle`) — без ключа `params`.
#[derive(Debug, Clone)]
struct StackFrame {
    name: String,
    params: Vec<(String, Value)>,
}

/// Разрешённая метка запуска (issue 45): абсолютные времена старта/конца
/// инстанции и флаг эмиссии. Неиспущенная метка (условие ложно или
/// зависимость не разрешилась) в таблице отсутствует — ссылки на неё
/// дают каскадный skip, а не ошибку.
#[derive(Debug, Clone)]
struct LabelResolution {
    start_ms: i128,
    end_ms: i128,
    emitted: bool,
}

/// Курсор scope (issue 45): `max(end)` всех уже испущенных прямых детей
/// scope в смещениях от старта scope. Стартовое значение — 0 (старт scope):
/// первое `+30m:` — это 30m от старта. Невыполненная строка (условие ложно
/// или зависимость не разрешилась) курсор не двигает.
#[derive(Debug, Clone, Default)]
struct Cursor {
    position_ms: i64,
}

impl Cursor {
    fn get(&self) -> i64 {
        self.position_ms
    }

    fn advance(&mut self, end_ms: i64) {
        if end_ms > self.position_ms {
            self.position_ms = end_ms;
        }
    }
}

/// Кандидат `here.events`: строка инстанции с флагом `enabled`.
/// `point` — для действий точки, `cycle` — для вызовов циклов/рутин.
/// Невыполненная строка аргументы не трогает (как и раньше): её параметры
/// пусты (у рутины — только дескриптор `TC`, таблица — литерал).
#[derive(Debug, Clone)]
struct Candidate {
    point: Option<PointView>,
    cycle: Option<CycleView>,
    event: EventView,
}

#[derive(Debug, Clone)]
struct PointView {
    name: String,
    actions: Vec<String>,
    attrs: Vec<(String, Value)>,
}

#[derive(Debug, Clone)]
enum CycleView {
    Cycle {
        name: String,
        duration_ms: i64,
        params: Vec<(String, Value)>,
    },
    Routine {
        name: String,
        params: Vec<(String, Value)>,
        labels: Vec<(String, Value)>,
    },
}

#[derive(Debug, Clone)]
struct EventView {
    action: Option<String>,
    offset: i64,
    label: Option<String>,
    at: AtFrame,
    enabled: bool,
}

impl StackFrame {
    fn to_value(&self) -> Value {
        let mut pairs = vec![("name".to_owned(), Value::Str(self.name.clone()))];
        if !self.params.is_empty() {
            pairs.push(("params".to_owned(), Value::Map(self.params.clone())));
        }
        Value::Map(pairs)
    }
}

impl PointView {
    fn to_value(&self) -> Value {
        Value::Map(vec![
            ("name".to_owned(), Value::Str(self.name.clone())),
            (
                "actions".to_owned(),
                Value::Array(self.actions.iter().cloned().map(Value::Str).collect()),
            ),
            ("attrs".to_owned(), Value::Map(self.attrs.clone())),
        ])
    }
}

impl CycleView {
    fn to_value(&self) -> Value {
        match self {
            CycleView::Cycle {
                name,
                duration_ms,
                params,
            } => Value::Map(vec![
                ("name".to_owned(), Value::Str(name.clone())),
                ("duration".to_owned(), Value::Num(*duration_ms)),
                ("params".to_owned(), Value::Map(params.clone())),
            ]),
            CycleView::Routine {
                name,
                params,
                labels,
            } => Value::Map(vec![
                ("name".to_owned(), Value::Str(name.clone())),
                ("params".to_owned(), Value::Map(params.clone())),
                ("labels".to_owned(), Value::Map(labels.clone())),
            ]),
        }
    }
}

impl EventView {
    fn to_value(&self) -> Value {
        let mut pairs = Vec::with_capacity(5);
        if let Some(action) = &self.action {
            pairs.push(("action".to_owned(), Value::Str(action.clone())));
        }
        pairs.push(("offset".to_owned(), Value::Num(self.offset)));
        if let Some(label) = &self.label {
            pairs.push(("label".to_owned(), Value::Str(label.clone())));
        }
        pairs.push(("at".to_owned(), self.at.to_value()));
        pairs.push(("enabled".to_owned(), Value::Bool(self.enabled)));
        Value::Map(pairs)
    }
}

impl Candidate {
    fn to_value(&self) -> Value {
        let mut pairs = Vec::with_capacity(3);
        if let Some(point) = &self.point {
            pairs.push(("point".to_owned(), point.to_value()));
        }
        if let Some(cycle) = &self.cycle {
            pairs.push(("cycle".to_owned(), cycle.to_value()));
        }
        pairs.push(("event".to_owned(), self.event.to_value()));
        Value::Map(pairs)
    }
}

/// Словарь `here`: стек вызовов, все кандидаты и геттеры по флагу.
/// Снимок «на данный момент» — строится заново на каждую строку;
/// в `action_attrs` замораживается копированием (вложенный `here` мёртв,
/// живых ссылок нет). Отсутствующие `point`/`cycle`/`action`/`label` —
/// без ключа (`Value` без null): доступ к ним — `unknown-field`.
fn here_value(stack: &[StackFrame], events: &[Candidate]) -> Value {
    let all: Vec<Value> = events.iter().map(Candidate::to_value).collect();
    let enabled: Vec<Value> = events
        .iter()
        .filter(|c| c.event.enabled)
        .map(Candidate::to_value)
        .collect();
    let disabled: Vec<Value> = events
        .iter()
        .filter(|c| !c.event.enabled)
        .map(Candidate::to_value)
        .collect();
    Value::Map(vec![
        (
            "stack".to_owned(),
            Value::Array(stack.iter().map(StackFrame::to_value).collect()),
        ),
        ("events".to_owned(), Value::Array(all)),
        ("enabled_events".to_owned(), Value::Array(enabled)),
        ("disabled_events".to_owned(), Value::Array(disabled)),
    ])
}

/// Дескриптор таблицы для `TC`: `{duration, labels}` (метки — в мс).
fn table_value(table: &TimeTable) -> Result<Value, Error> {
    let duration = duration_ms(&table.duration)?;
    let mut labels = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        labels.push((row.label.clone(), Value::Num(duration_ms(&row.offset)?)));
    }
    Ok(Value::Map(vec![
        ("duration".to_owned(), Value::Num(duration)),
        ("labels".to_owned(), Value::Map(labels)),
    ]))
}

/// Имя таблицы вызова рутины (форма проверена в `validate_names`).
fn call_table_name(args: &[Expr], name: &str) -> Result<String, Error> {
    match args.first() {
        Some(Expr::Name(t)) => Ok(t.clone()),
        _ => unreachable!("форма вызова {name} проверена в validate_names"),
    }
}

/// Параметры кадра `here` + дочернее окружение вызова рутины.
type RoutineFrame = (Vec<(String, Value)>, HashMap<String, Value>);

/// Данные вызова рутины: дескриптор `TC` + разрешённые аргументы.
/// Табличный параметр из окружения затирается (в условиях он невидим).
/// Возвращает параметры кадра `here` и дочернее окружение.
fn routine_call_values(
    routine: &Routine,
    table_name: &str,
    args: &[Expr],
    at: &AtFrame,
    ctx: &Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
) -> Result<RoutineFrame, Error> {
    let table = ctx
        .tables
        .tables
        .get(table_name)
        .expect("таблица проверена");
    let tc = table_value(table)?;
    let table_param = routine.params.first().expect("параметры проверены");
    let mut params = vec![(table_param.clone(), tc)];
    let mut child = env.clone();
    child.remove(table_param);
    for (param, arg) in routine.params.iter().skip(1).zip(args.iter().skip(1)) {
        let v = eval_expr_with_env(arg, at, ctx.defs, env)?;
        params.push((param.clone(), v.clone()));
        child.insert(param.clone(), v);
    }
    Ok((params, child))
}

// ---------------------------------------------------------------------------
// Моменты issue 45: разрешение единой оси в абсолютное время.
// ---------------------------------------------------------------------------

/// Разрешить выражение момента в абсолютное время (мс epoch).
/// Возвращает `None`, если зависимость не разрешена (метка не испущена) —
/// вызывающий делает каскадный skip: событие не испускается, курсор стоит.
/// `scope_base` — старт scope (мс epoch), `scope_limit` — длительность scope.
/// Границы здесь не проверяются (кроме `-X` больше лимита) — их смотрит вызывающий.
fn resolve_moment(
    moment: &MomentExpr,
    cursor: &Cursor,
    labels: &HashMap<String, LabelResolution>,
    scope_base: i128,
    scope_limit: i64,
    limit_raw: &str,
    moment_raw: &str,
) -> Result<Option<i128>, Error> {
    match moment {
        MomentExpr::Absolute { negative, duration } => {
            let x = duration_ms(duration)?;
            if *negative {
                if x > scope_limit {
                    return Err(Error::offset_out_of_bounds(moment_raw, limit_raw));
                }
                Ok(Some(scope_base + (scope_limit - x) as i128))
            } else {
                Ok(Some(scope_base + x as i128))
            }
        }
        MomentExpr::Relative { offset } => {
            let add = match offset {
                Some(d) => duration_ms(d)?,
                None => 0,
            };
            Ok(Some(scope_base + cursor.get() as i128 + add as i128))
        }
        MomentExpr::Dependency { kind, label } => match labels.get(label.as_str()) {
            Some(lr) if lr.emitted => Ok(Some(match kind {
                DepKind::After | DepKind::Until => lr.end_ms,
                DepKind::At => lr.start_ms,
            })),
            _ => Ok(None),
        },
        MomentExpr::Func { name, args } => resolve_moment_func(
            name,
            args,
            cursor,
            labels,
            scope_base,
            scope_limit,
            limit_raw,
            moment_raw,
        ),
    }
}

/// Аргументы функций момента: голая метка — конец её инстанции
/// (как `after`), длительность — от старта scope, выражение — рекурсивно.
/// `None` — каскад (метка не испущена).
fn resolve_moment_arg(
    arg: &MomentArg,
    cursor: &Cursor,
    labels: &HashMap<String, LabelResolution>,
    scope_base: i128,
    scope_limit: i64,
    limit_raw: &str,
    moment_raw: &str,
) -> Result<Option<i128>, Error> {
    match arg {
        MomentArg::Label(l) => match labels.get(l.as_str()) {
            Some(lr) if lr.emitted => Ok(Some(lr.end_ms)),
            _ => Ok(None),
        },
        MomentArg::Duration(d) => Ok(Some(scope_base + duration_ms(d)? as i128)),
        MomentArg::Expr(e) => resolve_moment(
            e,
            cursor,
            labels,
            scope_base,
            scope_limit,
            limit_raw,
            moment_raw,
        ),
    }
}

/// Функции момента: `start/end` — ровно 1 голая метка (старт/конец),
/// `max/min` — ≥1 аргумента. Неразрешённый аргумент — каскадный `None`.
/// Арность проверена в `validate_moments`; здесь — защита на случай
/// прямого вызова без валидации.
#[allow(clippy::too_many_arguments)]
fn resolve_moment_func(
    name: &str,
    args: &[MomentArg],
    cursor: &Cursor,
    labels: &HashMap<String, LabelResolution>,
    scope_base: i128,
    scope_limit: i64,
    limit_raw: &str,
    moment_raw: &str,
) -> Result<Option<i128>, Error> {
    match name {
        "start" | "end" => {
            let label = match args {
                [MomentArg::Label(l)] => l,
                _ => {
                    return Err(Error::invalid_moment_function(&format!(
                        "'{name}' takes a bare label"
                    )));
                }
            };
            match labels.get(label.as_str()) {
                Some(lr) if lr.emitted => Ok(Some(if name == "start" {
                    lr.start_ms
                } else {
                    lr.end_ms
                })),
                _ => Ok(None),
            }
        }
        "max" | "min" => {
            if args.is_empty() {
                return Err(Error::invalid_moment_function(&format!(
                    "'{name}' takes at least 1 argument"
                )));
            }
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                match resolve_moment_arg(
                    arg,
                    cursor,
                    labels,
                    scope_base,
                    scope_limit,
                    limit_raw,
                    moment_raw,
                )? {
                    Some(t) => values.push(t),
                    None => return Ok(None),
                }
            }
            let result = if name == "max" {
                values.into_iter().max()
            } else {
                values.into_iter().min()
            };
            Ok(result)
        }
        _ => Err(Error::invalid_moment_function(&format!(
            "unknown moment function '{name}'"
        ))),
    }
}

/// Длина вызова: `0` для действия точки, объявленная длительность
/// для цикла, длительность таблицы для рутины.
fn invocation_span(invocation: &Invocation, tables: &NameTables<'_>) -> Result<i64, Error> {
    match invocation {
        Invocation::PointAction { .. } => Ok(0),
        Invocation::CycleCall { name, args } => {
            if tables.routines.contains_key(name.as_str()) {
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
                let cycle = tables
                    .cycles
                    .get(name.as_str())
                    .expect("имена уже проверены");
                duration_ms(&cycle.duration)
            }
        }
    }
}

/// Шаг цепочки повторов: длина вызова; повтор действия точки —
/// `repeat-point-action` (как в статическом `chain`).
fn invocation_step(invocation: &Invocation, tables: &NameTables<'_>) -> Result<i64, Error> {
    match invocation {
        Invocation::PointAction { action, .. } => Err(Error::repeat_point_action(action)),
        Invocation::CycleCall { .. } => invocation_span(invocation, tables),
    }
}

/// Горизонт `fill`: конец scope по умолчанию, иначе `until`
/// (зеркало `fill_horizon` из validate — та же семантика границ).
fn dynamic_horizon(
    until: &Option<crate::parser::Until>,
    limit: i64,
    limit_raw: &str,
) -> Result<i64, Error> {
    match until {
        None => Ok(limit),
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
            Ok(h)
        }
    }
}

/// Вина за переполнение динамической строки: цикл — `cycle-overruns`,
/// действие точки — `action-overruns` (зеркало `blame` из validate).
fn blame_dynamic(stmt: &crate::parser::Stmt, outer: &str, end: i64, limit: i64) -> Error {
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

/// Старты экземпляров динамической строки от первого разрешённого смещения.
/// Семантика — как у статического `chain` + материализация: `Once` — один,
/// `repeat N` — встык, `fill` — до горизонта; всё, что за лимитом, —
/// вина `blame_dynamic`. `FillGaps` здесь быть не должно (валидация
/// отклоняет `gaps-dynamic-moment` раньше).
fn dynamic_starts(
    stmt: &crate::parser::Stmt,
    first: i64,
    limit: i64,
    limit_raw: &str,
    outer: &str,
    tables: &NameTables<'_>,
) -> Result<Vec<i64>, Error> {
    use crate::parser::Repeat;
    match &stmt.repeat {
        Repeat::Once => {
            let span = invocation_span(&stmt.invocation, tables)?;
            let end = first.saturating_add(span);
            if end > limit {
                return Err(blame_dynamic(stmt, outer, end, limit));
            }
            Ok(if first <= limit {
                vec![first]
            } else {
                Vec::new()
            })
        }
        Repeat::Times(raw) => {
            let n: u64 = raw.parse().map_err(|_| Error::invalid_repeat_count(raw))?;
            if n == 0 {
                return Err(Error::invalid_repeat_count(raw));
            }
            let step = invocation_step(&stmt.invocation, tables)?;
            let end = first as i128 + n as i128 * step as i128;
            let end = i64::try_from(end).unwrap_or(i64::MAX);
            if end > limit {
                return Err(blame_dynamic(stmt, outer, end, limit));
            }
            // Материализация только влезающих (как в статическом плане).
            let mut starts = Vec::new();
            let mut t = first as i128;
            for _ in 0..n {
                if t > limit as i128 {
                    break;
                }
                starts.push(t as i64);
                t += step as i128;
            }
            Ok(starts)
        }
        Repeat::Fill { until } => {
            let step = invocation_step(&stmt.invocation, tables)?;
            if step == 0 {
                return Err(match &stmt.invocation {
                    Invocation::CycleCall { name, .. } => Error::fill_zero_duration(name),
                    Invocation::PointAction { action, .. } => Error::repeat_point_action(action),
                });
            }
            let horizon = dynamic_horizon(until, limit, limit_raw)?;
            if first >= horizon {
                return Ok(Vec::new());
            }
            let n = ((horizon - first) / step) as u64;
            let end = first as i128 + n as i128 * step as i128;
            let end = i64::try_from(end).unwrap_or(i64::MAX);
            if end > limit {
                return Err(blame_dynamic(stmt, outer, end, limit));
            }
            let mut starts = Vec::new();
            let mut t = first as i128;
            while t < horizon as i128 && t <= limit as i128 {
                starts.push(t as i64);
                t += step as i128;
            }
            Ok(starts)
        }
        Repeat::FillGaps { .. } => Err(Error::gaps_dynamic_moment(&stmt.moment_raw())),
    }
}

/// Спан экземпляра цикла для таймлайна: имя цикла и границы
/// `[start, end)` в мс epoch (конец — по объявленной длительности).
/// У действий напрямую в `root_cycle` — `cycle: "root_cycle"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub cycle: String,
    pub start: i64,
    pub end: i64,
}

/// Событие вывода (см. docs/reference/output.md): время — `i64` мс epoch, остальное — имена
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
    zone_ms: i64,
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
        labels: HashMap::new(),
    };
    let plans = plan_stmts(
        &schedule.root.stmts,
        period_ms,
        &schedule.root.duration.raw,
        tables,
    )?;
    let stack = [StackFrame {
        name: "root_cycle".to_owned(),
        params: Vec::new(),
    }];
    let mut events = Vec::new();
    // Курсор корня (issue 45): движется только испущенными строками
    // (внутри `unfold_stmt`/`unfold_stmt_dynamic`).
    let mut root_cursor = Cursor::default();
    for (st, pl) in schedule.root.stmts.iter().zip(plans.iter()) {
        if matches!(st.moment, crate::parser::MomentExpr::Absolute { .. }) {
            unfold_stmt(
                st,
                &pl.starts,
                None,
                base,
                k,
                &root_span,
                &mut ctx,
                &root_env,
                &stack,
                &mut events,
                zone_ms,
                &mut root_cursor,
            )?;
        } else {
            unfold_stmt_dynamic(
                st,
                base,
                k,
                &root_span,
                &mut ctx,
                &root_env,
                &stack,
                &mut events,
                zone_ms,
                period_ms,
                &schedule.root.duration.raw,
                &mut root_cursor,
            )?;
        }
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
/// `query_zone` — зона окна (`from`): кадр наивных дат файла, когда в файле
/// нет `timezone` (см. docs/reference/semantics.md).
pub fn next_events(
    schedule: &Schedule,
    tables: &NameTables<'_>,
    defs: &Defs,
    from_ms: i64,
    within_ms: i64,
    n: usize,
    query_zone: Option<i16>,
) -> Result<Vec<Event>, Error> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let file_zone = match &schedule.timezone {
        Some(raw) => Some(parse_timezone(raw)?),
        None => None,
    };
    let t0 = parse_in_frame(&schedule.root.start_time, file_zone, query_zone)? as i128;
    let period = root_period_ms(&schedule.root)? as i128;
    let horizon = root_actual_ms(schedule, tables)? as i128;
    let point_attrs = resolve_point_attrs(schedule, defs)?;
    // Период влезает в i64: пришёл из root_period_ms.
    let period_ms = period as i64;
    let zone_ms = frame_zone_ms(file_zone, query_zone);

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
            zone_ms,
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
            } else if let Some(worst) = heap.peek()
                && top < *worst
            {
                heap.pop();
                heap.push(top);
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
/// `query_zone` — зона окна (`start`): кадр наивных дат файла, когда в файле
/// нет `timezone` (см. docs/reference/semantics.md).
pub fn expand(
    schedule: &Schedule,
    tables: &NameTables<'_>,
    defs: &Defs,
    start_ms: i64,
    end_ms: i64,
    query_zone: Option<i16>,
) -> Result<Vec<Event>, Error> {
    let file_zone = match &schedule.timezone {
        Some(raw) => Some(parse_timezone(raw)?),
        None => None,
    };
    let t0 = parse_in_frame(&schedule.root.start_time, file_zone, query_zone)?;
    let period = root_period_ms(&schedule.root)?;
    let horizon = root_actual_ms(schedule, tables)?;
    // Атрибуты точек — после всех проверок главы ошибок, до первой строки.
    let point_attrs = resolve_point_attrs(schedule, defs)?;
    let zone_ms = frame_zone_ms(file_zone, query_zone);

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
            zone_ms,
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

/// Общее состояние обхода: таблицы, определения, атрибуты точек и аккумуляторы.
struct Ctx<'a, 'n, 'o, 'm> {
    tables: &'a NameTables<'n>,
    defs: &'a Defs,
    point_attrs: &'m HashMap<String, Vec<(String, Value)>>,
    out: &'o mut Vec<RawEvent>,
    seq: &'o mut usize,
    /// Метки запуска текущего инстанса корня (issue 45): свежие на каждый `k`
    /// (Ctx создаётся заново в `unfold_root_instance`), общие для вложенных
    /// вызовов — ссылки видны внутри всего инстанса root_cycle.
    labels: HashMap<String, LabelResolution>,
}
/// Развёртка строки: старты экземпляров уже посчитаны `plan_stmts`
/// (валидация прошла, счёт конечен). Порядок обхода задаёт `seq` для сортировки.
/// `env` — динамическое окружение параметров цепочки вызовов,
/// `stack`/`events` — контекст инстанции для `here` (снимок «на данный момент»:
/// кандидаты ранее разобранных строк), `row_label` — метка строки в рутине.
/// Невыполненная строка пишется в `here.events` с `enabled: false` и дальше
/// не идёт (в timeline попадают только выполненные).
/// `cursor` — курсор scope (issue 45): двигается только испущенными
/// экземплярами (`max(end)`); метка `as` регистрируется в `ctx.labels`
/// на каждый испущенный экземпляр (последний побеждает).
#[allow(clippy::too_many_arguments)]
fn unfold_stmt(
    stmt: &crate::parser::Stmt,
    starts: &[i64],
    row_label: Option<&str>,
    base: i128,
    k: i128,
    parent: &Span,
    ctx: &mut Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
    stack: &[StackFrame],
    events: &mut Vec<Candidate>,
    zone_ms: i64,
    cursor: &mut Cursor,
) -> Result<(), Error> {
    for &start in starts {
        let abs = i64::try_from(base + start as i128).unwrap_or(i64::MAX);
        let at = AtFrame::new(abs, zone_ms);
        let mut cond_env = env.clone();
        cond_env.insert("here".to_owned(), here_value(stack, events));
        let enabled = match &stmt.condition {
            Some(cond) => eval_cond_with_env(cond, &at, ctx.defs, &cond_env)?,
            None => true,
        };
        if !enabled {
            events.push(disabled_candidate(
                &stmt.invocation,
                row_label,
                start,
                at,
                ctx,
            )?);
            continue;
        }
        unfold(
            &stmt.invocation,
            row_label,
            start,
            base + start as i128,
            k,
            parent,
            ctx,
            env,
            stack,
            events,
            zone_ms,
        )?;
        let span = invocation_span(&stmt.invocation, ctx.tables)?;
        cursor.advance(start.saturating_add(span));
        if let Some(label) = &stmt.launch_label {
            ctx.labels.insert(
                label.0.clone(),
                LabelResolution {
                    start_ms: base + start as i128,
                    end_ms: base + start as i128 + span as i128,
                    emitted: true,
                },
            );
        }
    }
    Ok(())
}

/// Развёртка строки с динамическим моментом (issue 45): момент разрешается
/// в рантайме через курсор scope и таблицу меток, старты — через
/// `dynamic_starts`, дальше — как обычно (условие на каждый экземпляр,
/// каскадный skip без движения курсора, курсор — max(end) испущенных,
/// метка `as` — старт/конец каждого испущенного экземпляра, последний побеждает).
/// `scope_limit`/`limit_raw` — длительность объемлющего scope.
#[allow(clippy::too_many_arguments)]
fn unfold_stmt_dynamic(
    stmt: &crate::parser::Stmt,
    base: i128,
    k: i128,
    parent: &Span,
    ctx: &mut Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
    stack: &[StackFrame],
    events: &mut Vec<Candidate>,
    zone_ms: i64,
    scope_limit: i64,
    limit_raw: &str,
    cursor: &mut Cursor,
) -> Result<(), Error> {
    let raw = stmt.moment_raw();
    let first = match resolve_moment(
        &stmt.moment,
        cursor,
        &ctx.labels,
        base,
        scope_limit,
        limit_raw,
        &raw,
    )? {
        Some(t) => t,
        None => {
            // Каскадный skip: зависимость от неиспущенной метки.
            // Смещение для `here` бессмысленно — кладём 0, курсор стоит.
            let at = AtFrame::new(clamp_i64(base), zone_ms);
            events.push(disabled_candidate(&stmt.invocation, None, 0, at, ctx)?);
            return Ok(());
        }
    };
    if first < base || first > base + scope_limit as i128 {
        return Err(Error::offset_out_of_bounds(&raw, limit_raw));
    }
    let first_offset = (first - base) as i64;
    let starts = dynamic_starts(
        stmt,
        first_offset,
        scope_limit,
        limit_raw,
        &parent.cycle,
        ctx.tables,
    )?;
    for start_offset in starts {
        let abs = base + start_offset as i128;
        let at = AtFrame::new(clamp_i64(abs), zone_ms);
        let mut cond_env = env.clone();
        cond_env.insert("here".to_owned(), here_value(stack, events));
        let enabled = match &stmt.condition {
            Some(cond) => eval_cond_with_env(cond, &at, ctx.defs, &cond_env)?,
            None => true,
        };
        if !enabled {
            events.push(disabled_candidate(
                &stmt.invocation,
                None,
                start_offset,
                at,
                ctx,
            )?);
            continue;
        }
        unfold(
            &stmt.invocation,
            None,
            start_offset,
            abs,
            k,
            parent,
            ctx,
            env,
            stack,
            events,
            zone_ms,
        )?;
        // Курсор — max(end) испущенных; метка — старт/конец инстанции.
        let span = invocation_span(&stmt.invocation, ctx.tables)?;
        cursor.advance(start_offset.saturating_add(span));
        if let Some(label) = &stmt.launch_label {
            ctx.labels.insert(
                label.0.clone(),
                LabelResolution {
                    start_ms: abs,
                    end_ms: abs + span as i128,
                    emitted: true,
                },
            );
        }
    }
    Ok(())
}

/// Кандидат невыполненной строки: аргументы не вычисляются (как и раньше),
/// параметры пусты; у рутины — только дескриптор `TC` (таблица — литерал).
fn disabled_candidate(
    invocation: &Invocation,
    row_label: Option<&str>,
    offset: i64,
    at: AtFrame,
    ctx: &Ctx<'_, '_, '_, '_>,
) -> Result<Candidate, Error> {
    let label = row_label.map(str::to_owned);
    let event = |action: Option<String>| EventView {
        action,
        offset,
        label,
        at,
        enabled: false,
    };
    match invocation {
        Invocation::PointAction { point, action, .. } => {
            let decl = ctx.tables.points.get(point.as_str());
            Ok(Candidate {
                point: Some(PointView {
                    name: point.clone(),
                    actions: decl.map(|p| p.actions.clone()).unwrap_or_default(),
                    attrs: ctx.point_attrs.get(point).cloned().unwrap_or_default(),
                }),
                cycle: None,
                event: event(Some(action.clone())),
            })
        }
        Invocation::CycleCall { name, args } => {
            if ctx.tables.routines.contains_key(name.as_str()) {
                let routine = ctx
                    .tables
                    .routines
                    .get(name.as_str())
                    .expect("имена проверены");
                let table_name = call_table_name(args, name)?;
                let table = ctx
                    .tables
                    .tables
                    .get(table_name.as_str())
                    .expect("таблица проверена");
                let table_param = routine.params.first().expect("параметры проверены");
                let tc = table_value(table)?;
                let labels = match &tc {
                    Value::Map(pairs) => pairs
                        .iter()
                        .find_map(|(k, v)| (k == "labels").then(|| v.clone()))
                        .and_then(|v| match v {
                            Value::Map(xs) => Some(xs),
                            _ => None,
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                Ok(Candidate {
                    point: None,
                    cycle: Some(CycleView::Routine {
                        name: name.clone(),
                        params: vec![(table_param.clone(), tc)],
                        labels,
                    }),
                    event: event(None),
                })
            } else {
                let cycle = ctx
                    .tables
                    .cycles
                    .get(name.as_str())
                    .expect("имена уже проверены");
                Ok(Candidate {
                    point: None,
                    cycle: Some(CycleView::Cycle {
                        name: name.clone(),
                        duration_ms: duration_ms(&cycle.duration)?,
                        params: Vec::new(),
                    }),
                    event: event(None),
                })
            }
        }
    }
}

/// Рекурсивная развёртка вызова с накопленной базой времени.
/// `parent` — спан ближайшего цикла (для корня — `root_cycle`).
/// Аргументы вычисляются в окружении вызывающего (`at` — время экземпляра),
/// параметры связываются поверх него (вложенный вызов перетирает целиком);
/// вызов без аргументов окружение не меняет (течёт вниз как есть).
/// Выполненная строка пишется в `here.events` с `enabled: true` до разбора
/// тела — блок действий видит `here` уже с собственным кандидатом.
#[allow(clippy::too_many_arguments)]
fn unfold(
    invocation: &Invocation,
    row_label: Option<&str>,
    offset: i64,
    base: i128,
    k: i128,
    parent: &Span,
    ctx: &mut Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
    stack: &[StackFrame],
    events: &mut Vec<Candidate>,
    zone_ms: i64,
) -> Result<(), Error> {
    let at = AtFrame::new(i64::try_from(base).unwrap_or(i64::MAX), zone_ms);
    let label = row_label.map(str::to_owned);
    match invocation {
        Invocation::PointAction {
            point,
            action,
            block,
        } => {
            let decl = ctx.tables.points.get(point.as_str());
            events.push(Candidate {
                point: Some(PointView {
                    name: point.clone(),
                    actions: decl.map(|p| p.actions.clone()).unwrap_or_default(),
                    attrs: ctx.point_attrs.get(point).cloned().unwrap_or_default(),
                }),
                cycle: None,
                event: EventView {
                    action: Some(action.clone()),
                    offset,
                    label,
                    at,
                    enabled: true,
                },
            });
            // Блок — либо литерал (значения-выражения), либо ссылка на
            // константу-мапу: оба вычислимы одним `eval_expr`. `here` в блоке —
            // заморозка момента (включая собственный кандидат): дальше не живой.
            let mut block_env = env.clone();
            block_env.insert("here".to_owned(), here_value(stack, events));
            let action_attrs = match block {
                None => Vec::new(),
                Some(b) => match eval_expr_with_env(b, &at, ctx.defs, &block_env)? {
                    Value::Map(pairs) => pairs,
                    _ => return Err(Error::type_mismatch()),
                },
            };
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
                unfold_routine(
                    name,
                    args,
                    label.as_deref(),
                    offset,
                    base,
                    k,
                    ctx,
                    env,
                    stack,
                    events,
                    zone_ms,
                )
            } else {
                let cycle = ctx
                    .tables
                    .cycles
                    .get(name.as_str())
                    .expect("имена уже проверены");
                // Арность уже проверена (wrong-arguments): длины совпадают.
                debug_assert_eq!(cycle.params.len(), args.len());
                let mut child = env.clone();
                let mut params = Vec::with_capacity(cycle.params.len());
                for (param, arg) in cycle.params.iter().zip(args.iter()) {
                    let v = eval_expr_with_env(arg, &at, ctx.defs, env)?;
                    params.push((param.clone(), v.clone()));
                    child.insert(param.clone(), v);
                }
                events.push(Candidate {
                    point: None,
                    cycle: Some(CycleView::Cycle {
                        name: name.clone(),
                        duration_ms: duration_ms(&cycle.duration)?,
                        params: params.clone(),
                    }),
                    event: EventView {
                        action: None,
                        offset,
                        label,
                        at,
                        enabled: true,
                    },
                });
                let limit = duration_ms(&cycle.duration)?;
                let child_span = Span {
                    cycle: name.clone(),
                    start: clamp_i64(base),
                    end: clamp_i64(base + limit as i128),
                };
                let child_stack: Vec<StackFrame> = stack
                    .iter()
                    .cloned()
                    .chain(std::iter::once(StackFrame {
                        name: name.clone(),
                        params,
                    }))
                    .collect();
                let mut child_events = Vec::new();
                let plans = plan_stmts(&cycle.stmts, limit, &cycle.duration.raw, ctx.tables)?;
                // Курсор инстанса цикла (issue 45): свой на каждый вызов.
                let mut child_cursor = Cursor::default();
                for (st, pl) in cycle.stmts.iter().zip(plans.iter()) {
                    if matches!(st.moment, crate::parser::MomentExpr::Absolute { .. }) {
                        unfold_stmt(
                            st,
                            &pl.starts,
                            None,
                            base,
                            k,
                            &child_span,
                            ctx,
                            &child,
                            &child_stack,
                            &mut child_events,
                            zone_ms,
                            &mut child_cursor,
                        )?;
                    } else {
                        unfold_stmt_dynamic(
                            st,
                            base,
                            k,
                            &child_span,
                            ctx,
                            &child,
                            &child_stack,
                            &mut child_events,
                            zone_ms,
                            limit,
                            &cycle.duration.raw,
                            &mut child_cursor,
                        )?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// Развёртка вызова рутины: инстанцирование с таблицей; тело — в окружении
/// вызывающего плюс данные, вызовы слотов — в пустом окружении (данные рутины им
/// недоступны статически, а чужое окружение затирало бы глобальные имена:
/// резолв идёт `env` раньше `defs`).
/// Табличный параметр из окружения затирается (в условиях он невидим — unknown-name).
/// Спан именуется рутиной (см. docs/reference/output.md: в спанах светится её имя).
/// Тело и вызовы слотов делят контекст `here`: вызовы слотов видят итоговый
/// список тела (детерминирован) — так чинится «обед всегда».
#[allow(clippy::too_many_arguments)]
fn unfold_routine(
    name: &str,
    args: &[Expr],
    row_label: Option<&str>,
    offset: i64,
    base: i128,
    k: i128,
    ctx: &mut Ctx<'_, '_, '_, '_>,
    env: &HashMap<String, Value>,
    stack: &[StackFrame],
    events: &mut Vec<Candidate>,
    zone_ms: i64,
) -> Result<(), Error> {
    let at = AtFrame::new(i64::try_from(base).unwrap_or(i64::MAX), zone_ms);
    let routine = ctx.tables.routines.get(name).expect("имена уже проверены");
    // Таблица — литеральная: пробросы подставлены при инстанцировании
    // родительской рутины, в циклах/корне — только литералы по валидации.
    // Арность уже проверена (wrong-arguments): длины совпадают.
    debug_assert_eq!(routine.params.len(), args.len());
    let table_name = call_table_name(args, name)?;
    let (params, child) = routine_call_values(routine, &table_name, args, &at, ctx, env)?;
    let table = ctx
        .tables
        .tables
        .get(table_name.as_str())
        .expect("таблица проверена");
    let labels = match params.first() {
        Some((_, Value::Map(pairs))) => pairs
            .iter()
            .find_map(|(k, v)| (k == "labels").then(|| v.clone()))
            .and_then(|v| match v {
                Value::Map(xs) => Some(xs),
                _ => None,
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    events.push(Candidate {
        point: None,
        cycle: Some(CycleView::Routine {
            name: name.to_owned(),
            params: params.clone(),
            labels,
        }),
        event: EventView {
            action: None,
            offset,
            label: row_label.map(str::to_owned),
            at,
            enabled: true,
        },
    });
    let limit = duration_ms(&table.duration)?;
    let child_span = Span {
        cycle: name.to_owned(),
        start: clamp_i64(base),
        end: clamp_i64(base + limit as i128),
    };
    let child_stack: Vec<StackFrame> = stack
        .iter()
        .cloned()
        .chain(std::iter::once(StackFrame {
            name: name.to_owned(),
            params,
        }))
        .collect();
    let inst = instantiate(routine, &table_name, ctx.tables)?;
    // Тело и вызовы слотов делят один таймлайн: занятость течёт из тела в вызовы слотов
    // (как в `check_tables`, где списки склеиваются). Контекст `here` — свой
    // на инстанцию: тело пишет, вызовы слотов читают итог.
    let mut occupied: Vec<(i64, i64)> = Vec::new();
    let mut routine_events = Vec::new();
    // Тела рутин — только абсолютные моменты (проверяет `validate_moments`):
    // курсор scope здесь не нужен, меток `as` в рутинах нет по грамматике.
    let mut routine_cursor = Cursor::default();
    let body = plan_stmts_with(
        &inst.body,
        limit,
        &table.duration.raw,
        ctx.tables,
        &mut occupied,
    )?;
    for ((st, pl), lab) in inst
        .body
        .iter()
        .zip(body.iter())
        .zip(inst.body_labels.iter())
    {
        unfold_stmt(
            st,
            &pl.starts,
            lab.as_deref(),
            base,
            k,
            &child_span,
            ctx,
            &child,
            &child_stack,
            &mut routine_events,
            zone_ms,
            &mut routine_cursor,
        )?;
    }
    let fresh: HashMap<String, Value> = HashMap::new();
    let slot_calls = plan_stmts_with(
        &inst.slot_calls,
        limit,
        &table.duration.raw,
        ctx.tables,
        &mut occupied,
    )?;
    for ((st, pl), lab) in inst
        .slot_calls
        .iter()
        .zip(slot_calls.iter())
        .zip(inst.slot_labels.iter())
    {
        unfold_stmt(
            st,
            &pl.starts,
            Some(lab.as_str()),
            base,
            k,
            &child_span,
            ctx,
            &fresh,
            &child_stack,
            &mut routine_events,
            zone_ms,
            &mut routine_cursor,
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cond::{Defs, TableReg, check_conditions, resolve_units};
    use crate::datetime::{format_datetime, parse_datetime};
    use crate::validate::{check_bounds, check_recursion, check_tables, validate_names};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn setup(
        src: &str,
    ) -> (
        &'static crate::parser::Schedule,
        NameTables<'static>,
        &'static Defs,
    ) {
        let file: &'static mut crate::parser::SourceFile =
            Box::leak(Box::new(crate::parser::parse(src).unwrap()));
        crate::reverse::materialize_reverse(&mut file.schedule).unwrap();
        let ast: &'static crate::parser::Schedule = &file.schedule;
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
        // Контракт (examples/valid/route.*): пятница 09.01 — полное расписание, 18 событий как в JSON.
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-09T00:00:00", "2026-01-10T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
    fn naive_file_inherits_query_zone_walls_stand() {
        // Наивный файл + aware-окно: стены стоят (06:00 остаётся 06:00 в зоне
        // окна), инстанты = стена − зона окна. См. docs/reference/semantics.md.
        // Файл без условий: календарные встроенные (`hour`, `morning`, …)
        // считают от абсолютных мс и кадру не подчиняются (граница модели).
        use crate::datetime::{format_datetime_tz, parse_datetime_zoned};
        let src = include_str!("../../../examples/valid/tz_offsets.cyclo");
        let (ast, t, d) = setup(src);
        let (s, _) = parse_datetime_zoned("2026-01-09T00:00:00+03:00").unwrap();
        let (e, _) = parse_datetime_zoned("2026-01-10T00:00:00+03:00").unwrap();
        let events = expand(ast, &t, d, s, e, Some(180)).unwrap();
        let got: Vec<String> = events
            .iter()
            .map(|ev| format_datetime_tz(ev.time, Some(180)))
            .collect();
        assert_eq!(
            got,
            vec![
                "2026-01-09T06:00:00+03:00",
                "2026-01-09T07:00:00+03:00",
                "2026-01-09T18:00:00+03:00",
                "2026-01-09T19:00:00+03:00",
            ]
        );
        // Зона файла бьёт зону окна: тот же запрос к файлу +03:00 с окном +02:00
        // даёт стены в +03:00, а не в +02:00.
        let src_z = include_str!("../../../examples/valid/tz_file.cyclo");
        let (az, tz, dz) = setup(src_z);
        let (sz, _) = parse_datetime_zoned("2026-01-09T00:00:00+02:00").unwrap();
        let (ez, _) = parse_datetime_zoned("2026-01-10T00:00:00+02:00").unwrap();
        let events_z = expand(az, &tz, dz, sz, ez, Some(120)).unwrap();
        assert_eq!(events_z.len(), 1);
        assert_eq!(
            format_datetime_tz(events_z[0].time, Some(180)),
            "2026-01-09T06:00:00+03:00"
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
            let full = expand(ast, &t, d, from, from + within, None).unwrap();
            let want: Vec<Event> = full.into_iter().take(n).collect();
            let got = next_events(ast, &t, d, from, within, n, None).unwrap();
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
        let got = next_events(ast, &t, d, from, 366 * 86_400_000, 3, None).unwrap();
        assert_eq!(times(&got), vec!["2026-12-31T12:00:00"]);
        // Капа не хватает — пусто без ошибки.
        let got = next_events(ast, &t, d, from, 30 * 86_400_000, 3, None).unwrap();
        assert_eq!(got, vec![]);
    }

    #[test]
    fn next_counts_events_not_instants() {
        // n считает события: связка 10:20 (arrive+depart) режется пополам.
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        let from = parse_datetime("2026-01-09T10:20:00").unwrap();
        let got = next_events(ast, &t, d, from, 86_400_000, 1, None).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(format_datetime(got[0].time), "2026-01-09T10:20:00");
    }

    #[test]
    fn empty_window_and_window_before_anchor() {
        let src = include_str!("../../../examples/valid/route.cyclo");
        let (ast, t, d) = setup(src);
        // end <= start — пусто без ошибки.
        let (s, e) = window("2026-01-11T00:00:00", "2026-01-10T00:00:00");
        assert_eq!(expand(ast, &t, d, s, e, None).unwrap(), vec![]);
        // Окно целиком до start_time — пусто.
        let (s, e) = window("2025-12-30T00:00:00", "2025-12-31T00:00:00");
        assert_eq!(expand(ast, &t, d, s, e, None).unwrap(), vec![]);
        // Окно встык к границе экземпляра: событие на end не входит.
        let (s, e) = window("2026-01-10T06:00:00", "2026-01-10T06:00:00");
        assert_eq!(expand(ast, &t, d, s, e, None).unwrap(), vec![]);
    }

    #[test]
    fn params_flow_into_blocks_and_conditions() {
        // Параметр виден и в блоках, и в условиях; условие режет строки.
        let src = "const LEC = {\"name\": \"БЖД\", \"type\": \"лек\"}; \
            const PR = {\"name\": \"БЖД\", \"type\": \"прак\"}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle LESSON(subj) duration = 1h { \
            0m: B.ring() {\"subject\": subj.name, \"event\": \"start\"}; \
            [subj.type == \"лек\"] 30m: B.ring() {\"subject\": subj.name, \"event\": \"extra\"}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 9h: LESSON(LEC); 13h: LESSON(PR); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
    fn names_in_literals_expand_dynamically() {
        // Имена/выражения внутри словарей считаются в момент использования:
        // const-словарь ссылается на другие const, инлайновый литерал видит
        // параметр, а блок-ссылка на параметр-мапу едет целиком.
        let src = "const BASE = \"БЖД\"; const ROOM = \"233/А\"; \
            const LEC = {\"subject\": BASE, \"room\": ROOM, \"label\": BASE ++ \"-\" ++ ROOM}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle LESSON(subj) duration = 1h { 0m: B.ring() subj; } \
            cycle WRAP(name) duration = 1h { 0m: LESSON({\"subject\": name, \"room\": ROOM}); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            9h: LESSON(LEC); 11h: WRAP(BASE); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(
            times(&events),
            vec!["2026-01-01T09:00:00", "2026-01-01T11:00:00"]
        );
        let str_ = |s: &str| crate::cond::Value::Str(s.to_owned());
        assert_eq!(
            events[0].action_attrs,
            vec![
                ("subject".to_owned(), str_("БЖД")),
                ("room".to_owned(), str_("233/А")),
                ("label".to_owned(), str_("БЖД-233/А")),
            ]
        );
        // Инлайновый литерал: `name` — параметр WRAP, `ROOM` — константа.
        assert_eq!(
            events[1].action_attrs,
            vec![
                ("subject".to_owned(), str_("БЖД")),
                ("room".to_owned(), str_("233/А")),
            ]
        );
    }

    #[test]
    fn index_expressions_evaluate_in_blocks() {
        // Питоновский минус, арифметика, параметр и вложенная цепочка.
        let src = "const TAGS = [\"поток\", \"утро\", \"БЖД\"]; const N = 3; \
            const LEC = {\"tags\": TAGS}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle LESSON(i) duration = 1h { 0m: B.ring() { \
            \"first\": TAGS[0], \"last\": TAGS[-1], \"sum\": TAGS[1 + 1], \
            \"param\": TAGS[i], \"nested\": LEC.tags[N - 1]}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: LESSON(1); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        let str_ = |s: &str| crate::cond::Value::Str(s.to_owned());
        assert_eq!(
            events[0].action_attrs,
            vec![
                ("first".to_owned(), str_("поток")),
                ("last".to_owned(), str_("БЖД")),
                ("sum".to_owned(), str_("БЖД")),
                ("param".to_owned(), str_("утро")),
                ("nested".to_owned(), str_("БЖД")),
            ]
        );
    }

    #[test]
    fn index_out_of_bounds_is_runtime() {
        // Слишком отрицательный индекс — index-out-of-bounds в момент строки.
        let src = "const TAGS = [\"a\"]; schedule \"T\" { point B { actions = [ring]; } \
            cycle L duration = 1h { 0m: B.ring() {\"x\": TAGS[-2]}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: L(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let err = expand(ast, &t, d, s, e, None).expect_err("индекс вне границ — ошибка");
        assert_eq!(
            (err.code, err.message.as_str()),
            ("index-out-of-bounds", "index out of bounds '-2'")
        );
    }

    #[test]
    fn inner_cycle_sees_outer_params() {
        // Динамический скоуп: безаргументный INNER видит subj вызывающего.
        let src = "const LEC = {\"name\": \"БЖД\", \"type\": \"лек\"}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle INNER duration = 30m { [subj.type == \"лек\"] 0m: B.ring() {\"subject\": subj.name}; } \
            cycle LESSON(subj) duration = 1h { 0m: INNER(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: LESSON(LEC); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
            cycle INNER(subj) duration = 30m { 0m: B.ring() {\"subject\": subj.name}; } \
            cycle OUTER(subj) duration = 1h { 0m: INNER(PR); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: OUTER(LEC); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
    fn unbound_param_is_runtime_unknown_name() {
        // Статика пропускает (имя — параметр LESSON), строка без связывания — unknown-name.
        let src = "const LEC = {\"name\": \"БЖД\"}; \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle LESSON(subj) duration = 1h { 0m: B.ring() {\"subject\": subj.name}; } \
            cycle INNER duration = 30m { 0m: B.ring() {\"subject\": subj.name}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 9h: INNER(); } }";
        let file = Box::leak(Box::new(crate::parser::parse(src).unwrap()));
        crate::reverse::materialize_reverse(&mut file.schedule).unwrap();
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
        let err = expand(ast, &t, d, s, e, None).expect_err("несвязанный параметр — ошибка");
        assert_eq!(
            (err.code, err.message.as_str()),
            ("unknown-name", "unknown name 'subj'")
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
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
        // Ошибки атрибутов — в начале развёртки (после всех проверок главы ошибок).
        for (decls, point, code, message) in [
            (
                "",
                "point A { actions = [x]; attrs = {\"a\": 1, \"a\": 2}; }",
                "duplicate-attribute",
                "duplicate attribute 'a'",
            ),
            (
                "",
                "point A { actions = [x]; attrs = NOPE; }",
                "unknown-name",
                "unknown name 'NOPE'",
            ),
            (
                "const N = 5;",
                "point A { actions = [x]; attrs = N; }",
                "type-mismatch",
                "type mismatch: cannot mix number and string",
            ),
            (
                "fun f(t) = t;",
                "point A { actions = [x]; attrs = f; }",
                "type-mismatch",
                "type mismatch: cannot mix number and string",
            ),
        ] {
            let src = format!(
                "{decls} schedule \"T\" {{ {point} \
                root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ 6h: A.x(); }} }}"
            );
            let file = Box::leak(src.into_boxed_str());
            let parsed = Box::leak(Box::new(crate::parser::parse(file).unwrap()));
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
            let err = expand(ast, &t, d, s, e, None).expect_err("атрибуты обязаны браковаться");
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
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
            expand(an, &tn, dn, s, e, None).unwrap(),
            expand(ap, &tp, dp, s, e, None).unwrap()
        );
        assert_eq!(
            times(&expand(an, &tn, dn, s, e, None).unwrap()),
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
            times(&expand(ast, &t, d, s, e, None).unwrap()),
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
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
        let fe = expand(af, &tf, df, s, e, None).unwrap();
        assert_eq!(fe, expand(au, &tu, du, s, e, None).unwrap());
        assert_eq!(fe.len(), 36);
    }

    #[test]
    fn fills_gaps_between_lessons() {
        // Дыра 1:00–1:30 после LESSON1; хвост за 2:30 в окно не входит.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle LESSON duration = 1h { 0m: A.x(); } \
            cycle BREAK20 duration = 20m { 0m: A.x(); } \
            cycle BREAK5 duration = 5m { 0m: A.x(); } \
            cycle DAY duration = 2h30m { \
            0h: LESSON(); 1h30m: LESSON(); \
            0h: fill gaps until 2h30m BREAK20(); \
            0h: fill gaps until 2h30m BREAK5(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: DAY(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e, None).unwrap()),
            vec![
                "2026-01-01T00:00:00",
                "2026-01-01T01:00:00",
                "2026-01-01T01:20:00",
                "2026-01-01T01:25:00",
                "2026-01-01T01:30:00",
            ]
        );
    }

    #[test]
    fn gaps_pack_directions() {
        // Окно DAY 2h30m: LESSON 0-60м и 90-150м, дыра 60-90м (30м).
        // BREAK20 (20м): left → 1:00, right → 1:10, center → 1:05.
        let head = "schedule \"T\" { point A { actions = [x]; } \
            cycle LESSON duration = 1h { 0m: A.x(); } \
            cycle BREAK20 duration = 20m { 0m: A.x(); } \
            cycle DAY duration = 2h30m { \
            0h: LESSON(); 1h30m: LESSON(); \
            0h: fill gaps";
        let tail = " until 2h30m BREAK20(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: DAY(); } }";
        for (pack, expected) in [
            ("", "2026-01-01T01:00:00"),
            ("pack left", "2026-01-01T01:00:00"),
            ("pack right", "2026-01-01T01:10:00"),
            ("pack center", "2026-01-01T01:05:00"),
        ] {
            let src = if pack.is_empty() {
                format!("{head}{tail}")
            } else {
                format!("{head} {pack}{tail}")
            };
            let (ast, t, d) = setup(&src);
            let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
            let got = times(&expand(ast, &t, d, s, e, None).unwrap());
            assert_eq!(
                got,
                vec!["2026-01-01T00:00:00", expected, "2026-01-01T01:30:00",],
                "pack `{pack}`"
            );
        }
    }

    #[test]
    fn gaps_pack_center_rounds_down() {
        // Дыра 30м, шаг 20м: остаток 10м делится пополам → сдвиг 5м вниз.
        // Дыра 25м (LESSON в 1h25m), шаг 20м: остаток 5м → сдвиг 2м30с вниз.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle LESSON duration = 1h { 0m: A.x(); } \
            cycle BREAK20 duration = 20m { 0m: A.x(); } \
            cycle DAY duration = 2h30m { \
            0h: LESSON(); 1h25m: LESSON(); \
            0h: fill gaps pack center until 2h30m BREAK20(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: DAY(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e, None).unwrap()),
            vec![
                "2026-01-01T00:00:00",
                "2026-01-01T01:02:30",
                "2026-01-01T01:25:00",
            ]
        );
    }

    #[test]
    fn gaps_without_until_fill_to_parent_end() {
        // Без until добивается весь свободный хвост до конца цикла.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            0h: R(); 0h: fill gaps R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let got = times(&expand(ast, &t, d, s, e, None).unwrap());
        assert_eq!(got.len(), 24);
        assert_eq!(got[1], "2026-01-01T01:00:00");
        assert_eq!(got[23], "2026-01-01T23:00:00");
    }

    #[test]
    fn gaps_condition_checked_per_instance() {
        // Условие на fill gaps — на каждый экземпляр в его старте (в дыре).
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            0h: R(); [hour(at) < 2] 0h: fill gaps R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e, None).unwrap()),
            vec!["2026-01-01T00:00:00", "2026-01-01T01:00:00"]
        );
    }

    #[test]
    fn skips_rows_with_false_condition() {
        // 2026-01-01T06:00:00 = 1767247200000 мс epoch.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [at.wall >= 1767247200000] 6h: R(); [at.wall < 1767290400000] 18h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e, None).unwrap()),
            vec!["2026-01-01T06:00:00"]
        );
    }

    #[test]
    fn false_parent_kills_nested_events() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle INNER duration = 1h { 0m: A.x(); } \
            cycle OUTER duration = 2h { 0m: INNER(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [at.wall < 0] 6h: OUTER(); 6h: OUTER(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e, None).unwrap()),
            vec!["2026-01-01T06:00:00"]
        );
    }

    #[test]
    fn condition_filters_repeat_instances() {
        // `fill` без условия дал бы 18 экземпляров; условие режет все после 01:20.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [at.wall < 1767231000000] 0h: fill R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        assert_eq!(
            times(&expand(ast, &t, d, s, e, None).unwrap()),
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
            times(&expand(ast, &t, d, s, e, None).unwrap()),
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
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(
            times(&events),
            vec!["2026-01-02T06:40:00", "2026-01-02T06:50:00"]
        );
    }

    /// Сквозная рутина: метки из таблицы, обед как вызов слота только по будням,
    /// данные текут в условия и блоки, спаны именованы рутиной.
    /// 2026-09-07 — понедельник, 2026-09-12 — суббота.
    fn routine_src() -> &'static str {
        "time_const DAY duration = 24h { \
            1st: 9h; \
            [workday(at)] lunch: 12h -> LUNCH(); \
        } \
        schedule \"T\" { point A { actions = [x]; } point B { actions = [y]; } \
        routine M(TC, subj) { [subj == 1] 1st: A.x() {\"n\": subj}; } \
        cycle LUNCH duration = 30m { 0m: B.y(); } \
        root_cycle start_time = \"2026-09-07T00:00:00\", duration = 24h { \
        [day_of_week(at) == 1] 0h: M(DAY, 1); \
        [day_of_week(at) == 6] 0h: M(DAY, 1); } }"
    }

    #[test]
    fn expands_routine_with_labels_and_slot_call() {
        let (ast, t, d) = setup(routine_src());
        let (s, e) = window("2026-09-07T00:00:00", "2026-09-13T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
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
        // Спаны: строки рутины — её именем, вызов слота — внутренним циклом
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
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(times(&events), vec!["2026-09-07T12:00:00"]);
    }

    #[test]
    fn day_of_week_matches_calendar() {
        // 2026-09-07 — понедельник (=1), 2026-09-13 — воскресенье (=7).
        let (ast, t, d) = setup(routine_src());
        let (s, e) = window("2026-09-07T00:00:00", "2026-09-08T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(events.len(), 2);
        for case in [("2026-09-07T00:00:00", 1), ("2026-09-13T00:00:00", 7)] {
            let abs = parse_datetime(case.0).unwrap();
            let at = AtFrame::new(abs, 0);
            let v = eval_expr_with_env(
                &crate::parser::Expr::Call {
                    name: "day_of_week".to_owned(),
                    args: vec![crate::parser::Expr::At],
                },
                &at,
                d,
                &HashMap::new(),
            )
            .unwrap();
            assert_eq!(v, Value::Num(case.1), "для {}", case.0);
        }
    }

    #[test]
    fn here_stack_reports_call_path() {
        // Стек — путь вызовов: корень → цикл; глубина = len(here.stack).
        let src = "schedule \"T\" { point B { actions = [ring]; } \
            cycle R duration = 1h { 0m: B.ring() \
            {\"depth\": len(here.stack), \"top\": here.stack[-1].name, \"root\": here.stack[0].name}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(events.len(), 1);
        let num = |n: i64| Value::Num(n);
        let str_ = |s: &str| Value::Str(s.to_owned());
        assert_eq!(
            events[0].action_attrs,
            vec![
                ("depth".to_owned(), num(2)),
                ("top".to_owned(), str_("R")),
                ("root".to_owned(), str_("root_cycle")),
            ]
        );
    }

    #[test]
    fn here_events_split_enabled_disabled() {
        // Все кандидаты — в here.events, выполненные/нет — в геттерах.
        // Записывающая строка — последняя: видит себя и двух предшественников.
        let src = "schedule \"T\" { point B { actions = [ring]; } \
            cycle R duration = 2h { \
            0m: B.ring(); \
            [false] 30m: B.ring(); \
            60m: B.ring() {\"all\": len(here.events), \"on\": len(here.enabled_events), \"off\": len(here.disabled_events)}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(
            times(&events),
            vec!["2026-01-01T06:00:00", "2026-01-01T07:00:00"]
        );
        let num = |n: i64| Value::Num(n);
        assert_eq!(
            events[1].action_attrs,
            vec![
                ("all".to_owned(), num(3)),
                ("on".to_owned(), num(2)),
                ("off".to_owned(), num(1)),
            ]
        );
    }

    #[test]
    fn slot_call_sees_finished_body() {
        // Вызов слота видит итоговый список тела: при пустом теле обеда нет.
        let src = "time_const DAY duration = 3h \
            { [len(here.enabled_events) > 0] lunch: 2h -> LUNCH(); } \
            schedule \"T\" { point B { actions = [ring]; } \
            cycle LUNCH duration = 30m { 0m: B.ring(); } \
            routine M(TC, flag) { [flag == 1] 0h: B.ring(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 0h: M(DAY, 1); 12h: M(DAY, 0); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        // Тело с flag=1 даёт звонок в 00:00 и обед в 02:00; тело с flag=0 — ничего.
        assert_eq!(
            times(&events),
            vec!["2026-01-01T00:00:00", "2026-01-01T02:00:00"]
        );
    }

    #[test]
    fn here_snapshot_freezes_in_block() {
        // {"snap": here} — мёртвый JSON момента: стек, кандидат, стена.
        let src = "schedule \"T\" { point B { actions = [ring]; } \
            cycle R duration = 1h { 0m: B.ring() {\"snap\": here}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(events.len(), 1);
        let wall = parse_datetime("2026-01-01T06:00:00").unwrap();
        let at = Value::Map(vec![
            ("wall".to_owned(), Value::Num(wall)),
            ("abs".to_owned(), Value::Num(wall)),
            ("zone".to_owned(), Value::Num(0)),
        ]);
        let candidate = Value::Map(vec![
            (
                "point".to_owned(),
                Value::Map(vec![
                    ("name".to_owned(), Value::Str("B".to_owned())),
                    (
                        "actions".to_owned(),
                        Value::Array(vec![Value::Str("ring".to_owned())]),
                    ),
                    ("attrs".to_owned(), Value::Map(vec![])),
                ]),
            ),
            (
                "event".to_owned(),
                Value::Map(vec![
                    ("action".to_owned(), Value::Str("ring".to_owned())),
                    ("offset".to_owned(), Value::Num(0)),
                    ("at".to_owned(), at),
                    ("enabled".to_owned(), Value::Bool(true)),
                ]),
            ),
        ]);
        let snap = Value::Map(vec![
            (
                "stack".to_owned(),
                Value::Array(vec![
                    Value::Map(vec![(
                        "name".to_owned(),
                        Value::Str("root_cycle".to_owned()),
                    )]),
                    Value::Map(vec![("name".to_owned(), Value::Str("R".to_owned()))]),
                ]),
            ),
            ("events".to_owned(), Value::Array(vec![candidate.clone()])),
            (
                "enabled_events".to_owned(),
                Value::Array(vec![candidate.clone()]),
            ),
            ("disabled_events".to_owned(), Value::Array(vec![])),
        ]);
        assert_eq!(events[0].action_attrs, vec![("snap".to_owned(), snap)]);
    }

    #[test]
    fn here_chains_reach_params_and_labels() {
        // Цепочки: параметры кадра, метка кандидата, метки таблицы.
        let src = "const SUBJ = {\"name\": \"БЖД\", \"teacher\": \"Иванов\"}; \
            time_const DAY duration = 2h { first: 0m; } \
            schedule \"T\" { point B { actions = [ring]; } \
            routine M(TC, subj) { first: B.ring() \
            {\"t\": here.stack[-1].params.subj.teacher, \
            \"lab\": here.events[0].event.label, \
            \"slot\": here.stack[-1].params.TC.labels.first}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(DAY, SUBJ); } }";
        let (ast, t, d) = setup(src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(events.len(), 1);
        let str_ = |s: &str| Value::Str(s.to_owned());
        assert_eq!(
            events[0].action_attrs,
            vec![
                ("t".to_owned(), str_("Иванов")),
                ("lab".to_owned(), str_("first")),
                ("slot".to_owned(), Value::Num(0)),
            ]
        );
    }

    #[test]
    fn julian_guard_rejects_bad_components() {
        // Страж julian_to_wall: кривой месяц — division-by-zero, не мусор.
        // Библиотека подключается из настоящего файла (include_str).
        let lib = include_str!("../../../examples/real/libs/julian.cyclo");
        let src = [
            lib,
            " schedule \"T\" { point B { actions = [ring]; } \
            cycle R duration = 1h { 0m: B.ring(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [julian_to_wall({\"y\": 2026, \"mo\": 13, \"d\": 1}) == at.wall] 6h: R(); } }",
        ]
        .concat();
        let (ast, t, d) = setup(&src);
        let (s, e) = window("2026-01-01T00:00:00", "2026-01-02T00:00:00");
        let err = expand(ast, &t, d, s, e, None).expect_err("страж обязан ронять");
        assert_eq!(err.code, "division-by-zero");
    }

    #[test]
    fn wall_calendar_uses_frame_zone() {
        // С timezone=+03:00 стена понедельника 07.09 — понедельник (dow==mon):
        // абсолют (воскресенье по UTC) дня бы не дал.
        let src = "schedule \"T\" { timezone = \"+03:00\" point B { actions = [ring]; } \
            cycle R duration = 1h { 0m: B.ring(); } \
            root_cycle start_time = \"2026-09-07T00:00:00\", duration = 24h { \
            [dow(at) == mon] 0h: R(); } }";
        let (ast, t, d) = setup(src);
        // Окно абсолютами: событие — 2026-09-06T21:00Z.
        let (s, e) = window("2026-09-06T21:00:00", "2026-09-07T21:00:00");
        let events = expand(ast, &t, d, s, e, None).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].time,
            parse_datetime("2026-09-06T21:00:00").unwrap()
        );
    }
}
