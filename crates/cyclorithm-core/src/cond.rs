//! Условия строк (§3–§5 спеки): объявления, подстановка, вычисление.
//!
//! Определения (`const`/`fun`/`pred` + системный файл) раскрываются
//! через окружение — для чистых выражений это та же подстановка.
//! Проверки в порядке объявления: дубли (E04), тела (E11/E12, рекурсия),
//! затем строки. Константное деление на ноль ловится статически,
//! деление нулём выражения — вычислением в момент строки (тоже E12).

use std::collections::{HashMap, HashSet};

use cyclorithm_parser::{
    ArithOp, BitOp, CmpOp, Cond, CondRhs, Decl, Duration, Expr, Invocation, Schedule, SlotRow,
};

use crate::validate::NameTables;
use crate::Error;

/// Значение выражения: число, строка или JSON-значение.
/// Мапы хранятся вектором пар (порядок ключей — порядок объявления).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Num(i64),
    Str(String),
    Bool(bool),
    Map(Vec<(String, Value)>),
    Array(Vec<Value>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Num,
    Str,
    /// Динамическое данное: параметр цикла или поле/индекс. Статика пропускает
    /// любые операции, ошибки — в момент строки (E12). См. черновик `attrs.md`.
    Dyn,
    Map,
    Array,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefKind {
    Const,
    Fun,
    Pred,
}

#[derive(Debug, Clone)]
struct Def {
    kind: DefKind,
    param: Option<String>,
    expr: Option<Expr>,
    cond: Option<Cond>,
    /// Файл-владелец: 0 — прелюдия, дальше импорты, последний — программа.
    unit: usize,
    /// Имена на `__` видны только в своём файле.
    private: bool,
}

/// Итоговое пространство имён: прелюдия, поверх импорты, поверх программа.
// Побеждает последнее; `__` чужого файла не видно.
#[derive(Debug)]
pub struct Defs {
    map: HashMap<String, Def>,
    /// Юнит тела программы: контекст условий расписания.
    main: usize,
}

static PRELUDE: &str = include_str!("std.cyclo");

/// Таблица слотов `time_const`: длительность, строки и юнит исходника
/// (для `__`-видимости условий `->`-строк — как у `Def.unit`).
#[derive(Debug, Clone)]
pub struct TimeTable {
    pub name: String,
    pub duration: Duration,
    pub rows: Vec<SlotRow>,
    pub unit: usize,
}

/// Реестр таблиц рядом с `Defs`: последнее объявление побеждает.
#[derive(Debug, Default, Clone)]
pub struct TableReg {
    pub tables: HashMap<String, TimeTable>,
    /// Порядок первого объявления (импорты, затем программа) —
    /// для детерминированного обхода в проверках.
    pub order: Vec<String>,
}

impl TableReg {
    /// Таблица по имени (таблицы живут в своём пространстве имён).
    pub fn get(&self, name: &str) -> Option<&TimeTable> {
        self.tables.get(name)
    }
}

/// Собрать определения одного файла поверх системных.
pub fn resolve_defs(decls: &[Decl]) -> Result<(Defs, TableReg), Error> {
    resolve_units(&[decls.to_vec()])
}

/// Собрать определения: оверлей групп в порядке наложения
// (последняя группа — тело программы), затем проверить все тела.
// Дубли — только внутри одной группы (`E04`); между файлами побеждает последнее.
// Таблицы (`time_const`) — отдельным реестром: своё пространство имён
// (позиции ссылок не пересекаются с выражениями), дубли — `duplicate table`.
pub fn resolve_units(units: &[Vec<Decl>]) -> Result<(Defs, TableReg), Error> {
    let system = cyclorithm_parser::parse_decls(PRELUDE).expect("прелюдия обязана разбираться");
    let mut map = HashMap::new();
    let mut all: Vec<String> = Vec::new();
    for d in &system {
        let (name, def) = to_def(d, 0);
        if !map.contains_key(&name) {
            all.push(name.clone());
        }
        map.insert(name, def);
    }
    let mut tables = TableReg::default();
    for (i, group) in units.iter().enumerate() {
        let unit = i + 1;
        let mut seen = HashSet::new();
        let mut seen_tables = HashSet::new();
        for d in group {
            if let Decl::TimeConst {
                name,
                duration,
                rows,
            } = d
            {
                if !seen_tables.insert(name.clone()) {
                    return Err(Error::e04("table", name));
                }
                if !tables.tables.contains_key(name) {
                    tables.order.push(name.clone());
                }
                tables.tables.insert(
                    name.clone(),
                    TimeTable {
                        name: name.clone(),
                        duration: duration.clone(),
                        rows: rows.clone(),
                        unit,
                    },
                );
                continue;
            }
            let (name, kind) = match d {
                Decl::Const { name, .. } => (name, "const"),
                Decl::Fun { name, .. } => (name, "fun"),
                Decl::Pred { name, .. } => (name, "pred"),
                Decl::TimeConst { .. } => unreachable!("таблицы разобраны выше"),
            };
            if !seen.insert(name.clone()) {
                return Err(Error::e04(kind, name));
            }
            all.push(name.clone());
            let (_, def) = to_def(d, unit);
            map.insert(name.clone(), def);
        }
    }
    let defs = Defs {
        map,
        main: units.len(),
    };
    for name in &all {
        check_def(&defs, name)?;
    }
    Ok((defs, tables))
}

fn to_def(decl: &Decl, unit: usize) -> (String, Def) {
    match decl {
        Decl::TimeConst { .. } => unreachable!("таблицы в Defs не попадают"),
        Decl::Const { name, body } => (
            name.clone(),
            Def {
                kind: DefKind::Const,
                param: None,
                expr: Some(body.clone()),
                cond: None,
                unit,
                private: name.starts_with("__"),
            },
        ),
        Decl::Fun { name, param, body } => (
            name.clone(),
            Def {
                kind: DefKind::Fun,
                param: Some(param.clone()),
                expr: Some(body.clone()),
                cond: None,
                unit,
                private: name.starts_with("__"),
            },
        ),
        Decl::Pred { name, body } => (
            name.clone(),
            Def {
                kind: DefKind::Pred,
                param: Some("at".to_owned()),
                expr: None,
                cond: Some(body.clone()),
                unit,
                private: name.starts_with("__"),
            },
        ),
    }
}

fn check_def(defs: &Defs, name: &str) -> Result<(), Error> {
    let def = defs.map.get(name).expect("своё имя");
    // Тело проверяется изнутри своего файла: `__` видно.
    let mut cx = CxTy {
        defs,
        stack: vec![(name.to_owned(), def.unit)],
        unit: def.unit,
        vars: HashMap::new(),
        // Тела объявлений — позиция данных: `const M = {...}`, `const A = M`.
        data: true,
    };
    match def.kind {
        // Константа — любое значение (число, строка, мапа, ...), не только Num:
        // данные живут в константах (`BJD_LECTURE`), условия их не видят (E11).
        DefKind::Const => {
            let body = def.expr.as_ref().expect("const: тело");
            cx.infer(body)?;
            Ok(())
        }
        DefKind::Fun => {
            let body = def.expr.as_ref().expect("fun: тело");
            cx.vars
                .insert(def.param.clone().expect("fun: параметр"), Ty::Num);
            cx.infer(body)?;
            Ok(())
        }
        DefKind::Pred => {
            let body = def.cond.as_ref().expect("pred: тело");
            cx.vars.insert("at".to_owned(), Ty::Num);
            cx.data = false;
            cx.infer_cond(body)?;
            Ok(())
        }
    }
}

struct CxTy<'a> {
    defs: &'a Defs,
    stack: Vec<(String, usize)>,
    unit: usize,
    vars: HashMap<String, Ty>,
    /// Позиция данных (`true`): голые имена констант-мап/массивов/bool видны.
    /// В условиях (`false`) они — E11 (в скоупе только числа, строки и `at`).
    data: bool,
}

struct CxEv<'a> {
    defs: &'a Defs,
    stack: Vec<(String, usize)>,
    unit: usize,
    vars: HashMap<String, Value>,
}

fn resolve<'a>(defs: &'a Defs, name: &str, unit: usize) -> Result<&'a Def, Error> {
    match defs.map.get(name) {
        Some(d) if d.private && d.unit != unit => Err(Error::e11(name)),
        Some(d) => Ok(d),
        None => Err(Error::e11(name)),
    }
}

/// Проверить все условия файла в порядке объявления: циклы, рутины, корень,
/// затем пожары `->` таблиц.
/// Заодно — аргументы вызовов и блоки действий (та же фаза E11/E12:
/// сначала условие строки, затем вызов — как в момент развёртки).
/// У вызова рутины первый аргумент — таблица (не выражение): пропускается.
/// Табличный параметр рутины в условиях невидим (E11): при развёртке он
/// затирается из окружения; в скоупе только данные (`params[1..]`).
pub fn check_conditions(
    schedule: &Schedule,
    defs: &Defs,
    tables: &NameTables<'_>,
) -> Result<(), Error> {
    // Динамический скоуп: строка видит параметры любого цикла и данные
    // любой рутины (связываются в момент вызова); статика знает только имена.
    let mut params: HashMap<String, Ty> = HashMap::new();
    for c in &schedule.cycles {
        for p in &c.params {
            params.insert(p.clone(), Ty::Dyn);
        }
    }
    for r in &schedule.routines {
        for p in r.params.iter().skip(1) {
            params.insert(p.clone(), Ty::Dyn);
        }
    }
    for c in &schedule.cycles {
        for st in &c.stmts {
            check_row(st.condition.as_ref(), &st.invocation, defs, &params, tables)?;
        }
    }
    for r in &schedule.routines {
        let mut scope = params.clone();
        scope.remove(&r.params[0]);
        for st in &r.stmts {
            check_row(st.condition.as_ref(), &st.invocation, defs, &scope, tables)?;
        }
    }
    for st in &schedule.root.stmts {
        check_row(st.condition.as_ref(), &st.invocation, defs, &params, tables)?;
    }
    // Пожары таблиц — без параметров (данные рутин им недоступны статически;
    // при развёртке пожар выполняется в пустом окружении).
    let empty: HashMap<String, Ty> = HashMap::new();
    for tname in &tables.tables.order {
        let t = tables
            .tables
            .get(tname.as_str())
            .expect("порядок — по реестру");
        for row in &t.rows {
            if let Some(firing) = &row.firing {
                check_row(row.condition.as_ref(), firing, defs, &empty, tables)?;
            }
        }
    }
    Ok(())
}

/// Проверить одну строку: условие, затем вызов (аргументы/блок).
fn check_row(
    condition: Option<&Cond>,
    invocation: &Invocation,
    defs: &Defs,
    params: &HashMap<String, Ty>,
    tables: &NameTables<'_>,
) -> Result<(), Error> {
    let mut cx = CxTy {
        defs,
        stack: Vec::new(),
        unit: defs.main,
        vars: params.clone(),
        data: false,
    };
    if let Some(cond) = condition {
        cx.infer_cond(cond)?;
    }
    // Аргументы и значения блока — позиция данных (мапы!): типы любые,
    // видны и голые имена констант-данных.
    cx.data = true;
    match invocation {
        Invocation::PointAction { block, .. } => {
            // Дубли ключей блока — E15 (порядок объявления).
            let mut seen = HashSet::new();
            for (k, _) in block {
                if !seen.insert(k) {
                    return Err(Error::e15(k));
                }
            }
            for (_, v) in block {
                cx.infer(v)?;
            }
        }
        Invocation::CycleCall { name, args } => {
            // Первый аргумент рутины — таблица, не выражение.
            if tables.routines.contains_key(name.as_str()) {
                for a in args.iter().skip(1) {
                    cx.infer(a)?;
                }
            } else {
                for a in args {
                    cx.infer(a)?;
                }
            }
        }
    }
    Ok(())
}

impl CxTy<'_> {
    fn infer_cond(&mut self, cond: &Cond) -> Result<(), Error> {
        match cond {
            Cond::Or(cs) | Cond::And(cs) => cs.iter().try_for_each(|c| self.infer_cond(c)),
            Cond::Not(c) => self.infer_cond(c),
            Cond::Pred { name, args } => {
                let arg = match args.as_slice() {
                    [a] => a,
                    _ => return Err(Error::e12_arity(name)),
                };
                // Аргумент предиката — число; Dyn (параметр/поле) пропускаем,
                // момент строки проверит (E12).
                match self.infer(arg)? {
                    Ty::Num | Ty::Dyn => {}
                    _ => return Err(Error::e12_mismatch()),
                }
                let defs = self.defs;
                let def = resolve(defs, name, self.unit)?;
                match def.kind {
                    DefKind::Pred => {}
                    _ => return Err(Error::e12_not_pred(name)),
                };
                self.enter(name, def.unit)?;
                let body = def.cond.clone().expect("pred: тело");
                let r = self.infer_cond(&body);
                self.leave();
                r
            }
            Cond::Cmp { left, right, .. } => {
                let lt = self.infer(left)?;
                match right {
                    CondRhs::One(r) => {
                        let rt = self.infer(r)?;
                        // Bool в условиях нет: любое участие — сразу E12.
                        if lt == Ty::Bool || rt == Ty::Bool {
                            return Err(Error::e12_mismatch());
                        }
                        let collection = |t: Ty| matches!(t, Ty::Map | Ty::Array);
                        match (lt, rt) {
                            // Динамика: статика пропускает, разберётся строка.
                            (Ty::Dyn, _) | (_, Ty::Dyn) => Ok(()),
                            // Мапа/массив с мапой/массивом — «пока»
                            // (глубокое сравнение — будущее решение).
                            (a, b) if collection(a) && collection(b) => Err(Error::e12_map_cmp()),
                            (a, b) if a == b => Ok(()),
                            _ => {
                                if date_cmp_ok(left, r)? {
                                    Ok(())
                                } else {
                                    Err(Error::e12_mismatch())
                                }
                            }
                        }
                    }
                    CondRhs::Alt(alts) => {
                        for a in alts {
                            match self.infer(a)? {
                                Ty::Num | Ty::Dyn => {}
                                _ => return Err(Error::e12_mismatch()),
                            }
                        }
                        match lt {
                            Ty::Num | Ty::Dyn => Ok(()),
                            _ => Err(Error::e12_mismatch()),
                        }
                    }
                }
            }
        }
    }

    fn infer(&mut self, expr: &Expr) -> Result<Ty, Error> {
        match expr {
            Expr::Num(raw) => {
                raw.parse::<i64>().map_err(|_| Error::e12_range(raw))?;
                Ok(Ty::Num)
            }
            Expr::Str(_) => Ok(Ty::Str),
            Expr::Bool(_) => Ok(Ty::Bool),
            Expr::Map(pairs) => {
                check_map_dupes(pairs)?;
                // Значения — литералы по грамматике; типы всё равно выводим
                // (дубли и кривые числа ловятся здесь же).
                for (_, v) in pairs {
                    self.infer(v)?;
                }
                Ok(Ty::Map)
            }
            Expr::Array(xs) => {
                for x in xs {
                    self.infer(x)?;
                }
                Ok(Ty::Array)
            }
            // Поля данных динамические: статически не проверяются
            // (даже когда мапа известна) — только момент строки.
            // Исключение — голое имя под доступом (см. ниже).
            Expr::Field { base, .. } | Expr::Index { base, .. } => {
                // Базу выводим ради её проверок (вложенные доступы, E11 имён);
                // сам доступ всегда Dyn (см. ниже).
                self.infer(base)?;
                if let Expr::Name(name) = base.as_ref() {
                    // Параметр — динамика, пропускаем.
                    if !self.vars.contains_key(name) {
                        let defs = self.defs;
                        let def = resolve(defs, name, self.unit)?;
                        if def.kind == DefKind::Const {
                            let ty = self.const_body_ty(name, def)?;
                            self.hide_data(name, ty)?;
                        }
                    }
                }
                Ok(Ty::Dyn)
            }
            Expr::At => Ok(Ty::Num),
            Expr::Name(name) => {
                if let Some(ty) = self.vars.get(name) {
                    return Ok(*ty);
                }
                // Голая константа (K, DAY); fun/pred без вызова — не значение.
                let defs = self.defs;
                let def = resolve(defs, name, self.unit)?;
                match def.kind {
                    DefKind::Const => {
                        let ty = self.const_body_ty(name, def)?;
                        // Данные напрямую в условии невидимы: только E11.
                        self.hide_data(name, ty)
                    }
                    DefKind::Fun | DefKind::Pred => Err(Error::e12_mismatch()),
                }
            }
            Expr::Neg(x) => match self.infer(x)? {
                Ty::Num => Ok(Ty::Num),
                Ty::Dyn => Ok(Ty::Dyn),
                _ => Err(Error::e12_mismatch()),
            },
            Expr::Truth(c) => {
                self.infer_cond(c)?;
                Ok(Ty::Num)
            }
            Expr::Bin { left, right, .. } => {
                let ty = match (self.infer(left)?, self.infer(right)?) {
                    (Ty::Num, Ty::Num) => Ty::Num,
                    (Ty::Num | Ty::Dyn, Ty::Num | Ty::Dyn) => Ty::Dyn,
                    _ => return Err(Error::e12_mismatch()),
                };
                if let Some(Err(e)) = self.const_div(right) {
                    return Err(e);
                }
                Ok(ty)
            }
            // Битовые — те же числовые операнды, но деления нет:
            // статической проверки делителя не требуется, сдвиг не ошибается.
            Expr::Bit { left, right, .. } => match (self.infer(left)?, self.infer(right)?) {
                (Ty::Num, Ty::Num) => Ok(Ty::Num),
                (Ty::Num | Ty::Dyn, Ty::Num | Ty::Dyn) => Ok(Ty::Dyn),
                _ => Err(Error::e12_mismatch()),
            },
            Expr::Concat(xs) => {
                for x in xs {
                    match self.infer(x)? {
                        Ty::Str | Ty::Dyn => {}
                        _ => return Err(Error::e12_mismatch()),
                    }
                }
                Ok(Ty::Str)
            }
            Expr::Call { name, args } => self.infer_call(name, args),
        }
    }

    fn infer_call(&mut self, name: &str, args: &[Expr]) -> Result<Ty, Error> {
        match name {
            "str" | "pad" if !self.defs.map.contains_key(name) => {
                let want = if name == "str" { 1 } else { 2 };
                if args.len() != want {
                    return Err(Error::e12_arity(name));
                }
                for a in args {
                    match self.infer(a)? {
                        Ty::Num | Ty::Dyn => {}
                        _ => return Err(Error::e12_mismatch()),
                    }
                }
                Ok(Ty::Str)
            }
            // Делимые функции — те же операторы, вызванные явно (так пишет прелюдия).
            "floordiv" | "floormod" if !self.defs.map.contains_key(name) => {
                let [a, b] = args else {
                    return Err(Error::e12_arity(name));
                };
                match (self.infer(a)?, self.infer(b)?) {
                    (Ty::Num, Ty::Num) => {}
                    (Ty::Num | Ty::Dyn, Ty::Num | Ty::Dyn) => {}
                    _ => return Err(Error::e12_mismatch()),
                }
                if let Some(Err(e)) = self.const_div(b) {
                    return Err(e);
                }
                Ok(Ty::Num)
            }
            // Конструктор даты — встроенная функция (§4.13): ровно 7 чисел.
            // Все-константа проверяется сразу (как константный ноль у деления),
            // иначе — в момент строки.
            "mkdate" if !self.defs.map.contains_key(name) => {
                if args.len() != 7 {
                    return Err(Error::e12_arity(name));
                }
                for a in args {
                    match self.infer(a)? {
                        Ty::Num | Ty::Dyn => {}
                        _ => return Err(Error::e12_mismatch()),
                    }
                }
                let mut vals = Vec::with_capacity(7);
                for a in args {
                    match const_eval(a, self) {
                        None => return Ok(Ty::Num),
                        Some(Err(e)) => return Err(e),
                        Some(Ok(Value::Num(v))) => vals.push(v),
                        Some(Ok(_)) => return Err(Error::e12_mismatch()),
                    }
                }
                build_date(&vals, name)?;
                Ok(Ty::Num)
            }
            _ => {
                let defs = self.defs;
                let def = resolve(defs, name, self.unit)?;
                match def.kind {
                    DefKind::Const => {
                        if !args.is_empty() {
                            return Err(Error::e12_arity(name));
                        }
                        let ty = self.const_body_ty(name, def)?;
                        // Как голое имя: данные через вызов в условии невидимы.
                        self.hide_data(name, ty)
                    }
                    DefKind::Fun => {
                        let arg = match args {
                            [a] => a,
                            _ => return Err(Error::e12_arity(name)),
                        };
                        let arg_ty = self.infer(arg)?;
                        self.enter(name, def.unit)?;
                        let param = def.param.clone().expect("fun: параметр");
                        let body = def.expr.clone().expect("fun: тело");
                        // Восстановить внешнее значение: параметры вложенных
                        // вызовов часто зовутся так же (`t` в прелюдии).
                        let old = self.vars.insert(param.clone(), arg_ty);
                        let ty = self.infer(&body);
                        match old {
                            Some(v) => {
                                self.vars.insert(param, v);
                            }
                            None => {
                                self.vars.remove(&param);
                            }
                        }
                        self.leave();
                        ty
                    }
                    DefKind::Pred => Err(Error::e12_mismatch()),
                }
            }
        }
    }

    /// Тип тела константы. Тело — всегда позиция данных (`const A = M` —
    /// алиас мапы); видимость решает место использования (`hide_data`).
    fn const_body_ty(&mut self, name: &str, def: &Def) -> Result<Ty, Error> {
        self.enter(name, def.unit)?;
        let body = def.expr.clone().expect("const: тело");
        let saved = std::mem::replace(&mut self.data, true);
        let ty = self.infer(&body);
        self.data = saved;
        self.leave();
        ty
    }

    /// Данные по голому имени в позиции условия невидимы: только E11.
    fn hide_data(&self, name: &str, ty: Ty) -> Result<Ty, Error> {
        if !self.data && matches!(ty, Ty::Map | Ty::Array | Ty::Bool) {
            return Err(Error::e11(name));
        }
        Ok(ty)
    }

    fn enter(&mut self, name: &str, unit: usize) -> Result<(), Error> {
        if self.stack.iter().any(|(n, _)| n == name) {
            return Err(Error::e12_recursive(name));
        }
        let prev = std::mem::replace(&mut self.unit, unit);
        self.stack.push((name.to_owned(), prev));
        Ok(())
    }

    fn leave(&mut self) {
        if let Some((_, prev)) = self.stack.pop() {
            self.unit = prev;
        }
    }

    fn const_div(&mut self, expr: &Expr) -> Option<Result<(), Error>> {
        match const_eval(expr, self)? {
            Err(e) => Some(Err(e)),
            Ok(Value::Num(0)) => Some(Err(Error::e12_divzero())),
            Ok(_) => Some(Ok(())),
        }
    }
}

/// Дубли ключей в литерале мапы — E15 (первый повтор в порядке объявления).
/// Вызывается из `infer`, поэтому покрывает все позиции литералов:
/// тела объявлений, условия, аргументы, блоки.
fn check_map_dupes(pairs: &[(String, cyclorithm_parser::Expr)]) -> Result<(), Error> {
    let mut seen = HashSet::new();
    for (k, _) in pairs {
        if !seen.insert(k) {
            return Err(Error::e15(k));
        }
    }
    Ok(())
}

/// Разрешить атрибуты точек: `attrs` каждой точки в готовый словарь.
/// Без `attrs` — пустой. Литерал — как есть (дубли — E15); ссылка —
/// тело константы-мапы (неизвестное имя — E11, не мапа — E12).
/// Значения — литералы по грамматике, вычисляются с `at = 0` без окружения.
/// Вызывать после всех проверок §5, в начале развёртки.
pub fn resolve_point_attrs(
    schedule: &Schedule,
    defs: &Defs,
) -> Result<HashMap<String, Vec<(String, Value)>>, Error> {
    let empty: HashMap<String, Value> = HashMap::new();
    let mut out = HashMap::new();
    for p in &schedule.points {
        let attrs = match &p.attrs {
            None => Vec::new(),
            Some(Expr::Map(pairs)) => {
                check_map_dupes(pairs)?;
                eval_attr_pairs(pairs, defs, &empty)?
            }
            Some(Expr::Name(name)) => {
                let def = resolve(defs, name, defs.main)?;
                match def.kind {
                    DefKind::Const => {}
                    _ => return Err(Error::e12_mismatch()),
                }
                match eval_expr_with_env(def.expr.as_ref().expect("const: тело"), 0, defs, &empty)?
                {
                    Value::Map(pairs) => pairs,
                    _ => return Err(Error::e12_mismatch()),
                }
            }
            Some(_) => return Err(Error::e12_mismatch()),
        };
        out.insert(p.name.clone(), attrs);
    }
    Ok(out)
}

/// Вычислить пары литерала атрибутов (значения — литералы, `at` нет).
fn eval_attr_pairs(
    pairs: &[(String, Expr)],
    defs: &Defs,
    env: &HashMap<String, Value>,
) -> Result<Vec<(String, Value)>, Error> {
    pairs
        .iter()
        .map(|(k, v)| eval_expr_with_env(v, 0, defs, env).map(|ev| (k.clone(), ev)))
        .collect()
}

/// Голый `at` рядом со строковым литералом (§4.13): проверить литерал.
/// Остальное смешение — ложь, вызыватель даст `E12`.
fn date_cmp_ok(left: &Expr, right: &Expr) -> Result<bool, Error> {
    let lit = match (left, right) {
        (Expr::At, Expr::Str(s)) | (Expr::Str(s), Expr::At) => s,
        _ => return Ok(false),
    };
    normalize_date_literal(lit)?;
    Ok(true)
}

/// Строковый литерал даты к канонической форме `YYYY-MM-DDTHH:MM:SS.mmm`.
/// Короткие формы дополняются нулями; кривой литерал — `E12`.
fn normalize_date_literal(raw: &str) -> Result<String, Error> {
    let bad = || Error::e12_date(raw);
    let b = raw.as_bytes();
    // Фиксированные длины: 10 дата, 16 +часы:минуты, 19 +секунды, 23 +милли.
    let full = match b.len() {
        10 => format!("{raw}T00:00:00.000"),
        16 => format!("{raw}:00.000"),
        19 => format!("{raw}.000"),
        23 => raw.to_owned(),
        _ => return Err(bad()),
    };
    let f = full.as_bytes();
    if f[4] != b'-'
        || f[7] != b'-'
        || f[10] != b'T'
        || f[13] != b':'
        || f[16] != b':'
        || f[19] != b'.'
    {
        return Err(bad());
    }
    let num = |from: usize, len: usize| {
        f[from..from + len].iter().try_fold(0i64, |v, &c| {
            if c.is_ascii_digit() {
                Some(v * 10 + (c - b'0') as i64)
            } else {
                None
            }
        })
    };
    let y = num(0, 4).ok_or_else(bad)?;
    let mo = num(5, 2).ok_or_else(bad)?;
    let d = num(8, 2).ok_or_else(bad)?;
    let h = num(11, 2).ok_or_else(bad)?;
    let mi = num(14, 2).ok_or_else(bad)?;
    let se = num(17, 2).ok_or_else(bad)?;
    num(20, 3).ok_or_else(bad)?;
    // Год — ровно 4 цифры, уже в `0000…9999`; остальное — диапазоны календаря.
    if !(1..=12).contains(&mo)
        || d < 1
        || d > crate::datetime::days_in_month(y, mo)
        || h > 23
        || mi > 59
        || se > 59
    {
        return Err(bad());
    }
    Ok(full)
}

/// Вычисление константы; `None` — внутри есть `at` или параметр.
fn const_eval(expr: &Expr, cx: &mut CxTy<'_>) -> Option<Result<Value, Error>> {
    if has_at(expr) || has_param(expr, cx) {
        return None;
    }
    let mut ev = CxEv {
        defs: cx.defs,
        stack: cx.stack.clone(),
        unit: cx.unit,
        vars: HashMap::new(),
    };
    let r = eval_expr(expr, 0, &mut ev);
    Some(r)
}

fn has_at(expr: &Expr) -> bool {
    match expr {
        Expr::At => true,
        Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Name(_) => false,
        Expr::Map(pairs) => pairs.iter().any(|(_, v)| has_at(v)),
        Expr::Array(xs) => xs.iter().any(has_at),
        Expr::Field { base, .. } | Expr::Index { base, .. } => has_at(base),
        Expr::Neg(x) => has_at(x),
        Expr::Truth(c) => has_cond_at(c),
        Expr::Bin { left, right, .. } | Expr::Bit { left, right, .. } => {
            has_at(left) || has_at(right)
        }
        Expr::Concat(xs) => xs.iter().any(has_at),
        Expr::Call { args, .. } => args.iter().any(has_at),
    }
}

fn has_cond_at(cond: &Cond) -> bool {
    match cond {
        Cond::Or(cs) | Cond::And(cs) => cs.iter().any(has_cond_at),
        Cond::Not(c) => has_cond_at(c),
        Cond::Pred { args, .. } => args.iter().any(has_at),
        Cond::Cmp { left, right, .. } => {
            has_at(left)
                || match right {
                    CondRhs::One(r) => has_at(r),
                    CondRhs::Alt(alts) => alts.iter().any(has_at),
                }
        }
    }
}

fn has_param(expr: &Expr, cx: &CxTy<'_>) -> bool {
    match expr {
        Expr::Name(n) => cx.vars.contains_key(n),
        Expr::At | Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) => false,
        Expr::Map(pairs) => pairs.iter().any(|(_, v)| has_param(v, cx)),
        Expr::Array(xs) => xs.iter().any(|x| has_param(x, cx)),
        Expr::Field { base, .. } | Expr::Index { base, .. } => has_param(base, cx),
        Expr::Neg(x) => has_param(x, cx),
        Expr::Truth(c) => has_cond_param(c, cx),
        Expr::Bin { left, right, .. } | Expr::Bit { left, right, .. } => {
            has_param(left, cx) || has_param(right, cx)
        }
        Expr::Concat(xs) => xs.iter().any(|x| has_param(x, cx)),
        Expr::Call { args, .. } => args.iter().any(|a| has_param(a, cx)),
    }
}

fn has_cond_param(cond: &Cond, cx: &CxTy<'_>) -> bool {
    match cond {
        Cond::Or(cs) | Cond::And(cs) => cs.iter().any(|c| has_cond_param(c, cx)),
        Cond::Not(c) => has_cond_param(c, cx),
        Cond::Pred { args, .. } => args.iter().any(|a| has_param(a, cx)),
        Cond::Cmp { left, right, .. } => {
            has_param(left, cx)
                || match right {
                    CondRhs::One(r) => has_param(r, cx),
                    CondRhs::Alt(alts) => alts.iter().any(|a| has_param(a, cx)),
                }
        }
    }
}

/// Вычислить условие для абсолютного времени строки (`at`).
pub fn eval_cond(cond: &Cond, at: i64, defs: &Defs) -> Result<bool, Error> {
    CxEv {
        defs,
        stack: Vec::new(),
        unit: defs.main,
        vars: HashMap::new(),
    }
    .eval_cond(cond, at)
}

/// Вычислить условие в окружении параметров (динамический скоуп развёртки).
/// Пустое окружение — то же, что `eval_cond`.
pub fn eval_cond_with_env(
    cond: &Cond,
    at: i64,
    defs: &Defs,
    env: &HashMap<String, Value>,
) -> Result<bool, Error> {
    CxEv {
        defs,
        stack: Vec::new(),
        unit: defs.main,
        vars: env.clone(),
    }
    .eval_cond(cond, at)
}

/// Вычислить выражение (аргумент вызова, значение блока) в окружении
/// параметров. Несвязанное имя — E11, как голое неизвестное имя.
pub fn eval_expr_with_env(
    expr: &Expr,
    at: i64,
    defs: &Defs,
    env: &HashMap<String, Value>,
) -> Result<Value, Error> {
    let mut cx = CxEv {
        defs,
        stack: Vec::new(),
        unit: defs.main,
        vars: env.clone(),
    };
    eval_expr(expr, at, &mut cx)
}

impl CxEv<'_> {
    fn enter(&mut self, name: &str, unit: usize) -> Result<(), Error> {
        if self.stack.iter().any(|(n, _)| n == name) {
            return Err(Error::e12_recursive(name));
        }
        let prev = std::mem::replace(&mut self.unit, unit);
        self.stack.push((name.to_owned(), prev));
        Ok(())
    }

    fn leave(&mut self) {
        if let Some((_, prev)) = self.stack.pop() {
            self.unit = prev;
        }
    }

    fn eval_cond(&mut self, cond: &Cond, at: i64) -> Result<bool, Error> {
        match cond {
            Cond::Or(cs) => {
                for c in cs {
                    if self.eval_cond(c, at)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Cond::And(cs) => {
                for c in cs {
                    if !self.eval_cond(c, at)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Cond::Not(c) => Ok(!self.eval_cond(c, at)?),
            Cond::Pred { name, args } => {
                let arg = match args.as_slice() {
                    [a] => eval_expr(a, at, self)?,
                    _ => return Err(Error::e12_arity(name)),
                };
                let at_arg = match arg {
                    Value::Num(n) => n,
                    _ => return Err(Error::e12_mismatch()),
                };
                let defs = self.defs;
                let def = resolve(defs, name, self.unit)?;
                match def.kind {
                    DefKind::Pred => {}
                    _ => return Err(Error::e12_not_pred(name)),
                }
                self.enter(name, def.unit)?;
                let body = def.cond.clone().expect("pred: тело");
                let r = self.eval_cond(&body, at_arg);
                self.leave();
                r
            }
            Cond::Cmp { op, left, right } => {
                let l = eval_expr(left, at, self)?;
                match right {
                    CondRhs::One(rexpr) => {
                        let r = eval_expr(rexpr, at, self)?;
                        // Голый `at` рядом со строкой: проверка уже пропустила
                        // только эту форму смешения — приводим здесь.
                        let canonical = || {
                            crate::datetime::format_datetime_full(at).ok_or_else(|| {
                                Error::e12_date(&crate::datetime::format_datetime(at))
                            })
                        };
                        match (&l, &r, left, rexpr) {
                            (Value::Num(_), Value::Str(_), Expr::At, Expr::Str(lit)) => {
                                let norm = normalize_date_literal(lit)?;
                                cmp_values(*op, &Value::Str(canonical()?), &Value::Str(norm))
                            }
                            (Value::Str(_), Value::Num(_), Expr::Str(lit), Expr::At) => {
                                let norm = normalize_date_literal(lit)?;
                                cmp_values(*op, &Value::Str(norm), &Value::Str(canonical()?))
                            }
                            _ => cmp_values(*op, &l, &r),
                        }
                    }
                    CondRhs::Alt(alts) => {
                        if !matches!(op, CmpOp::Eq | CmpOp::Ne) {
                            return Err(Error::e12_mismatch());
                        }
                        let mut any_eq = false;
                        for a in alts {
                            let v = eval_expr(a, at, self)?;
                            if cmp_values(CmpOp::Eq, &l, &v)? {
                                any_eq = true;
                                break;
                            }
                        }
                        Ok(if *op == CmpOp::Eq { any_eq } else { !any_eq })
                    }
                }
            }
        }
    }
}

fn cmp_values(op: CmpOp, l: &Value, r: &Value) -> Result<bool, Error> {
    match (l, r) {
        (Value::Num(a), Value::Num(b)) => Ok(match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        }),
        (Value::Str(a), Value::Str(b)) => Ok(match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        }),
        // Мапы/массивы не сравниваются («пока», см. черновик) — даже между собой.
        (Value::Map(_) | Value::Array(_), _) | (_, Value::Map(_) | Value::Array(_)) => {
            Err(Error::e12_map_cmp())
        }
        _ => Err(Error::e12_mismatch()),
    }
}

/// Вычислить выражение для `at`. Переполнение — E12.
fn eval_expr(expr: &Expr, at: i64, cx: &mut CxEv<'_>) -> Result<Value, Error> {
    match expr {
        Expr::Num(raw) => Ok(Value::Num(raw.parse().map_err(|_| Error::e12_range(raw))?)),
        Expr::Str(s) => Ok(Value::Str(s.clone())),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::Map(pairs) => pairs
            .iter()
            .map(|(k, v)| eval_expr(v, at, cx).map(|ev| (k.clone(), ev)))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Map),
        Expr::Array(xs) => xs
            .iter()
            .map(|x| eval_expr(x, at, cx))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        // Доступ — только в момент строки: нет ключа / не-мапа / не-массив —
        // `unknown field`, выход за границы (и отрицательный) — `out of bounds`.
        Expr::Field { base, field } => match eval_expr(base, at, cx)? {
            Value::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| k == field)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| Error::e12_field(field)),
            _ => Err(Error::e12_field(field)),
        },
        Expr::Index { base, index } => {
            let items = match eval_expr(base, at, cx)? {
                Value::Array(xs) => xs,
                _ => return Err(Error::e12_field(index)),
            };
            let i: i64 = index.parse().map_err(|_| Error::e12_range(index))?;
            if i < 0 {
                return Err(Error::e12_index(index));
            }
            items
                .get(i as usize)
                .cloned()
                .ok_or_else(|| Error::e12_index(index))
        }
        Expr::At => Ok(Value::Num(at)),
        Expr::Name(name) => {
            if let Some(v) = cx.vars.get(name) {
                return Ok(v.clone());
            }
            let defs = cx.defs;
            let (unit, body) = match resolve(defs, name, cx.unit) {
                Ok(def) if def.kind == DefKind::Const => {
                    (def.unit, def.expr.clone().expect("const: тело"))
                }
                Ok(_) => return Err(Error::e12_mismatch()),
                Err(e) => return Err(e),
            };
            cx.enter(name, unit)?;
            let r = eval_expr(&body, at, cx);
            cx.leave();
            r
        }
        Expr::Neg(x) => match eval_expr(x, at, cx)? {
            Value::Num(v) => v
                .checked_neg()
                .map(Value::Num)
                .ok_or_else(|| Error::e12_range("negation overflow")),
            _ => Err(Error::e12_mismatch()),
        },
        Expr::Truth(c) => Ok(Value::Num(i64::from(eval_cond_in(cx, c, at)?))),
        Expr::Bin { op, left, right } => {
            let (a, b) = match (eval_expr(left, at, cx)?, eval_expr(right, at, cx)?) {
                (Value::Num(a), Value::Num(b)) => (a, b),
                _ => return Err(Error::e12_mismatch()),
            };
            let v = match op {
                ArithOp::Add => a.checked_add(b),
                ArithOp::Sub => a.checked_sub(b),
                ArithOp::Mul => a.checked_mul(b),
                ArithOp::Div => {
                    if b == 0 {
                        return Err(Error::e12_divzero());
                    }
                    a.checked_div(b)
                }
                ArithOp::Mod => {
                    if b == 0 {
                        return Err(Error::e12_divzero());
                    }
                    a.checked_rem(b)
                }
                ArithOp::FloorDiv => {
                    if b == 0 {
                        return Err(Error::e12_divzero());
                    }
                    a.checked_div_euclid(b)
                }
                ArithOp::FloorMod => {
                    if b == 0 {
                        return Err(Error::e12_divzero());
                    }
                    a.checked_rem_euclid(b)
                }
            };
            v.map(Value::Num)
                .ok_or_else(|| Error::e12_range("arithmetic overflow"))
        }
        // Битовые (§4.13): two's complement с wrap'ом, ошибок нет по построению.
        // `>>` — логический (добивка нулями), величина сдвига — по модулю 64.
        Expr::Bit { op, left, right } => {
            let (a, b) = match (eval_expr(left, at, cx)?, eval_expr(right, at, cx)?) {
                (Value::Num(a), Value::Num(b)) => (a, b),
                _ => return Err(Error::e12_mismatch()),
            };
            let k = b.rem_euclid(64) as u32;
            let v = match op {
                BitOp::And => a & b,
                BitOp::Or => a | b,
                BitOp::Xor => a ^ b,
                BitOp::Shl => a.wrapping_shl(k),
                BitOp::Shr => (a as u64).wrapping_shr(k) as i64,
            };
            Ok(Value::Num(v))
        }
        Expr::Concat(xs) => {
            let mut out = String::new();
            for x in xs {
                match eval_expr(x, at, cx)? {
                    Value::Str(s) => out.push_str(&s),
                    _ => return Err(Error::e12_mismatch()),
                }
            }
            Ok(Value::Str(out))
        }
        Expr::Call { name, args } => eval_call(name, args, at, cx),
    }
}

fn eval_cond_in(cx: &mut CxEv<'_>, cond: &Cond, at: i64) -> Result<bool, Error> {
    cx.eval_cond(cond, at)
}

fn eval_call(name: &str, args: &[Expr], at: i64, cx: &mut CxEv<'_>) -> Result<Value, Error> {
    let defs = cx.defs;
    if let Some(def) = defs.map.get(name) {
        if !(def.private && def.unit != cx.unit) {
            return eval_def_call(name, def, args, at, cx);
        }
    }
    match name {
        "str" => {
            let a = args.first().ok_or_else(|| Error::e12_arity(name))?;
            if args.len() != 1 {
                return Err(Error::e12_arity(name));
            }
            match eval_expr(a, at, cx)? {
                Value::Num(n) => Ok(Value::Str(n.to_string())),
                _ => Err(Error::e12_mismatch()),
            }
        }
        "pad" => {
            if args.len() != 2 {
                return Err(Error::e12_arity(name));
            }
            let (n, w) = match (eval_expr(&args[0], at, cx)?, eval_expr(&args[1], at, cx)?) {
                (Value::Num(n), Value::Num(w)) => (n, w),
                _ => return Err(Error::e12_mismatch()),
            };
            let s = n.to_string();
            let w = w.max(0) as usize;
            if s.len() >= w {
                Ok(Value::Str(s))
            } else {
                Ok(Value::Str("0".repeat(w - s.len()) + &s))
            }
        }
        "floordiv" | "floormod" => {
            let [a, b] = args else {
                return Err(Error::e12_arity(name));
            };
            let (x, y) = match (eval_expr(a, at, cx)?, eval_expr(b, at, cx)?) {
                (Value::Num(x), Value::Num(y)) => (x, y),
                _ => return Err(Error::e12_mismatch()),
            };
            if y == 0 {
                return Err(Error::e12_divzero());
            }
            Ok(Value::Num(if name == "floordiv" {
                x.div_euclid(y)
            } else {
                x.rem_euclid(y)
            }))
        }
        "mkdate" => {
            if args.len() != 7 {
                return Err(Error::e12_arity(name));
            }
            let mut vals = Vec::with_capacity(7);
            for a in args {
                match eval_expr(a, at, cx)? {
                    Value::Num(v) => vals.push(v),
                    _ => return Err(Error::e12_mismatch()),
                }
            }
            build_date(&vals, name).map(Value::Num)
        }
        _ => Err(Error::e11(name)),
    }
}

/// Собрать дату из 7 чисел (§4.13): кривые компоненты — `invalid date`,
/// переполнение сборки — `integer out of range`. Сырь — числа как даны.
fn build_date(v: &[i64], name: &str) -> Result<i64, Error> {
    let [y, mo, d, h, mi, s, ms] = v else {
        return Err(Error::e12_arity(name));
    };
    let raw = format!("{y}-{mo}-{d}T{h}:{mi}:{s}.{ms}");
    match crate::datetime::make_datetime(*y, *mo, *d, *h, *mi, *s, *ms) {
        Ok(t) => Ok(t),
        Err(crate::datetime::DateBuildErr::Invalid) => Err(Error::e12_date(&raw)),
        Err(crate::datetime::DateBuildErr::Overflow) => Err(Error::e12_range(&raw)),
    }
}

fn eval_def_call(
    name: &str,
    def: &Def,
    args: &[Expr],
    at: i64,
    cx: &mut CxEv<'_>,
) -> Result<Value, Error> {
    if cx.stack.iter().any(|(n, _)| n == name) {
        return Err(Error::e12_recursive(name));
    }
    match def.kind {
        DefKind::Const => {
            if !args.is_empty() {
                return Err(Error::e12_arity(name));
            }
            cx.enter(name, def.unit)?;
            let body = def.expr.clone().expect("const: тело");
            let r = eval_expr(&body, at, cx);
            cx.leave();
            r
        }
        DefKind::Fun => {
            let arg = match args {
                [a] => eval_expr(a, at, cx)?,
                _ => return Err(Error::e12_arity(name)),
            };
            cx.enter(name, def.unit)?;
            let param = def.param.clone().expect("fun: параметр");
            let body = def.expr.clone().expect("fun: тело");
            let old = cx.vars.insert(param.clone(), arg);
            let r = eval_expr(&body, at, cx);
            match old {
                Some(v) => {
                    cx.vars.insert(param, v);
                }
                None => {
                    cx.vars.remove(&param);
                }
            }
            cx.leave();
            r
        }
        DefKind::Pred => Err(Error::e12_mismatch()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate_names;
    use cyclorithm_parser as p;

    fn cond_of(row: &str) -> Cond {
        let src = format!(
            "schedule \"T\" {{ point A {{ actions = [x]; }} \
            cycle R duration = 1h {{ 0m: A.x(); }} \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ [{row}] 6h: R(); }} }}"
        );
        let s = p::parse(&src).expect("фикстура обязана разбираться");
        s.schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть")
    }

    fn test_defs() -> Defs {
        resolve_defs(&[]).expect("прелюдия обязана проверяться").0
    }

    fn yes(row: &str, at: i64) -> bool {
        let c = cond_of(row);
        let d = test_defs();
        check_single(&c, &d).expect("условие обязано проходить проверку");
        eval_cond(&c, at, &d).expect("вычисление обязано удаваться")
    }

    fn no(row: &str, at: i64) -> bool {
        !yes(row, at)
    }

    fn check_single(c: &Cond, d: &Defs) -> Result<(), Error> {
        CxTy {
            defs: d,
            stack: Vec::new(),
            unit: d.main,
            vars: HashMap::new(),
            data: false,
        }
        .infer_cond(c)
    }

    fn static_err(row: &str) -> Error {
        let d = test_defs();
        check_single(&cond_of(row), &d).expect_err("ожидалась ошибка проверки")
    }

    fn defs_of(body: &str) -> Result<Defs, Error> {
        let src = format!("{body} schedule \"T\" {{ point A {{ actions = [x]; }} root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ 0m: A.x(); }} }}");
        let s = p::parse(&src).expect("фикстура обязана разбираться");
        resolve_defs(&s.decls).map(|(d, _)| d)
    }

    fn eval_with(body: &str, row: &str, at: i64) -> Result<bool, Error> {
        let d = defs_of(body)?;
        let c = cond_of(row);
        check_single(&c, &d)?;
        eval_cond(&c, at, &d)
    }

    #[test]
    fn arithmetic_follows_precedence() {
        assert!(yes("at + 2 * 3 == 8", 2));
        assert!(yes("(at + 2) * 3 == 12", 2));
        assert!(yes("-at == 0 - 2", 2));
    }

    #[test]
    fn division_rounds_as_spec() {
        assert!(yes("0 - 7 / 2 == 0 - 3", 0));
        assert!(yes("0 - 7 % 3 == 0 - 1", 0));
        assert!(yes("(0 - 7) floordiv 2 == 0 - 4", 0));
        assert!(yes("(0 - 7) floormod 2 == 1", 0));
    }

    #[test]
    fn bitwise_operators_wrap() {
        assert!(yes("1 << 3 == 8", 0));
        assert!(yes("256 >> 2 == 64", 0));
        assert!(yes("14 & 11 == 10", 0));
        assert!(yes("7 ^ 3 == 4", 0));
        assert!(yes("8 | 3 == 11", 0));
        // `1 << 63` — минимум i64: равен вычисленному выражением.
        assert!(yes("(1 << 63) + 9223372036854775807 == 0 - 1", 0));
        // `>>` логический: `-1 >> 1` — максимум i64.
        assert!(yes("(0 - 1 >> 1) == 9223372036854775807", 0));
        // Величина сдвига — по модулю 64.
        assert!(yes("1 << 64 == 1", 0));
        assert!(yes("1 << (0 - 1) == 1 << 63", 0));
        assert!(yes("(0 - 1 >> 65) == 9223372036854775807", 0));
    }

    #[test]
    fn bitwise_precedence() {
        // Сдвиг слабее сложения: `(1 + 2) << 3`.
        assert!(yes("1 + 2 << 3 == 24", 0));
        // `&` сильнее `^`, `^` сильнее `|`: `1 | 2 ^ (3 & 12) == 3`.
        assert!(yes("1 | 2 ^ 3 & 12 == 3", 0));
        assert!(yes("1 << (1 + 1) == 4", 0));
        assert!(no("1 << 2 + 1 == 5", 0));
    }

    #[test]
    fn bitwise_rejects_string_mixing() {
        let e = static_err("at & \"x\" == \"y\"");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "type mismatch: cannot mix number and string")
        );
    }

    #[test]
    fn bitwise_shift_by_zero_expression_never_errors() {
        // Правый операнд-константа 0 — не деление: статика проходит,
        // в момент строки ошибки тоже нет.
        let c = cond_of("1 << (at - at) == 1");
        let d = test_defs();
        check_single(&c, &d).expect("сдвиг на ноль выражения — не ошибка");
        assert!(eval_cond(&c, 100, &d).expect("вычисление обязано пройти"));
    }

    #[test]
    fn strings_concatenate_and_format() {
        assert!(yes("str(5) ++ \"x\" == \"5x\"", 0));
        assert!(yes("str(0 - 5) == \"-5\"", 0));
        assert!(yes("pad(6, 2) == \"06\"", 0));
        assert!(yes("pad(2026, 4) == \"2026\"", 0));
        assert!(yes("\"b\" > \"a\" and \"a\" < \"b\"", 0));
    }

    #[test]
    fn logic_and_alternation() {
        assert!(yes("not at == 1", 2));
        assert!(no("not at == 1", 1));
        assert!(yes("at == 1 or at == 2", 2));
        assert!(yes("at == (1 or 2)", 2));
        assert!(no("at == (1 or 2)", 3));
        assert!(yes("at != (1 or 2)", 3));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // Без скобок: `at == 1 or (at == 2 and at == 2)`.
        assert!(yes("at == 1 or at == 2 and at == 2", 1));
        assert!(no("at == 1 or at == 2 and at == 2", 3));
        // Группа переопределяет: `(at == 1 or at == 2) and at == 2`.
        assert!(no("(at == 1 or at == 2) and at == 2", 1));
        assert!(yes("(at == 1 or at == 2) and at == 2", 2));
        assert!(yes("not (at == 1 or at == 2)", 3));
    }

    #[test]
    fn truth_bridge_takes_and_or() {
        assert!(yes("2 * (at == 1 or at == 2) == 2", 2));
        assert!(yes("2 * (at == 1 or at == 2) == 0", 3));
        assert!(yes("1 + (at == 1 and not at == 2) == 2", 1));
        assert!(yes("1 + (at == 1 and not at == 2) == 1", 2));
    }

    #[test]
    fn truth_bridge_short_circuits() {
        // Ленивость or действует и внутри мостика: деление на ноль
        // в мёртвой ветке не срабатывает при истинной первой.
        assert!(yes("1 * (at == 1 or 1 / (at - at) == 0) == 1", 1));
    }

    fn rand_vals(t0: i64, step: i64, n: usize) -> Vec<i64> {
        let d = test_defs();
        let mut cx = CxEv {
            defs: &d,
            stack: Vec::new(),
            unit: d.main,
            vars: HashMap::new(),
        };
        (0..n)
            .map(|i| {
                let t = t0 + i as i64 * step;
                let e = Expr::Call {
                    name: "rand".to_owned(),
                    args: vec![Expr::Num(t.to_string())],
                };
                match eval_expr(&e, 0, &mut cx) {
                    Ok(Value::Num(v)) => v,
                    _ => panic!("rand обязан давать число"),
                }
            })
            .collect()
    }

    #[test]
    fn rand_passes_statistical_guard() {
        // Страж качества прелюдийного ГСЧ: минутные метки января 2026.
        // Границы с запасом (замер: chi2 98, WW z 0.5, доли 0.504/0.100/0.010).
        let v = rand_vals(1767225600000, 60000, 43200);
        let n = v.len() as f64;
        assert!(v.iter().all(|&x| (0..10000).contains(&x)));
        for (th, want, tol) in [(5000, 0.5, 0.01), (1000, 0.1, 0.005), (100, 0.01, 0.002)] {
            let got = v.iter().filter(|&&x| x < th).count() as f64 / n;
            assert!((got - want).abs() < tol, "доля < {th}: {got}");
        }
        let mut bins = [0u32; 100];
        for &x in &v {
            bins[(x / 100) as usize] += 1;
        }
        let exp = n / 100.0;
        let chi2: f64 = bins
            .iter()
            .map(|&b| (f64::from(b) - exp).powi(2) / exp)
            .sum();
        assert!(chi2 < 140.0, "chi2: {chi2}");
        let med = {
            let mut s = v.clone();
            s.sort_unstable();
            s[v.len() / 2]
        };
        let s: Vec<u8> = v.iter().map(|&x| u8::from(x >= med)).collect();
        let n1 = s.iter().filter(|&&b| b == 1).count() as f64;
        let n0 = n - n1;
        let runs = 1.0 + s.windows(2).filter(|w| w[0] != w[1]).count() as f64;
        let mu = 2.0 * n1 * n0 / n + 1.0;
        let var = 2.0 * n1 * n0 * (2.0 * n1 * n0 - n) / (n * n * (n - 1.0));
        let z = (runs - mu) / var.sqrt();
        assert!(z.abs() < 2.5, "Вальд–Вольфовиц z: {z}");
        let lag1 = {
            let (a, b) = (&v[..v.len() - 1], &v[1..]);
            let (ma, mb) = (
                a.iter().sum::<i64>() as f64 / a.len() as f64,
                b.iter().sum::<i64>() as f64 / b.len() as f64,
            );
            let cov: f64 = a
                .iter()
                .zip(b.iter())
                .map(|(&x, &y)| (x as f64 - ma) * (y as f64 - mb))
                .sum::<f64>()
                / a.len() as f64;
            let va: f64 = a.iter().map(|&x| (x as f64 - ma).powi(2)).sum::<f64>() / a.len() as f64;
            cov / va.sqrt() / va.sqrt()
        };
        assert!(lag1.abs() < 0.03, "лаг-1: {lag1}");
    }

    #[test]
    fn workday_weekends() {
        // Метки — полночи UTC: 2026-01-01 чт, далее пт/сб/вс/пн.
        assert!(yes("workday(1767225600000)", 0));
        assert!(yes("workday(1767312000000)", 0));
        assert!(no("workday(1767398400000)", 0));
        assert!(no("workday(1767484800000)", 0));
        assert!(yes("workday(1767571200000)", 0));
    }

    #[test]
    fn quarter_boundaries() {
        assert!(yes("quarter(1767225600000) == 1", 0)); // 01.01
        assert!(yes("quarter(1774915200000) == 1", 0)); // 31.03
        assert!(yes("quarter(1775001600000) == 2", 0)); // 01.04
        assert!(yes("quarter(1776211200000) == 2", 0)); // 15.04
        assert!(yes("quarter(1784073600000) == 3", 0)); // 15.07
        assert!(yes("quarter(1792022400000) == 4", 0)); // 15.10
        assert!(yes("quarter(1797292800000) == 4", 0)); // 15.12
    }

    #[test]
    fn is_leap_century_rules() {
        assert!(yes("is_leap(1704067200000) == 1", 0)); // 2024
        assert!(yes("is_leap(1767225600000) == 0", 0)); // 2026
        assert!(yes("is_leap(946684800000) == 1", 0)); // 2000 (% 400)
        assert!(yes("is_leap(0 - 2208988800000) == 0", 0)); // 1900 (% 100)
    }

    #[test]
    fn days_in_month_year_table() {
        // 15-е числа 2026 (невисокосный): 31/28/31/30/31/30/31/31/30/31/30/31.
        for (t, want) in [
            (1768435200000i64, 31),
            (1771113600000i64, 28),
            (1773532800000i64, 31),
            (1776211200000i64, 30),
            (1778803200000i64, 31),
            (1781481600000i64, 30),
            (1784073600000i64, 31),
            (1786752000000i64, 31),
            (1789430400000i64, 30),
            (1792022400000i64, 31),
            (1794700800000i64, 30),
            (1797292800000i64, 31),
        ] {
            assert!(yes(&format!("days_in_month({t}) == {want}"), 0));
        }
        assert!(yes("days_in_month(1707955200000) == 29", 0)); // 02.2024
        assert!(yes("days_in_month(0 - 2205100800000) == 28", 0)); // 02.1900
    }

    #[test]
    fn start_of_day_month_rounding() {
        assert!(yes("start_of_day(1767225600000) == 1767225600000", 0)); // полночь
        assert!(yes("start_of_day(1767268800123) == 1767225600000", 0)); // 12:00:00.123
        assert!(yes("start_of_month(1768478400000) == 1767225600000", 0)); // 15.01 12:00
        assert!(yes("start_of_month(1773554400000) == 1772323200000", 0)); // 15.03 06:00
        assert!(yes("day(start_of_month(1773554400000)) == 1", 0)); // композиция
    }

    #[test]
    fn rand_is_deterministic_across_runs() {
        // Эталонные значения: один и тот же вход — один и тот же выход
        // в любом прогоне (контракт детерминизма прелюдии).
        // Соседние миллисекунды лавинят (9453 → 5532 → 1487).
        assert_eq!(rand_vals(1767225600000, 1, 3), vec![9453, 5532, 1487]);
        // Дальняя метка: 2026-06-15T00:00:00Z.
        assert_eq!(rand_vals(1781481600000, 1, 1), vec![1737]);
    }

    #[test]
    fn static_errors_propagate_through_group() {
        let e = static_err("(at + \"x\" == \"y\")");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "type mismatch: cannot mix number and string")
        );
    }

    #[test]
    fn truth_bridge_gives_one_zero() {
        assert!(yes("12 * (at >= 2) == 12", 2));
        assert!(yes("12 * (at >= 3) == 0", 2));
    }

    #[test]
    fn rejects_unknown_names() {
        let e = static_err("banana(at) == 1");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E11", "unknown name 'banana'")
        );
    }

    #[test]
    fn rejects_bad_arity_and_mixing() {
        let e = static_err("str(1, 2) == \"x\"");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "wrong arguments for 'str'")
        );
        let e = static_err("at + \"x\" == \"y\"");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "type mismatch: cannot mix number and string")
        );
    }

    #[test]
    fn mkdate_epoch_and_known_dates() {
        assert!(yes("mkdate(1970, 1, 1, 0, 0, 0, 0) == 0", 0));
        assert!(yes("mkdate(2026, 1, 1, 0, 0, 0, 0) == 1767225600000", 0));
        assert!(yes(
            "mkdate(2026, 1, 1, 12, 30, 15, 250) == 1767270615250",
            0
        ));
        assert!(yes("mkdate(2000, 2, 29, 0, 0, 0, 0) == 951782400000", 0));
        assert!(yes(
            "mkdate(1960, 5, 5, 12, 30, 15, 250) == 0 - 304774184750",
            0
        ));
    }

    #[test]
    fn mkdate_validates_components_statically() {
        for bad in [
            "mkdate(2026, 13, 1, 0, 0, 0, 0) == 0",
            "mkdate(2026, 0, 1, 0, 0, 0, 0) == 0",
            "mkdate(2026, 4, 31, 0, 0, 0, 0) == 0",
            "mkdate(2026, 2, 29, 0, 0, 0, 0) == 0",
            "mkdate(2026, 1, 0, 0, 0, 0, 0) == 0",
            "mkdate(2026, 1, 1, 24, 0, 0, 0) == 0",
            "mkdate(2026, 1, 1, 0, 60, 0, 0) == 0",
            "mkdate(2026, 1, 1, 0, 0, 60, 0) == 0",
            "mkdate(2026, 1, 1, 0, 0, 0, 1000) == 0",
            "mkdate(2026, 1, 1, 0, 0, 0, 0 - 1) == 0",
            "mkdate(1900, 2, 29, 0, 0, 0, 0) == 0",
        ] {
            let e = static_err(bad);
            assert_eq!(e.code, "E12", "{bad}");
            assert!(
                e.message.starts_with("invalid date"),
                "{bad}: {}",
                e.message
            );
        }
    }

    #[test]
    fn mkdate_rejects_bad_arity_mixing_and_overflow() {
        let e = static_err("mkdate(2026, 1, 1, 0, 0, 0) == 0");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "wrong arguments for 'mkdate'")
        );
        let e = static_err("mkdate(2026, 1, 1, 0, 0, 0, 0, 0) == 0");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "wrong arguments for 'mkdate'")
        );
        let e = static_err("mkdate(2026, \"x\", 1, 0, 0, 0, 0) == 0");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "type mismatch: cannot mix number and string")
        );
        let e = static_err("mkdate(300000000, 1, 1, 0, 0, 0, 0) == 0");
        assert_eq!(e.code, "E12");
        assert!(
            e.message.starts_with("integer out of range"),
            "{}",
            e.message
        );
    }

    #[test]
    fn mkdate_runtime_invalid_is_error_not_skip() {
        // 29 февраля невисокосного через выражение: статика проходит,
        // в момент строки — ошибка (как деление на ноль выражением).
        let c = cond_of("mkdate(2026, 2, 27 + (at - at) + 2, 0, 0, 0, 0) == 0");
        let d = test_defs();
        check_single(&c, &d).expect("день не константа — статика проходит");
        let e = eval_cond(&c, 100, &d).expect_err("кривая дата — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "invalid date '2026-2-29T0:0:0.0'")
        );
    }

    #[test]
    fn mkdate_roundtrips_calendar() {
        // Разборка собирается обратно в ту же метку (включая до эпохи).
        for t in [
            1767225600000i64,
            1767270615250,
            0,
            946684800000,
            0 - 2208988800000,
        ] {
            let row = format!(
                "mkdate(year({t}), month({t}), day({t}), hour({t}), \
                minute({t}), second({t}), millisecond({t})) == {t}"
            );
            assert!(yes(&row, 0), "{t}");
        }
    }

    #[test]
    fn rejects_constant_division_by_zero() {
        let e = static_err("1 / 0 == 0");
        assert_eq!((e.code, e.message.as_str()), ("E12", "division by zero"));
    }

    #[test]
    fn runtime_division_by_zero_is_error_not_skip() {
        let c = cond_of("1 / (at - at + 1 - 1) == 0");
        let d = test_defs();
        check_single(&c, &d).expect("делитель не константа — статика проходит");
        let e = eval_cond(&c, 100, &d).expect_err("ноль в момент строки — ошибка");
        assert_eq!((e.code, e.message.as_str()), ("E12", "division by zero"));
    }

    #[test]
    fn rejects_integer_overflow() {
        let e = static_err("99999999999999999999999 == 0");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "integer out of range '99999999999999999999999'")
        );
    }

    #[test]
    fn arithmetic_overflow_is_runtime_error() {
        // `i64::MAX + 1`: статика пропускает (типы в норме), падает вычисление.
        let c = cond_of("9223372036854775807 + 1 == 0");
        let d = test_defs();
        check_single(&c, &d).expect("переполнение не константа — статика проходит");
        let e = eval_cond(&c, 0, &d).expect_err("переполнение в момент строки — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "integer out of range 'arithmetic overflow'")
        );
    }

    #[test]
    fn or_and_short_circuit_dead_branches() {
        // Правая часть при at=1 и at=2 — деление на ноль; ленивость её не трогает.
        assert!(yes("at == 1 or 1 / (at - 1) == 0", 1));
        assert!(no("at == 1 and 1 / (at - 2) == 0", 2));
    }

    #[test]
    fn alternation_beyond_eq_ne_is_runtime_error() {
        // Статика пропускает (числа, арность в норме), падает вычисление.
        let c = cond_of("at < (1 or 2)");
        let d = test_defs();
        check_single(&c, &d).expect("оператор статика не смотрит");
        let e = eval_cond(&c, 1, &d).expect_err("только == и !=");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "type mismatch: cannot mix number and string")
        );
    }

    #[test]
    fn pad_negative_width_is_noop() {
        assert!(yes("pad(5, 0 - 3) == \"5\"", 0));
    }

    #[test]
    fn pred_accepts_any_numeric_argument() {
        // Аргумент предиката — не обязательно голый `at`: станет временем тела.
        let r = eval_with("pred big(at) = at > 10;", "big(at - at + 50)", 0)
            .expect("вычисление обязано удаваться");
        assert!(r);
    }

    #[test]
    fn const_body_sees_call_site_time() {
        // `at` в теле объявления — время места вызова, не объявления.
        let r = eval_with("const K = at;", "K == 100", 100).expect("должно вычисляться");
        assert!(r);
    }

    #[test]
    fn prelude_matches_control_points() {
        // at = 0 — четверг 1970-01-01 (контрольная точка черновика).
        assert!(eval_with("", "dow(at) == 3", 0).unwrap());
        assert!(eval_with(
            "",
            "day(at) == 1 and month(at) == 1 and year(at) == 1970",
            0
        )
        .unwrap());
        assert!(eval_with("", "datestr(at) == \"1970-01-01\"", 0).unwrap());
        assert!(eval_with("", "datetimestr(at) == \"1970-01-01T00:00:00.000\"", 0).unwrap());
        assert!(eval_with("", "hour(at) == 6", 6 * 3600000).unwrap());
        assert!(eval_with("", "weekend(at)", 2 * 86400000).unwrap());
        assert!(!eval_with("", "weekend(at)", 0).unwrap());
        assert!(eval_with("", "morning(at)", 8 * 3600000).unwrap());
    }

    #[test]
    fn prelude_calendar_matches_core_dates() {
        // Сверка календаря прелюдии с наивным временем ядра.
        for ms in [0, 1767225600000, 1709160000000, 951782400000, 4102444800000] {
            let s = crate::datetime::format_datetime(ms);
            let date = &s[..10];
            assert!(
                eval_with("", &format!("datestr(at) == \"{date}\""), ms).unwrap(),
                "для {s}"
            );
        }
    }

    #[test]
    fn program_shadows_system_silently() {
        // Побеждает последнее: свой sat сдвигает weekend на четверг.
        assert!(eval_with("const sat = 3;", "weekend(at)", 0).unwrap());
        assert!(!eval_with("", "weekend(at)", 0).unwrap());
    }

    #[test]
    fn private_names_stay_in_file() {
        let e = defs_of("fun f(t) = __z(t);").expect_err("чужое __ — ошибка");
        assert_eq!((e.code, e.message.as_str()), ("E11", "unknown name '__z'"));
    }

    #[test]
    fn map_consts_are_values() {
        // Мапы/массивы/bool живут в константах (включая алиасы).
        defs_of("const M = {\"name\": \"БЖД\", \"n\": 1, \"ok\": true, \"tags\": [\"a\", 2]};")
            .expect("мапа обязана проверяться");
        defs_of("const M = {\"n\": 1}; const A = M;").expect("алиас мапы обязан проверяться");
        defs_of("const A = [1, {\"k\": false}];").expect("массив обязан проверяться");
    }

    #[test]
    fn duplicate_map_keys_are_e15() {
        // Дубль в литерале — E15 уже в объявлении (до развёртки).
        let e = defs_of("const M = {\"a\": 1, \"a\": 2};").expect_err("дубль — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E15", "duplicate attribute 'a'")
        );
        // Вложенный литерал — тоже E15.
        let e = defs_of("const M = {\"a\": {\"b\": 1, \"b\": 2}};")
            .expect_err("вложенный дубль — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E15", "duplicate attribute 'b'")
        );
    }

    #[test]
    fn bare_data_const_in_condition_is_e11() {
        // Данные напрямую в условии невидимы — даже под доступом.
        let d = defs_of("const M = {\"n\": 1, \"tags\": [\"a\"]};").unwrap();
        for row in ["M == 1", "M.n == 1", "M.tags[0] == \"a\""] {
            let e = check_single(&cond_of(row), &d).expect_err("статика обязана браковать");
            assert_eq!((e.code, e.message.as_str()), ("E11", "unknown name 'M'"));
        }
    }

    #[test]
    fn map_comparison_is_e12() {
        // Мапа с мапой — E12 «пока» (глубокое сравнение — будущее).
        let e = static_err("{\"a\": 1} == {\"a\": 1}");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "cannot compare maps or arrays")
        );
        // Мапа с числом — обычное смешение.
        let e = static_err("{\"a\": 1} == 1");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "type mismatch: cannot mix number and string")
        );
    }

    #[test]
    fn bool_in_condition_is_e12() {
        // Булева типа в условиях нет: литерал в сравнении — E12.
        let e = static_err("true == true");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "type mismatch: cannot mix number and string")
        );
    }

    #[test]
    fn duplicate_block_keys_are_e15() {
        // Дубль ключей блока — E15 в фазе строк.
        let e = check_rows(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x() { a = 1, a = 2 }; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }",
        )
        .expect_err("дубль в блоке — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E15", "duplicate attribute 'a'")
        );
    }

    #[test]
    fn call_args_see_data_names() {
        // Аргументы — позиция данных: константы-мапы видны, неизвестные — E11.
        check_rows(
            "const M = {\"n\": 1}; schedule \"T\" { point A { actions = [x]; } \
            cycle R(a) duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(M); } }",
        )
        .expect("мапа в аргументе обязана проходить");
        check_rows(
            "const M = {\"n\": 1}; schedule \"T\" { point A { actions = [x]; } \
            cycle R(a) duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(M.n); } }",
        )
        .expect("поле в аргументе обязано проходить");
        let e = check_rows(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle R(a) duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(banana); } }",
        )
        .expect_err("неизвестное имя в аргументе — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E11", "unknown name 'banana'")
        );
        // Дубль в литерале аргумента — E15 (интеграция infer).
        let e = check_rows(
            "schedule \"T\" { point A { actions = [x]; } \
            cycle R(a) duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 6h: R({\"k\": 1, \"k\": 2}); } }",
        )
        .expect_err("дубль в аргументе — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E15", "duplicate attribute 'k'")
        );
    }

    fn check_rows(src: &str) -> Result<(), Error> {
        let f = p::parse(src).expect("фикстура обязана разбираться");
        let (d, reg) =
            resolve_units(std::slice::from_ref(&f.decls)).expect("объявления обязаны проверяться");
        let t = validate_names(&f.schedule, &reg)?;
        check_conditions(&f.schedule, &d, &t)
    }

    #[test]
    fn rejects_recursive_definitions() {
        let e = defs_of("fun a(t) = b(t); fun b(t) = a(t);").expect_err("цикл — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "recursive definition 'a'")
        );
        let e = defs_of("const c = c + 1;").expect_err("самовызов — ошибка");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "recursive definition 'c'")
        );
    }

    #[test]
    fn rejects_duplicate_definitions() {
        let e = defs_of("const a = 1; fun a(t) = t;").expect_err("дубль — ошибка");
        assert_eq!((e.code, e.message.as_str()), ("E04", "duplicate fun 'a'"));
    }
    #[test]
    fn fun_of_non_predicate_is_error() {
        let e = static_err("hour(at)");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E12", "'hour' is not a predicate")
        );
    }

    #[test]
    fn bare_at_compares_with_date_strings() {
        // 2026-01-01T00:00:00 = 1767225600000 мс epoch.
        let new_year = 1_767_225_600_000;
        assert!(yes("at == \"2026-01-01\"", new_year));
        assert!(yes("at < \"2026-06-01\"", new_year + 1));
        assert!(no("at == \"2026-01-01\"", new_year + 1));
        assert!(yes("\"2026-01-01\" <= at", new_year));
        assert!(yes("at == \"2026-01-01T00:00\"", new_year));
        assert!(yes("at == \"2026-01-01T00:00:00.000\"", new_year));
        assert!(no("at < \"2026-01-01\"", new_year));
        // Каноника совпадает с datetimestr прелюдии.
        assert!(yes("datetimestr(at) >= \"2026-01-01\"", new_year));
    }

    #[test]
    fn rejects_bad_date_literals() {
        for raw in [
            "tomorrow",
            "2026-13-01",
            "2026-02-30",
            "2026-01-01T24:00:00",
            "2026-01-01T00:00:00.12",
            "2026-1-1",
        ] {
            let e = static_err(&format!("at >= \"{raw}\""));
            assert_eq!(
                (e.code, e.message.as_str()),
                ("E12", format!("invalid date '{raw}'").as_str()),
                "для {raw:?}"
            );
        }
    }

    #[test]
    fn rejects_mixing_beyond_bare_at() {
        // Не голый `at` и не литерал — обычное смешение.
        for row in [
            "hour(at) >= \"2026\"",
            "at + 1 >= \"2026-01-01\"",
            "at >= datestr(at)",
        ] {
            let e = static_err(row);
            assert_eq!(
                (e.code, e.message.as_str()),
                ("E12", "type mismatch: cannot mix number and string"),
                "для {row:?}"
            );
        }
    }

    #[test]
    fn cross_file_shadow_wins_silently() {
        // Импорт переопределяет системное имя без E04; программа — поверх.
        let imp = p::parse_decls("const sat = 3;").unwrap();
        let (d, _) = resolve_units(&[imp, vec![]]).expect("склейка обязана сходиться");
        let c = cond_of("weekend(at)");
        check_single(&c, &d).unwrap();
        assert!(eval_cond(&c, 0, &d).unwrap());
    }

    #[test]
    fn private_names_do_not_cross_files() {
        // `__` импорта не видно из программы — E11.
        let imp = p::parse_decls("fun __h(t) = t;").unwrap();
        let (d, _) = resolve_units(&[imp, vec![]]).expect("склейка обязана сходиться");
        let c = cond_of("__h(at) == 1");
        let e = check_single(&c, &d).expect_err("чужое __ — ошибка");
        assert_eq!((e.code, e.message.as_str()), ("E11", "unknown name '__h'"));
        // Своё `__` внутри своего файла работает.
        let prog = p::parse_decls("fun __p(t) = t + 1;").unwrap();
        let (d, _) = resolve_units(&[vec![], prog]).expect("склейка обязана сходиться");
        let c = cond_of("__p(at) == 3");
        check_single(&c, &d).unwrap();
        assert!(eval_cond(&c, 2, &d).unwrap());
    }

    #[test]
    fn duplicate_across_files_is_not_e04() {
        // Дубль — только внутри одного файла.
        let a = p::parse_decls("const K = 1;").unwrap();
        let b = p::parse_decls("const K = 2;").unwrap();
        let (d, _) = resolve_units(&[a, b]).expect("склейка обязана сходиться");
        let c = cond_of("at >= K");
        check_single(&c, &d).unwrap();
        assert!(eval_cond(&c, 2, &d).unwrap());
        assert!(!eval_cond(&c, 1, &d).unwrap());
    }

    #[test]
    fn routine_table_param_invisible_in_conds() {
        // Табличный параметр в условиях — E11, данные (`params[1..]`) — видны.
        let ok = "time_const D duration = 2h { 1st: 0m; } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC, subj) { [subj == 1] 1st: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(D, 1); } }";
        check_rows(ok).expect("данные рутины видны в условиях");
        let bad = ok.replace("[subj == 1]", "[TC == 1]");
        let e = check_rows(&bad).expect_err("таблица в условиях невидима");
        assert_eq!((e.code, e.message.as_str()), ("E11", "unknown name 'TC'"));
    }

    #[test]
    fn table_firing_conditions_checked() {
        // Условия пожаров проверяются без параметров (E11), вызов — как строка.
        let src = "time_const D duration = 2h { [banana == 1] tick: 0m -> A.x(); } \
            schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0h: M(D); } }";
        let e = check_rows(src).expect_err("имя в пожаре обязано проверяться");
        assert_eq!(
            (e.code, e.message.as_str()),
            ("E11", "unknown name 'banana'")
        );
    }

    #[test]
    fn time_const_registry_dup_and_overlay() {
        // Дубль таблицы внутри файла — E04 `duplicate table`.
        let d = p::parse_decls(
            "time_const D duration = 1h { 1st: 0m; } time_const D duration = 2h { 1st: 0m; }",
        )
        .unwrap();
        let e = resolve_units(&[d]).expect_err("дубль таблицы");
        assert_eq!((e.code, e.message.as_str()), ("E04", "duplicate table 'D'"));
        // Между файлами побеждает последнее; в выражениях таблиц не видно (E11).
        let a = p::parse_decls("time_const D duration = 1h { 1st: 0m; }").unwrap();
        let b = p::parse_decls("time_const D duration = 2h { 1st: 0m; }").unwrap();
        let (d, reg) = resolve_units(&[a, b]).expect("склейка обязана сходиться");
        assert_eq!(reg.tables["D"].duration.raw, "2h");
        assert_eq!(reg.tables["D"].unit, 2);
        assert_eq!(reg.tables["D"].rows.len(), 1);
        let c = cond_of("at >= D");
        let e = check_single(&c, &d).expect_err("таблица — не имя условия");
        assert_eq!((e.code, e.message.as_str()), ("E11", "unknown name 'D'"));
    }
}
