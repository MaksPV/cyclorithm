//! Условия строк (§3–§5 спеки): типизация и вычисление.
//!
//! Всё, кроме значения `at`, известно статически: проверка типов,
//! имён, арностей и константного деления на ноль идёт валидацией
//! (`check_conditions`, E11/E12) в порядке объявления, деление
//! на ноль выражением ловится вычислением в момент строки (тоже E12).

use cycloritm_parser::{ArithOp, CmpOp, Cond, CondRhs, Expr, Schedule};

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

/// Проверить все условия файла в порядке объявления: сначала циклы,
// затем `root_cycle`. Первая ошибка побеждает.
pub fn check_conditions(schedule: &Schedule) -> Result<(), Error> {
    for c in &schedule.cycles {
        for st in &c.stmts {
            if let Some(cond) = &st.condition {
                check_cond(cond)?;
            }
        }
    }
    for st in &schedule.root.stmts {
        if let Some(cond) = &st.condition {
            check_cond(cond)?;
        }
    }
    Ok(())
}

fn check_cond(cond: &Cond) -> Result<(), Error> {
    match cond {
        Cond::Or(cs) | Cond::And(cs) => cs.iter().try_for_each(check_cond),
        Cond::Not(c) => check_cond(c),
        Cond::Cmp { left, right, .. } => {
            let lt = check_expr(left)?;
            match right {
                CondRhs::One(r) => {
                    if check_expr(r)? != lt {
                        return Err(Error::e12_mismatch());
                    }
                }
                CondRhs::Alt(alts) => {
                    for a in alts {
                        if check_expr(a)? != Ty::Num {
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

/// Тип выражения + заодно: неизвестные имена (E11), арности (E12),
/// переполнение литералов (E12), константное деление на ноль (E12).
fn check_expr(expr: &Expr) -> Result<Ty, Error> {
    match expr {
        Expr::Num(raw) => {
            raw.parse::<i64>().map_err(|_| Error::e12_range(raw))?;
            Ok(Ty::Num)
        }
        Expr::Str(_) => Ok(Ty::Str),
        Expr::At => Ok(Ty::Num),
        Expr::Name(name) => Err(Error::e11(name)),
        Expr::Neg(x) => check_expr(x),
        Expr::Bin { left, right, .. } => {
            if check_expr(left)? != Ty::Num || check_expr(right)? != Ty::Num {
                return Err(Error::e12_mismatch());
            }
            if let Some(Err(e)) = const_div(right) {
                return Err(e);
            }
            Ok(Ty::Num)
        }
        Expr::Concat(xs) => {
            for x in xs {
                if check_expr(x)? != Ty::Str {
                    return Err(Error::e12_mismatch());
                }
            }
            Ok(Ty::Str)
        }
        Expr::Call { name, args } => check_call(name, args),
    }
}

fn check_call(name: &str, args: &[Expr]) -> Result<Ty, Error> {
    match name {
        "str" => {
            if args.len() != 1 {
                return Err(Error::e12_arity(name));
            }
            if check_expr(&args[0])? != Ty::Num {
                return Err(Error::e12_mismatch());
            }
            Ok(Ty::Str)
        }
        "pad" => {
            if args.len() != 2 {
                return Err(Error::e12_arity(name));
            }
            for a in args {
                if check_expr(a)? != Ty::Num {
                    return Err(Error::e12_mismatch());
                }
            }
            Ok(Ty::Str)
        }
        _ => Err(Error::e11(name)),
    }
}

/// Константный делитель: `Some(Err)` — статический ноль (или другая
// статическая ошибка — она же всплывёт первой), `Some(Ok)` — ненулевая
// константа, `None` — зависит от `at`, проверит вычисление.
fn const_div(expr: &Expr) -> Option<Result<(), Error>> {
    match const_eval(expr)? {
        Err(e) => Some(Err(e)),
        Ok(Value::Num(0)) => Some(Err(Error::e12_divzero())),
        Ok(_) => Some(Ok(())),
    }
}

/// Вычисление константы; `None` — внутри есть `at`.
fn const_eval(expr: &Expr) -> Option<Result<Value, Error>> {
    if has_at(expr) {
        return None;
    }
    Some(eval_expr(expr, 0))
}

fn has_at(expr: &Expr) -> bool {
    match expr {
        Expr::At => true,
        Expr::Num(_) | Expr::Str(_) | Expr::Name(_) => false,
        Expr::Neg(x) => has_at(x),
        Expr::Bin { left, right, .. } => has_at(left) || has_at(right),
        Expr::Concat(xs) => xs.iter().any(has_at),
        Expr::Call { args, .. } => args.iter().any(has_at),
    }
}

/// Вычислить условие для абсолютного времени строки (`at`).
pub fn eval_cond(cond: &Cond, at: i64) -> Result<bool, Error> {
    match cond {
        Cond::Or(cs) => {
            for c in cs {
                if eval_cond(c, at)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Cond::And(cs) => {
            for c in cs {
                if !eval_cond(c, at)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Cond::Not(c) => Ok(!eval_cond(c, at)?),
        Cond::Cmp { op, left, right } => {
            let l = eval_expr(left, at)?;
            match right {
                CondRhs::One(r) => {
                    let r = eval_expr(r, at)?;
                    cmp_values(*op, &l, &r)
                }
                CondRhs::Alt(alts) => {
                    if !matches!(op, CmpOp::Eq | CmpOp::Ne) {
                        return Err(Error::e12_mismatch());
                    }
                    let mut any_eq = false;
                    for a in alts {
                        let v = eval_expr(a, at)?;
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
pub fn eval_expr(expr: &Expr, at: i64) -> Result<Value, Error> {
    match expr {
        Expr::Num(raw) => Ok(Value::Num(raw.parse().map_err(|_| Error::e12_range(raw))?)),
        Expr::Str(s) => Ok(Value::Str(s.clone())),
        Expr::At => Ok(Value::Num(at)),
        Expr::Name(name) => Err(Error::e11(name)),
        Expr::Neg(x) => match eval_expr(x, at)? {
            Value::Num(v) => v
                .checked_neg()
                .map(Value::Num)
                .ok_or_else(|| Error::e12_range("negation overflow")),
            Value::Str(_) => Err(Error::e12_mismatch()),
        },
        Expr::Bin { op, left, right } => {
            let (a, b) = match (eval_expr(left, at)?, eval_expr(right, at)?) {
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
                match eval_expr(x, at)? {
                    Value::Str(s) => out.push_str(&s),
                    Value::Num(_) => return Err(Error::e12_mismatch()),
                }
            }
            Ok(Value::Str(out))
        }
        Expr::Call { name, args } => match name.as_str() {
            "str" => {
                let a = args.first().ok_or_else(|| Error::e12_arity(name))?;
                match eval_expr(a, at)? {
                    Value::Num(n) => Ok(Value::Str(n.to_string())),
                    Value::Str(_) => Err(Error::e12_mismatch()),
                }
            }
            "pad" => {
                let (an, aw) = match (args.first(), args.get(1)) {
                    (Some(a), Some(b)) => (a, b),
                    _ => return Err(Error::e12_arity(name)),
                };
                let (n, w) = match (eval_expr(an, at)?, eval_expr(aw, at)?) {
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
            _ => Err(Error::e11(name)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond_of(row: &str) -> Cond {
        let src = format!(
            "schedule \"T\" {{ point A {{ actions = [x]; }} \
            cycle R duration = 1h {{ 0m: A.x(); }} \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ [{row}] 6h: R(); }} }}"
        );
        let s = cycloritm_parser::parse(&src).expect("фикстура обязана разбираться");
        s.root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть")
    }

    fn yes(row: &str, at: i64) -> bool {
        let c = cond_of(row);
        check_cond(&c).expect("условие обязано проходить проверку");
        eval_cond(&c, at).expect("вычисление обязано удаваться")
    }

    fn no(row: &str, at: i64) -> bool {
        !yes(row, at)
    }

    fn static_err(row: &str) -> Error {
        check_cond(&cond_of(row)).expect_err("ожидалась ошибка проверки")
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
    fn rejects_unknown_names() {
        let e = static_err("hour(at) == 1");
        assert_eq!((e.code, e.message.as_str()), ("E11", "unknown name 'hour'"));
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
        check_cond(&c).expect("делитель не константа — статика проходит");
        let e = eval_cond(&c, 100).expect_err("ноль в момент строки — ошибка");
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
}
