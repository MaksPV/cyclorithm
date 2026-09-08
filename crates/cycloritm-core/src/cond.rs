//! Условия строк (§3–§5 спеки): объявления, подстановка, вычисление.
//!
//! Определения (`const`/`fun`/`pred` + системный файл) раскрываются
//! через окружение — для чистых выражений это та же подстановка.
//! Проверки в порядке объявления: дубли (E04), тела (E11/E12, рекурсия),
//! затем строки. Константное деление на ноль ловится статически,
//! деление нулём выражения — вычислением в момент строки (тоже E12).

use std::collections::{HashMap, HashSet};

use cycloritm_parser::{ArithOp, CmpOp, Cond, CondRhs, Decl, Expr, Schedule};

use crate::Error;

/// Значение выражения: число или строка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Num(i64),
    Str(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Num,
    Str,
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

/// Собрать определения одного файла поверх системных.
pub fn resolve_defs(decls: &[Decl]) -> Result<Defs, Error> {
    resolve_units(&[decls.to_vec()])
}

/// Собрать определения: оверлей групп в порядке наложения
// (последняя группа — тело программы), затем проверить все тела.
// Дубли — только внутри одной группы (`E04`); между файлами побеждает последнее.
pub fn resolve_units(units: &[Vec<Decl>]) -> Result<Defs, Error> {
    let system = cycloritm_parser::parse_decls(PRELUDE).expect("прелюдия обязана разбираться");
    let mut map = HashMap::new();
    let mut all: Vec<String> = Vec::new();
    for d in &system {
        let (name, def) = to_def(d, 0);
        if !map.contains_key(&name) {
            all.push(name.clone());
        }
        map.insert(name, def);
    }
    for (i, group) in units.iter().enumerate() {
        let unit = i + 1;
        let mut seen = HashSet::new();
        for d in group {
            let (name, kind) = match d {
                Decl::Const { name, .. } => (name, "const"),
                Decl::Fun { name, .. } => (name, "fun"),
                Decl::Pred { name, .. } => (name, "pred"),
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
    Ok(defs)
}

fn to_def(decl: &Decl, unit: usize) -> (String, Def) {
    match decl {
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
    };
    match def.kind {
        DefKind::Const => {
            let body = def.expr.as_ref().expect("const: тело");
            if cx.infer(body)? != Ty::Num {
                return Err(Error::e12_mismatch());
            }
        }
        DefKind::Fun => {
            let body = def.expr.as_ref().expect("fun: тело");
            cx.vars
                .insert(def.param.clone().expect("fun: параметр"), Ty::Num);
            cx.infer(body)?;
        }
        DefKind::Pred => {
            let body = def.cond.as_ref().expect("pred: тело");
            cx.vars.insert("at".to_owned(), Ty::Num);
            cx.infer_cond(body)?;
        }
    }
    Ok(())
}

struct CxTy<'a> {
    defs: &'a Defs,
    stack: Vec<(String, usize)>,
    unit: usize,
    vars: HashMap<String, Ty>,
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

/// Проверить все условия файла в порядке объявления: циклы, затем корень.
pub fn check_conditions(schedule: &Schedule, defs: &Defs) -> Result<(), Error> {
    for c in &schedule.cycles {
        for st in &c.stmts {
            if let Some(cond) = &st.condition {
                CxTy {
                    defs,
                    stack: Vec::new(),
                    unit: defs.main,
                    vars: HashMap::new(),
                }
                .infer_cond(cond)?;
            }
        }
    }
    for st in &schedule.root.stmts {
        if let Some(cond) = &st.condition {
            CxTy {
                defs,
                stack: Vec::new(),
                unit: defs.main,
                vars: HashMap::new(),
            }
            .infer_cond(cond)?;
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
                if self.infer(arg)? != Ty::Num {
                    return Err(Error::e12_mismatch());
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
                        if lt != rt && !date_cmp_ok(left, r)? {
                            return Err(Error::e12_mismatch());
                        }
                    }
                    CondRhs::Alt(alts) => {
                        for a in alts {
                            if self.infer(a)? != Ty::Num {
                                return Err(Error::e12_mismatch());
                            }
                        }
                        if lt != Ty::Num {
                            return Err(Error::e12_mismatch());
                        }
                    }
                }
                Ok(())
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
                        self.enter(name, def.unit)?;
                        let body = def.expr.clone().expect("const: тело");
                        let ty = self.infer(&body);
                        self.leave();
                        ty
                    }
                    DefKind::Fun | DefKind::Pred => Err(Error::e12_mismatch()),
                }
            }
            Expr::Neg(x) => self.infer(x),
            Expr::Truth(c) => {
                self.infer_cond(c)?;
                Ok(Ty::Num)
            }
            Expr::Bin { left, right, .. } => {
                if self.infer(left)? != Ty::Num || self.infer(right)? != Ty::Num {
                    return Err(Error::e12_mismatch());
                }
                if let Some(Err(e)) = self.const_div(right) {
                    return Err(e);
                }
                Ok(Ty::Num)
            }
            Expr::Concat(xs) => {
                for x in xs {
                    if self.infer(x)? != Ty::Str {
                        return Err(Error::e12_mismatch());
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
                    if self.infer(a)? != Ty::Num {
                        return Err(Error::e12_mismatch());
                    }
                }
                Ok(Ty::Str)
            }
            // Делимые функции — те же операторы, вызванные явно (так пишет прелюдия).
            "floordiv" | "floormod" if !self.defs.map.contains_key(name) => {
                let [a, b] = args else {
                    return Err(Error::e12_arity(name));
                };
                if self.infer(a)? != Ty::Num || self.infer(b)? != Ty::Num {
                    return Err(Error::e12_mismatch());
                }
                if let Some(Err(e)) = self.const_div(b) {
                    return Err(e);
                }
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
                        self.enter(name, def.unit)?;
                        let body = def.expr.clone().expect("const: тело");
                        let ty = self.infer(&body);
                        self.leave();
                        ty
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
        Expr::Num(_) | Expr::Str(_) | Expr::Name(_) => false,
        Expr::Neg(x) => has_at(x),
        Expr::Truth(c) => has_cond_at(c),
        Expr::Bin { left, right, .. } => has_at(left) || has_at(right),
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
        Expr::At | Expr::Num(_) | Expr::Str(_) => false,
        Expr::Neg(x) => has_param(x, cx),
        Expr::Truth(c) => has_cond_param(c, cx),
        Expr::Bin { left, right, .. } => has_param(left, cx) || has_param(right, cx),
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
                    Value::Str(_) => return Err(Error::e12_mismatch()),
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
        _ => Err(Error::e12_mismatch()),
    }
}

/// Вычислить выражение для `at`. Переполнение — E12.
fn eval_expr(expr: &Expr, at: i64, cx: &mut CxEv<'_>) -> Result<Value, Error> {
    match expr {
        Expr::Num(raw) => Ok(Value::Num(raw.parse().map_err(|_| Error::e12_range(raw))?)),
        Expr::Str(s) => Ok(Value::Str(s.clone())),
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
            Value::Str(_) => Err(Error::e12_mismatch()),
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
        Expr::Concat(xs) => {
            let mut out = String::new();
            for x in xs {
                match eval_expr(x, at, cx)? {
                    Value::Str(s) => out.push_str(&s),
                    Value::Num(_) => return Err(Error::e12_mismatch()),
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
                Value::Str(_) => Err(Error::e12_mismatch()),
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
        _ => Err(Error::e11(name)),
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
    use cycloritm_parser as p;

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
        resolve_defs(&[]).expect("прелюдия обязана проверяться")
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
        resolve_defs(&s.decls)
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
        let d = resolve_units(&[imp, vec![]]).expect("склейка обязана сходиться");
        let c = cond_of("weekend(at)");
        check_single(&c, &d).unwrap();
        assert!(eval_cond(&c, 0, &d).unwrap());
    }

    #[test]
    fn private_names_do_not_cross_files() {
        // `__` импорта не видно из программы — E11.
        let imp = p::parse_decls("fun __h(t) = t;").unwrap();
        let d = resolve_units(&[imp, vec![]]).expect("склейка обязана сходиться");
        let c = cond_of("__h(at) == 1");
        let e = check_single(&c, &d).expect_err("чужое __ — ошибка");
        assert_eq!((e.code, e.message.as_str()), ("E11", "unknown name '__h'"));
        // Своё `__` внутри своего файла работает.
        let prog = p::parse_decls("fun __p(t) = t + 1;").unwrap();
        let d = resolve_units(&[vec![], prog]).expect("склейка обязана сходиться");
        let c = cond_of("__p(at) == 3");
        check_single(&c, &d).unwrap();
        assert!(eval_cond(&c, 2, &d).unwrap());
    }

    #[test]
    fn duplicate_across_files_is_not_e04() {
        // Дубль — только внутри одного файла.
        let a = p::parse_decls("const K = 1;").unwrap();
        let b = p::parse_decls("const K = 2;").unwrap();
        let d = resolve_units(&[a, b]).expect("склейка обязана сходиться");
        let c = cond_of("at >= K");
        check_single(&c, &d).unwrap();
        assert!(eval_cond(&c, 2, &d).unwrap());
        assert!(!eval_cond(&c, 1, &d).unwrap());
    }
}
