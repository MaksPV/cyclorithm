//! Grammar and AST for the Cyclorithm DSL.

use pest::iterators::Pair;
use pest::Parser as _;
use pest_derive::Parser;

/// Парсер грамматики из §3 спеки (см. `grammar.pest`).
#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct CycloParser;

/// Разбор исходника в AST. Ошибка — синтаксическая, без E-кода
/// (коды E01–E09 — только валидация уже разобранного AST в ядре).
pub fn parse(src: &str) -> Result<SourceFile, pest::error::Error<Rule>> {
    let file = CycloParser::parse(Rule::file, src)?
        .next()
        .expect("file непуст");
    debug_assert_eq!(file.as_rule(), Rule::file);
    let mut uses = Vec::new();
    let mut decls = Vec::new();
    let mut schedule = None;
    for p in file.into_inner() {
        match p.as_rule() {
            Rule::use_decl => uses.push(build_use(p)),
            Rule::decl => decls.push(build_decl(p)?),
            Rule::schedule => schedule = Some(build_schedule(p)?),
            Rule::EOI => {}
            r => unreachable!("file: неожиданное правило {r:?}"),
        }
    }
    Ok(SourceFile {
        uses,
        decls,
        schedule: schedule.expect("file содержит ровно schedule"),
    })
}

/// Позиция синтаксической ошибки для редактора: 1-базные
/// (строка, колонка); у спан-ошибки — начало спана.
pub fn error_position(e: &pest::error::Error<Rule>) -> (usize, usize) {
    match e.line_col {
        pest::error::LineColLocation::Pos((l, c)) => (l, c),
        pest::error::LineColLocation::Span((sl, sc), _) => (sl, sc),
    }
}

/// Путь из `use "path";` — без кавычек (строки без escapes, как везде).
fn build_use(pair: Pair<Rule>) -> String {
    debug_assert_eq!(pair.as_rule(), Rule::use_decl);
    let s = pair.into_inner().next().expect("use: путь").as_str();
    s[1..s.len() - 1].to_owned()
}

/// Разбор файла одних объявлений (системная библиотека): расписания нет.
pub fn parse_decls(src: &str) -> Result<Vec<Decl>, pest::error::Error<Rule>> {
    let file = CycloParser::parse(Rule::decls_file, src)?
        .next()
        .expect("decls_file непуст");
    debug_assert_eq!(file.as_rule(), Rule::decls_file);
    file.into_inner()
        .filter(|p| p.as_rule() == Rule::decl)
        .map(build_decl)
        .collect()
}

/// Единица импорта: свои `use`, объявления, флаг наличия `schedule`
/// (расписание внутри импорта запрещено кодом E14 — решает резолвер).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitFile {
    pub uses: Vec<String>,
    pub decls: Vec<Decl>,
    pub has_schedule: bool,
}

/// Разбор импортированного файла: объявления и транзитивные `use`.
pub fn parse_unit(src: &str) -> Result<UnitFile, pest::error::Error<Rule>> {
    let file = CycloParser::parse(Rule::unit_file, src)?
        .next()
        .expect("unit_file непуст");
    debug_assert_eq!(file.as_rule(), Rule::unit_file);
    let mut uses = Vec::new();
    let mut decls = Vec::new();
    let mut has_schedule = false;
    for p in file.into_inner() {
        match p.as_rule() {
            Rule::use_decl => uses.push(build_use(p)),
            Rule::decl => decls.push(build_decl(p)?),
            Rule::schedule => {
                // Тело не строим: наличие расписания — уже E14.
                has_schedule = true;
            }
            Rule::EOI => {}
            r => unreachable!("unit_file: неожиданное правило {r:?}"),
        }
    }
    Ok(UnitFile {
        uses,
        decls,
        has_schedule,
    })
}

/// Объявление верхнего уровня: `const` — число, `fun` — число от аргумента,
/// `pred` — истина/ложь (параметр буквально `at`, иначе синтаксис).
fn build_decl(pair: Pair<Rule>) -> Result<Decl, pest::error::Error<Rule>> {
    debug_assert_eq!(pair.as_rule(), Rule::decl);
    let kind = pair.into_inner().next().expect("decl: const, fun или pred");
    match kind.as_rule() {
        Rule::const_decl => {
            let mut inner = kind.into_inner();
            let name = inner.next().expect("const: имя").as_str().to_owned();
            let body = build_operand(
                inner
                    .next()
                    .expect("const: тело")
                    .into_inner()
                    .next()
                    .expect("decl_value: содержимое"),
            );
            Ok(Decl::Const { name, body })
        }
        Rule::fun_decl => {
            let mut inner = kind.into_inner();
            let name = inner.next().expect("fun: имя").as_str().to_owned();
            let param = inner.next().expect("fun: параметр").as_str().to_owned();
            let body = build_operand(
                inner
                    .next()
                    .expect("fun: тело")
                    .into_inner()
                    .next()
                    .expect("decl_value: содержимое"),
            );
            Ok(Decl::Fun { name, param, body })
        }
        Rule::pred_decl => {
            let span = kind.as_span();
            let mut inner = kind.into_inner();
            let name = inner.next().expect("pred: имя").as_str().to_owned();
            let param = inner.next().expect("pred: параметр");
            if param.as_str() != "at" {
                return Err(pest::error::Error::new_from_span(
                    pest::error::ErrorVariant::CustomError {
                        message: "predicate parameter must be 'at'".to_owned(),
                    },
                    span,
                ));
            }
            let body = build_or(inner.next().expect("pred: тело"));
            Ok(Decl::Pred { name, body })
        }
        r => unreachable!("decl: неожиданное правило {r:?}"),
    }
}

fn build_schedule(pair: Pair<Rule>) -> Result<Schedule, pest::error::Error<Rule>> {
    debug_assert_eq!(pair.as_rule(), Rule::schedule);
    let mut inner = pair.into_inner();
    let name = unquote(inner.next().expect("schedule: имя"));
    let mut points = Vec::new();
    let mut cycles = Vec::new();
    let mut root = None;
    for p in inner {
        match p.as_rule() {
            Rule::point => points.push(build_point(p)),
            Rule::cycle => cycles.push(build_cycle(p)?),
            Rule::root_cycle => root = Some(build_root_cycle(p)?),
            r => unreachable!("schedule: неожиданное правило {r:?}"),
        }
    }
    Ok(Schedule {
        name,
        points,
        cycles,
        root: root.expect("schedule: root_cycle обязателен"),
    })
}

fn build_point(pair: Pair<Rule>) -> Point {
    let mut inner = pair.into_inner();
    let name = inner.next().expect("point: имя").as_str().to_owned();
    let actions = inner
        .next()
        .expect("point: actions")
        .into_inner()
        .map(|a| a.as_str().to_owned())
        .collect();
    let attrs = inner.next().map(|p| {
        debug_assert_eq!(p.as_rule(), Rule::point_attrs);
        let src = p
            .into_inner()
            .next()
            .expect("point_attrs: источник")
            .into_inner()
            .next()
            .expect("attrs_src: содержимое");
        match src.as_rule() {
            Rule::map_lit => build_map_lit(src),
            Rule::IDENT => Expr::Name(src.as_str().to_owned()),
            r => unreachable!("attrs_src: неожиданное правило {r:?}"),
        }
    });
    Point {
        name,
        actions,
        attrs,
    }
}

fn build_cycle(pair: Pair<Rule>) -> Result<Cycle, pest::error::Error<Rule>> {
    let mut inner = pair.into_inner();
    let name = inner.next().expect("cycle: имя").as_str().to_owned();
    let mut next = inner.next().expect("cycle: параметры или duration");
    let params = if next.as_rule() == Rule::cycle_params {
        let ps = next
            .into_inner()
            .map(|p| p.as_str().to_owned())
            .collect();
        next = inner.next().expect("cycle: duration");
        ps
    } else {
        Vec::new()
    };
    let duration = build_duration(next);
    let stmts = inner.map(build_stmt).collect::<Result<_, _>>()?;
    Ok(Cycle {
        name,
        params,
        duration,
        stmts,
    })
}

fn build_root_cycle(pair: Pair<Rule>) -> Result<RootCycle, pest::error::Error<Rule>> {
    let mut inner = pair.into_inner();
    let start_time = unquote(inner.next().expect("root_cycle: start_time"));
    let duration = build_duration(inner.next().expect("root_cycle: duration"));
    let stmts = inner.map(build_stmt).collect::<Result<_, _>>()?;
    Ok(RootCycle {
        start_time,
        duration,
        stmts,
    })
}

fn build_stmt(pair: Pair<Rule>) -> Result<Stmt, pest::error::Error<Rule>> {
    debug_assert_eq!(pair.as_rule(), Rule::stmt);
    let span = pair.as_span();
    let mut inner = pair.into_inner();
    let mut first = inner.next().expect("stmt: условие, минус или смещение");
    let mut condition = None;
    if first.as_rule() == Rule::condition_block {
        let cond = first.into_inner().next().expect("condition_block: условие");
        condition = Some(build_cond(cond));
        first = inner.next().expect("stmt: смещение или минус");
    }
    // Минус смещения (§3 спеки): пишется слитно (`-10m` ок, `- 10m` — ошибка).
    // Грамматика пробел пропускает осознанно — границу проверяем по спанам.
    let (negative, offset_pair) = if first.as_rule() == Rule::neg_sign {
        let offset_pair = inner.next().expect("stmt: длительность после минуса");
        if first.as_span().end() != offset_pair.as_span().start() {
            return Err(pest::error::Error::new_from_span(
                pest::error::ErrorVariant::CustomError {
                    message: "minus in offset must be glued to duration ('-10m')".to_owned(),
                },
                span,
            ));
        }
        (true, offset_pair)
    } else {
        (false, first)
    };
    let offset = build_duration(offset_pair);
    let body = inner.next().expect("stmt: тело");
    debug_assert_eq!(body.as_rule(), Rule::stmt_body);
    let mut binner = body.into_inner();
    let bfirst = binner.next().expect("stmt_body: модификатор или вызов");
    let (repeat, call) = if bfirst.as_rule() == Rule::repeat_mod {
        let repeat = build_repeat(bfirst)?;
        let call = binner
            .next()
            .expect("stmt: вызов после модификатора")
            .into_inner()
            .next()
            .expect("invocation: вызов");
        (repeat, call)
    } else {
        let call = bfirst.into_inner().next().expect("invocation: вызов");
        (Repeat::Once, call)
    };
    let invocation = match call.as_rule() {
        Rule::point_action => {
            let mut parts = call.into_inner();
            let point = parts.next().expect("вызов: точка").as_str().to_owned();
            let action = parts.next().expect("вызов: действие").as_str().to_owned();
            let block = parts
                .next()
                .map(|b| {
                    debug_assert_eq!(b.as_rule(), Rule::action_block);
                    b.into_inner().map(|p| {
                        debug_assert_eq!(p.as_rule(), Rule::block_pair);
                        let mut kv = p.into_inner();
                        let key = kv.next().expect("block_pair: ключ").as_str().to_owned();
                        let value =
                            build_call_arg(kv.next().expect("block_pair: значение"));
                        (key, value)
                    })
                })
                .map(|it| it.collect::<Vec<_>>())
                .unwrap_or_default();
            Invocation::PointAction {
                point,
                action,
                block,
            }
        }
        Rule::cycle_call => {
            let mut parts = call.into_inner();
            let name = parts
                .next()
                .expect("вызов: цикл")
                .as_str()
                .to_owned();
            let args = parts.map(build_call_arg).collect();
            Invocation::CycleCall { name, args }
        }
        r => unreachable!("stmt: неожиданный вызов {r:?}"),
    };
    Ok(Stmt {
        offset,
        negative,
        repeat,
        condition,
        invocation,
    })
}

/// Условие строки (§3 спеки): логика над сравнениями.
fn build_cond(pair: Pair<Rule>) -> Cond {
    debug_assert_eq!(pair.as_rule(), Rule::condition);
    build_or(pair.into_inner().next().expect("condition: or_expr"))
}

fn build_or(pair: Pair<Rule>) -> Cond {
    debug_assert_eq!(pair.as_rule(), Rule::or_expr);
    let mut inner = pair.into_inner();
    let mut acc = build_and(inner.next().expect("or_expr: левый операнд"));
    while inner.next().is_some() {
        let rhs = build_and(inner.next().expect("or_expr: правый операнд"));
        acc = match acc {
            Cond::Or(mut all) => {
                all.push(rhs);
                Cond::Or(all)
            }
            other => Cond::Or(vec![other, rhs]),
        };
    }
    acc
}

fn build_not(pair: Pair<Rule>) -> Cond {
    debug_assert_eq!(pair.as_rule(), Rule::not_expr);
    let mut inner = pair.into_inner();
    let first = inner.next().expect("not_expr: операнд");
    let (negated, atom) = if first.as_rule() == Rule::kw_not {
        (true, inner.next().expect("not_expr: операнд после not"))
    } else {
        (false, first)
    };
    let cond = match atom.as_rule() {
        Rule::bool_group => build_or(atom.into_inner().next().expect("bool_group: выражение")),
        Rule::comparison => build_comparison(atom),
        Rule::call => {
            let (name, args) = build_call_parts(atom);
            Cond::Pred { name, args }
        }
        r => unreachable!("not_expr: неожиданный операнд {r:?}"),
    };
    if negated {
        Cond::Not(Box::new(cond))
    } else {
        cond
    }
}

fn build_and(pair: Pair<Rule>) -> Cond {
    debug_assert_eq!(pair.as_rule(), Rule::and_expr);
    let mut inner = pair.into_inner();
    let mut acc = build_not(inner.next().expect("and_expr: левый операнд"));
    while inner.next().is_some() {
        let rhs = build_not(inner.next().expect("and_expr: правый операнд"));
        acc = match acc {
            Cond::And(mut all) => {
                all.push(rhs);
                Cond::And(all)
            }
            other => Cond::And(vec![other, rhs]),
        };
    }
    acc
}

fn build_comparison(pair: Pair<Rule>) -> Cond {
    debug_assert_eq!(pair.as_rule(), Rule::comparison);
    let mut inner = pair.into_inner();
    let left = build_operand(
        inner
            .next()
            .expect("comparison: левая часть")
            .into_inner()
            .next()
            .expect("cmp_side: содержимое"),
    );
    let op = match inner.next().expect("comparison: оператор").as_str() {
        "==" => CmpOp::Eq,
        "!=" => CmpOp::Ne,
        "<" => CmpOp::Lt,
        "<=" => CmpOp::Le,
        ">" => CmpOp::Gt,
        ">=" => CmpOp::Ge,
        o => unreachable!("cmp_op: неожиданный оператор {o:?}"),
    };
    let right = inner
        .next()
        .expect("comparison: правая часть")
        .into_inner()
        .next()
        .expect("cmp_right: содержимое");
    let right = if right.as_rule() == Rule::alternation {
        let mut alts = right.into_inner();
        let mut values = vec![build_bitor(alts.next().expect("alternation: ветка"))];
        while alts.next().is_some() {
            values.push(build_bitor(alts.next().expect("alternation: ветка")));
        }
        CondRhs::Alt(values)
    } else {
        CondRhs::One(build_operand(right))
    };
    Cond::Cmp { op, left, right }
}

/// Операнд сравнения: склейка или битовое выражение. Обёртки (`cmp_side`,
/// `cmp_right`, `cond_arg`) снимает вызывающий.
fn build_operand(pair: Pair<Rule>) -> Expr {
    match pair.as_rule() {
        Rule::concat => Expr::Concat(pair.into_inner().map(build_concat_term).collect()),
        Rule::bitor => build_bitor(pair),
        r => unreachable!("операнд: неожиданное правило {r:?}"),
    }
}

fn build_concat_term(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::concat_term);
    let inner = pair.into_inner().next().expect("concat_term: значение");
    match inner.as_rule() {
        Rule::postfix => build_postfix(inner),
        Rule::concat => Expr::Concat(inner.into_inner().map(build_concat_term).collect()),
        r => unreachable!("concat_term: неожиданное правило {r:?}"),
    }
}

fn build_arith(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::arith);
    let mut inner = pair.into_inner();
    let mut acc = build_term(inner.next().expect("arith: левый операнд"));
    while let Some(op) = inner.next() {
        let rhs = build_term(inner.next().expect("arith: правый операнд"));
        let op = match op.as_str() {
            "+" => ArithOp::Add,
            "-" => ArithOp::Sub,
            o => unreachable!("add_op: неожиданный оператор {o:?}"),
        };
        acc = Expr::Bin {
            op,
            left: Box::new(acc),
            right: Box::new(rhs),
        };
    }
    acc
}

/// Битовые уровни — та же левоассоциативная свёртка, что у арифметики.
fn build_bitor(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::bitor);
    let mut inner = pair.into_inner();
    let mut acc = build_bitxor(inner.next().expect("bitor: левый операнд"));
    // `"|"` — безымянный литерал, пары не даёт: дальше идут только операнды.
    for rhs in inner {
        acc = Expr::Bit {
            op: BitOp::Or,
            left: Box::new(acc),
            right: Box::new(build_bitxor(rhs)),
        };
    }
    acc
}

fn build_bitxor(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::bitxor);
    let mut inner = pair.into_inner();
    let mut acc = build_bitand(inner.next().expect("bitxor: левый операнд"));
    for rhs in inner {
        acc = Expr::Bit {
            op: BitOp::Xor,
            left: Box::new(acc),
            right: Box::new(build_bitand(rhs)),
        };
    }
    acc
}

fn build_bitand(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::bitand);
    let mut inner = pair.into_inner();
    let mut acc = build_shift(inner.next().expect("bitand: левый операнд"));
    for rhs in inner {
        acc = Expr::Bit {
            op: BitOp::And,
            left: Box::new(acc),
            right: Box::new(build_shift(rhs)),
        };
    }
    acc
}

fn build_shift(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::shift);
    let mut inner = pair.into_inner();
    let mut acc = build_arith(inner.next().expect("shift: левый операнд"));
    while let Some(op) = inner.next() {
        let rhs = build_arith(inner.next().expect("shift: правый операнд"));
        let op = match op.as_str() {
            "<<" => BitOp::Shl,
            ">>" => BitOp::Shr,
            o => unreachable!("shift_op: неожиданный оператор {o:?}"),
        };
        acc = Expr::Bit {
            op,
            left: Box::new(acc),
            right: Box::new(rhs),
        };
    }
    acc
}

fn build_term(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::term);
    let mut inner = pair.into_inner();
    let mut acc = build_factor(inner.next().expect("term: левый операнд"));
    while let Some(op) = inner.next() {
        let rhs = build_factor(inner.next().expect("term: правый операнд"));
        let op = match op.as_str() {
            "*" => ArithOp::Mul,
            "/" => ArithOp::Div,
            "%" => ArithOp::Mod,
            "floordiv" => ArithOp::FloorDiv,
            "floormod" => ArithOp::FloorMod,
            o => unreachable!("mul_op: неожиданный оператор {o:?}"),
        };
        acc = Expr::Bin {
            op,
            left: Box::new(acc),
            right: Box::new(rhs),
        };
    }
    acc
}

fn build_factor(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::factor);
    let mut inner = pair.into_inner();
    let first = inner.next().expect("factor: операнд");
    let (negated, value) = if first.as_rule() == Rule::neg_sign {
        (true, inner.next().expect("factor: операнд после минуса"))
    } else {
        (false, first)
    };
    let expr = match value.as_rule() {
        Rule::postfix => build_postfix(value),
        Rule::bitor => build_bitor(value),
        r => unreachable!("factor: неожиданное правило {r:?}"),
    };
    if negated {
        Expr::Neg(Box::new(expr))
    } else {
        expr
    }
}

/// Постфикс (§3 спеки): база и цепочка `.поле` / `[n]`, свёртка слева.
fn build_postfix(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::postfix);
    let mut inner = pair.into_inner();
    let mut acc = build_postfix_base(inner.next().expect("postfix: база"));
    for wrapper in inner {
        debug_assert_eq!(wrapper.as_rule(), Rule::postfix_suffix);
        let suffix = wrapper
            .into_inner()
            .next()
            .expect("postfix_suffix: содержимое");
        match suffix.as_rule() {
            Rule::field_access => {
                let field = suffix
                    .into_inner()
                    .next()
                    .expect("field_access: имя")
                    .as_str()
                    .to_owned();
                acc = Expr::Field {
                    base: Box::new(acc),
                    field,
                };
            }
            Rule::index_access => {
                let text = suffix.as_str();
                let index = text[1..text.len() - 1].to_owned();
                acc = Expr::Index {
                    base: Box::new(acc),
                    index,
                };
            }
            r => unreachable!("postfix: неожиданный суффикс {r:?}"),
        }
    }
    acc
}

/// База постфикса: прежние листья `factor` плюс JSON-литералы и `true`/`false`.
fn build_postfix_base(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::postfix_base);
    let pair = pair.into_inner().next().expect("postfix_base: содержимое");
    match pair.as_rule() {
        Rule::number => Expr::Num(pair.as_str().to_owned()),
        Rule::string => {
            let s = pair.as_str();
            Expr::Str(s[1..s.len() - 1].to_owned())
        }
        Rule::bool_lit => Expr::Bool(pair.as_str() == "true"),
        Rule::map_lit => build_map_lit(pair),
        Rule::array_lit => build_array_lit(pair),
        Rule::call => {
            let (name, args) = build_call_parts(pair);
            Expr::Call { name, args }
        }
        Rule::IDENT => {
            if pair.as_str() == "at" {
                Expr::At
            } else {
                Expr::Name(pair.as_str().to_owned())
            }
        }
        Rule::truth => {
            let expr = pair.into_inner().next().expect("truth: условие");
            Expr::Truth(Box::new(build_or(expr)))
        }
        Rule::bitor => build_bitor(pair),
        r => unreachable!("postfix_base: неожиданное правило {r:?}"),
    }
}

/// Мапа `{"k": v, ...}`: ключи — строки без кавычек, значения — литералы.
/// Пары хранятся вектором как есть (дубли — E15 в ядре, не синтаксис).
fn build_map_lit(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::map_lit);
    let pairs = pair
        .into_inner()
        .map(|p| {
            debug_assert_eq!(p.as_rule(), Rule::map_pair);
            let mut kv = p.into_inner();
            let key = unquote(kv.next().expect("map_pair: ключ"));
            let value = build_literal_value(kv.next().expect("map_pair: значение"));
            (key, value)
        })
        .collect();
    Expr::Map(pairs)
}

fn build_array_lit(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::array_lit);
    Expr::Array(pair.into_inner().map(build_literal_value).collect())
}

fn build_literal_value(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::literal_value);
    let inner = pair.into_inner().next().expect("literal_value: литерал");
    match inner.as_rule() {
        Rule::number => Expr::Num(inner.as_str().to_owned()),
        Rule::string => {
            let s = inner.as_str();
            Expr::Str(s[1..s.len() - 1].to_owned())
        }
        Rule::bool_lit => Expr::Bool(inner.as_str() == "true"),
        Rule::map_lit => build_map_lit(inner),
        Rule::array_lit => build_array_lit(inner),
        r => unreachable!("literal_value: неожиданное правило {r:?}"),
    }
}

fn build_call_parts(pair: Pair<Rule>) -> (String, Vec<Expr>) {
    debug_assert_eq!(pair.as_rule(), Rule::call);
    let mut inner = pair.into_inner();
    let name = inner.next().expect("call: имя").as_str().to_owned();
    let args = inner.map(build_call_arg).collect();
    (name, args)
}

fn build_call_arg(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::cond_arg);
    build_operand(pair.into_inner().next().expect("cond_arg: выражение"))
}
fn build_repeat(pair: Pair<Rule>) -> Result<Repeat, pest::error::Error<Rule>> {
    debug_assert_eq!(pair.as_rule(), Rule::repeat_mod);
    let span = pair.as_span();
    let kind = pair
        .into_inner()
        .next()
        .expect("repeat_mod: repeat_n или fill_mod");
    match kind.as_rule() {
        Rule::repeat_n => {
            let count = kind
                .into_inner()
                .next()
                .expect("repeat: число")
                .as_str()
                .to_owned();
            Ok(Repeat::Times(count))
        }
        Rule::fill_mod => {
            let mut finner = kind.into_inner();
            let first = finner.next();
            match first {
                None => Ok(Repeat::Fill { until: None }),
                Some(p) if p.as_rule() == Rule::neg_sign => {
                    let dur = finner.next().expect("until: длительность после минуса");
                    if p.as_span().end() != dur.as_span().start() {
                        return Err(pest::error::Error::new_from_span(
                            pest::error::ErrorVariant::CustomError {
                                message: "minus in until must be glued to duration ('-2h')"
                                    .to_owned(),
                            },
                            span,
                        ));
                    }
                    Ok(Repeat::Fill {
                        until: Some(Until {
                            negative: true,
                            duration: build_duration(dur),
                        }),
                    })
                }
                Some(p) => {
                    debug_assert_eq!(p.as_rule(), Rule::duration);
                    Ok(Repeat::Fill {
                        until: Some(Until {
                            negative: false,
                            duration: build_duration(p),
                        }),
                    })
                }
            }
        }
        r => unreachable!("repeat_mod: неожиданное правило {r:?}"),
    }
}

fn build_duration(pair: Pair<Rule>) -> Duration {
    // Спан повторения `duration_item+` иногда захватывает пробелы/перенос
    // перед следующим токеном (напр. `"24h\n  "` перед `{`). Семантику несут
    // `items`, а `raw` идёт в сообщения E05 — висячий хвост срезаем.
    let raw = pair.as_str().trim_end().to_owned();
    let items = pair
        .into_inner()
        .map(|item| {
            let mut parts = item.into_inner();
            let number = parts
                .next()
                .expect("duration_item: число")
                .as_str()
                .to_owned();
            let unit = match parts.next().expect("duration_item: юнит").as_str() {
                "w" => DurationUnit::Week,
                "d" => DurationUnit::Day,
                "h" => DurationUnit::Hour,
                "m" => DurationUnit::Minute,
                "s" => DurationUnit::Second,
                "ms" => DurationUnit::Millisecond,
                u => unreachable!("duration_unit: неожиданный юнит {u:?}"),
            };
            DurationItem { number, unit }
        })
        .collect();
    Duration { raw, items }
}

/// Снять кавычки `"..."`. Экранирования в строках нет, кавычка внутри
/// непредставима — среза достаточно.
fn unquote(pair: Pair<Rule>) -> String {
    let s = pair.as_str();
    s[1..s.len() - 1].to_owned()
}

// ---------------------------------------------------------------------------
// AST — строго по §3 спеки, без валидации.
// Проверки E01–E09 — дело ядра над уже разобранным AST: парсер принимает
// и `1h2h`, и переполнение, и `duration = 0`, ничего числового не решает.
// Поэтому числа и сырой текст длительностей хранятся как есть.
// ---------------------------------------------------------------------------

/// Корень файла: импорты, объявления и `schedule`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub uses: Vec<String>,
    pub decls: Vec<Decl>,
    pub schedule: Schedule,
}

/// Объявление верхнего уровня (§3 спеки).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decl {
    Const {
        name: String,
        body: Expr,
    },
    Fun {
        name: String,
        param: String,
        body: Expr,
    },
    Pred {
        name: String,
        body: Cond,
    },
}

/// Корень расписания: `schedule "имя" { point* cycle* root_cycle }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub name: String,
    pub points: Vec<Point>,
    pub cycles: Vec<Cycle>,
    pub root: RootCycle,
}

/// `point DEPOT { actions = [depart, arrive]; }`
/// (`attrs = ...;` — опционально: литерал мапы или ссылка на константу-мапу).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub name: String,
    pub actions: Vec<String>,
    pub attrs: Option<Expr>,
}

/// `cycle CITY_ROUTE duration = 1h20m { ... }`
/// (`LESSON(subj)` — параметры для данных строк, см. черновик `attrs.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    pub name: String,
    pub params: Vec<String>,
    pub duration: Duration,
    pub stmts: Vec<Stmt>,
}

/// `root_cycle start_time = "...", duration = 24h { ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootCycle {
    /// Сырая строка без кавычек; корректность дат — E08 в ядре.
    pub start_time: String,
    pub duration: Duration,
    pub stmts: Vec<Stmt>,
}

/// Одна строка цикла: `[условие] [минус] <смещение>: [<повтор>] <вызов>;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub offset: Duration,
    pub negative: bool,
    pub repeat: Repeat,
    pub condition: Option<Cond>,
    pub invocation: Invocation,
}

/// Условие строки: логика над сравнениями.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    Or(Vec<Cond>),
    And(Vec<Cond>),
    Not(Box<Cond>),
    Pred {
        name: String,
        args: Vec<Expr>,
    },
    Cmp {
        op: CmpOp,
        left: Expr,
        right: CondRhs,
    },
}

/// Правая часть сравнения: одиночное значение или альтернация `(a or b)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CondRhs {
    One(Expr),
    Alt(Vec<Expr>),
}

/// Выражение условия: числа — сырым текстом, `at` — время строки.
/// `Bool`/`Map`/`Array` — JSON-значения (черновик `attrs.md`): литералы
/// и доступ `.поле` / `[n]`; вычисляются в момент строки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Num(String),
    Str(String),
    Bool(bool),
    Map(Vec<(String, Expr)>),
    Array(Vec<Expr>),
    Field { base: Box<Expr>, field: String },
    Index { base: Box<Expr>, index: String },
    At,
    Name(String),
    Neg(Box<Expr>),
    Bin {
        op: ArithOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Bit {
        op: BitOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Concat(Vec<Expr>),
    Truth(Box<Cond>),
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

/// Оператор сравнения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Арифметический оператор.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    FloorDiv,
    FloorMod,
}

/// Битовый оператор (§4.13 спеки): только над числами, с wrap-семантикой.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitOp {
    Shl,
    Shr,
    And,
    Or,
    Xor,
}

impl Stmt {
    /// Сырой текст смещения для сообщений E07: с минусом (`'-2h'`) или без.
    pub fn offset_raw(&self) -> String {
        if self.negative {
            format!("-{}", self.offset.raw)
        } else {
            self.offset.raw.clone()
        }
    }
}

/// Модификатор повторов строки (§3 спеки): `repeat N` / `fill` / `fill until`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    /// Без модификатора: одиночный вызов.
    Once,
    /// `repeat N`: ровно N экземпляров (`N ≥ 1`, иначе E10).
    Times(String),
    /// `fill [until [−]T]`: мягкое заполнение до горизонта.
    Fill { until: Option<Until> },
}

/// Горизонт `fill until`: смещение от старта родителя, минус — как у строк.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Until {
    pub negative: bool,
    pub duration: Duration,
}

impl Until {
    /// Сырой текст горизонта для сообщений E07: с минусом (`'-2h'`) или без.
    pub fn raw(&self) -> String {
        if self.negative {
            format!("-{}", self.duration.raw)
        } else {
            self.duration.raw.clone()
        }
    }
}
/// Вызов: `DEPOT.depart()` — действие точки (с опциональным блоком
/// `{k = v, ...}` — данные события), `CITY_ROUTE()` — вызов цикла
/// (с опциональными аргументами — значениями параметров).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    PointAction {
        point: String,
        action: String,
        block: Vec<(String, Expr)>,
    },
    CycleCall { name: String, args: Vec<Expr> },
}

/// Длительность сырым списком компонентов (`1h20m` → `[1h, 20m]`).
/// `raw` — точный срез исходника для сообщений `invalid duration '...'` (E05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duration {
    pub raw: String,
    pub items: Vec<DurationItem>,
}

/// Один компонент: число — сырыми цифрами (переполнение различит ядро, E05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationItem {
    pub number: String,
    pub unit: DurationUnit,
}

/// Единицы в порядке убывания из спеки: `w > d > h > m > s > ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationUnit {
    Week,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_route_matches_fixture() {
        let src = include_str!("../../../examples/valid/route.cyclo");
        let got = parse(src).expect("route.cyclo обязан разбираться");
        assert_eq!(got.schedule, route_ast());
    }

    #[test]
    fn parse_bitwise_precedence() {
        // `<<` сильнее `&`, `&` сильнее `^`, `^` сильнее `|`,
        // все слабее `+`, все сильнее сравнения.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [1 + 2 << 3 & 5 ^ 6 | 7 == 8] 0m: A.x(); } }";
        let got = parse(src).expect("битовое условие обязано разбираться");
        let cond = got
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        let bit = |op, l: Expr, r: Expr| Expr::Bit {
            op,
            left: Box::new(l),
            right: Box::new(r),
        };
        let num = |n: &str| Expr::Num(n.to_owned());
        let expected = Cond::Cmp {
            op: CmpOp::Eq,
            left: bit(
                BitOp::Or,
                bit(
                    BitOp::Xor,
                    bit(
                        BitOp::And,
                        bit(
                            BitOp::Shl,
                            Expr::Bin {
                                op: ArithOp::Add,
                                left: Box::new(num("1")),
                                right: Box::new(num("2")),
                            },
                            num("3"),
                        ),
                        num("5"),
                    ),
                    num("6"),
                ),
                num("7"),
            ),
            right: CondRhs::One(num("8")),
        };
        assert_eq!(cond, expected);
    }

    #[test]
    fn parse_shr_is_not_ge() {
        // `>>` — сдвиг, а не два сравнения: `8 >> 2 == 2` истинно.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [8 >> 2 == 2] 0m: A.x(); } }";
        let got = parse(src).expect("сдвиг вправо обязан разбираться");
        let cond = got
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        let expected = Cond::Cmp {
            op: CmpOp::Eq,
            left: Expr::Bit {
                op: BitOp::Shr,
                left: Box::new(Expr::Num("8".to_owned())),
                right: Box::new(Expr::Num("2".to_owned())),
            },
            right: CondRhs::One(Expr::Num("2".to_owned())),
        };
        assert_eq!(cond, expected);
    }

    #[test]
    fn parse_bool_group_is_transparent() {
        // Группа без операторов — тот же AST, что без скобок (узла нет).
        let plain = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [at == 1] 0m: A.x(); } }";
        let grouped = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [(at == 1)] 0m: A.x(); } }";
        let nested = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [((at == 1))] 0m: A.x(); } }";
        let cond_of = |src: &str| {
            parse(src)
                .expect("условие обязано разбираться")
                .schedule
                .root
                .stmts
                .into_iter()
                .next()
                .expect("строка есть")
                .condition
                .expect("условие есть")
        };
        assert_eq!(cond_of(grouped), cond_of(plain));
        assert_eq!(cond_of(nested), cond_of(plain));
    }

    #[test]
    fn parse_bool_group_overrides_precedence() {
        // `(a or b) and c`: группа связывает or раньше and.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [(at == 1 or at == 2) and at == 3] 0m: A.x(); } }";
        let cond = parse(src)
            .expect("группа обязана разбираться")
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        let cmp = |n: &str| Cond::Cmp {
            op: CmpOp::Eq,
            left: Expr::At,
            right: CondRhs::One(Expr::Num(n.to_owned())),
        };
        assert_eq!(
            cond,
            Cond::And(vec![Cond::Or(vec![cmp("1"), cmp("2")]), cmp("3")])
        );
    }

    #[test]
    fn parse_bool_group_under_not() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [not (at == 1 or at == 2)] 0m: A.x(); } }";
        let cond = parse(src)
            .expect("not с группой обязан разбираться")
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        let cmp = |n: &str| Cond::Cmp {
            op: CmpOp::Eq,
            left: Expr::At,
            right: CondRhs::One(Expr::Num(n.to_owned())),
        };
        assert_eq!(
            cond,
            Cond::Not(Box::new(Cond::Or(vec![cmp("1"), cmp("2")])))
        );
    }

    #[test]
    fn parse_alternation_stays_numeric() {
        // `(1 or 2)` в правой части сравнения — альтернация, а не группа.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [at == (1 or 2)] 0m: A.x(); } }";
        let cond = parse(src)
            .expect("альтернация обязана разбираться")
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        assert_eq!(
            cond,
            Cond::Cmp {
                op: CmpOp::Eq,
                left: Expr::At,
                right: CondRhs::Alt(vec![Expr::Num("1".to_owned()), Expr::Num("2".to_owned())]),
            }
        );
    }

    #[test]
    fn parse_truth_bridge_takes_full_condition() {
        // Мостик в арифметике: `2 * (or-условие)` — Truth держит Or целиком.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [2 * (at == 1 or at == 2) == 2] 0m: A.x(); } }";
        let cond = parse(src)
            .expect("мостик с or обязан разбираться")
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        let cmp = |n: &str| Cond::Cmp {
            op: CmpOp::Eq,
            left: Expr::At,
            right: CondRhs::One(Expr::Num(n.to_owned())),
        };
        assert_eq!(
            cond,
            Cond::Cmp {
                op: CmpOp::Eq,
                left: Expr::Bin {
                    op: ArithOp::Mul,
                    left: Box::new(Expr::Num("2".to_owned())),
                    right: Box::new(Expr::Truth(Box::new(Cond::Or(vec![cmp("1"), cmp("2")])))),
                },
                right: CondRhs::One(Expr::Num("2".to_owned())),
            }
        );
    }

    #[test]
    fn parses_json_literals_in_const() {
        // Мапы/массивы/bool — литералы; содержимое — только литералы.
        let src = "const M = {\"name\": \"БЖД\", \"n\": 1, \"ok\": true, \
            \"tags\": [\"a\", 2], \"meta\": {\"k\": false}, \"empty\": {}, \"arr\": []}; \
            schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0m: A.x(); } }";
        let f = parse(src).expect("мапы обязаны разбираться");
        let body = match &f.decls[..] {
            [Decl::Const { name, body }] => {
                assert_eq!(name, "M");
                body.clone()
            }
            d => panic!("ожидалась одна const, получено {d:?}"),
        };
        let str_ = |s: &str| Expr::Str(s.to_owned());
        let num = |s: &str| Expr::Num(s.to_owned());
        assert_eq!(
            body,
            Expr::Map(vec![
                ("name".to_owned(), str_("БЖД")),
                ("n".to_owned(), num("1")),
                ("ok".to_owned(), Expr::Bool(true)),
                (
                    "tags".to_owned(),
                    Expr::Array(vec![str_("a"), num("2")])
                ),
                (
                    "meta".to_owned(),
                    Expr::Map(vec![("k".to_owned(), Expr::Bool(false))])
                ),
                ("empty".to_owned(), Expr::Map(vec![])),
                ("arr".to_owned(), Expr::Array(vec![])),
            ])
        );
    }

    #[test]
    fn parses_field_and_index_chains() {
        // `.поле` и `[n]` — постфиксы, свёртка слева.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { [subj.meta.n == 1 and subj.tags[0] == \"a\"] 0m: A.x(); } }";
        let cond = parse(src)
            .expect("доступ обязан разбираться")
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        let subj = |f: &str| Expr::Field {
            base: Box::new(Expr::Name("subj".to_owned())),
            field: f.to_owned(),
        };
        assert_eq!(
            cond,
            Cond::And(vec![
                Cond::Cmp {
                    op: CmpOp::Eq,
                    left: Expr::Field {
                        base: Box::new(subj("meta")),
                        field: "n".to_owned(),
                    },
                    right: CondRhs::One(Expr::Num("1".to_owned())),
                },
                Cond::Cmp {
                    op: CmpOp::Eq,
                    left: Expr::Index {
                        base: Box::new(subj("tags")),
                        index: "0".to_owned(),
                    },
                    right: CondRhs::One(Expr::Str("a".to_owned())),
                },
            ])
        );
    }

    #[test]
    fn parses_point_attrs_forms() {
        // Без attrs — None; литерал — Map; ссылка — Name.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            point B { actions = [x]; attrs = {\"building\": \"Л\", \"floors\": 5}; } \
            point C { actions = [x]; attrs = CORPUS; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0m: A.x(); } }";
        let points = parse(src)
            .expect("точки с attrs обязаны разбираться")
            .schedule
            .points;
        assert_eq!(points[0].attrs, None);
        assert_eq!(
            points[1].attrs,
            Some(Expr::Map(vec![
                ("building".to_owned(), Expr::Str("Л".to_owned())),
                ("floors".to_owned(), Expr::Num("5".to_owned())),
            ]))
        );
        assert_eq!(points[2].attrs, Some(Expr::Name("CORPUS".to_owned())));
    }

    #[test]
    fn parses_cycle_params_args_and_blocks() {
        // Параметры, аргументы (мапа и имя), блоки (полный и пустой).
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle L(subj) duration = 1h { \
            0m: A.x() { subject = subj.name, event = \"start\" }; 45m: A.x() {}; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h \
            { 9h: L({\"name\": \"БЖД\"}); 13h: L(M); } }";
        let s = parse(src).expect("параметры обязаны разбираться").schedule;
        assert_eq!(s.cycles[0].params, vec!["subj".to_owned()]);
        let block = match &s.cycles[0].stmts[0].invocation {
            Invocation::PointAction { block, .. } => block.clone(),
            r => panic!("ожидалось действие точки, получено {r:?}"),
        };
        assert_eq!(
            block,
            vec![
                (
                    "subject".to_owned(),
                    Expr::Field {
                        base: Box::new(Expr::Name("subj".to_owned())),
                        field: "name".to_owned(),
                    }
                ),
                ("event".to_owned(), Expr::Str("start".to_owned())),
            ]
        );
        match &s.cycles[0].stmts[1].invocation {
            Invocation::PointAction { block, .. } => assert!(block.is_empty()),
            r => panic!("ожидалось действие точки, получено {r:?}"),
        }
        let args: Vec<Vec<Expr>> = s
            .root
            .stmts
            .iter()
            .map(|st| match &st.invocation {
                Invocation::CycleCall { args, .. } => args.clone(),
                r => panic!("ожидался вызов цикла, получено {r:?}"),
            })
            .collect();
        assert_eq!(
            args,
            vec![
                vec![Expr::Map(vec![(
                    "name".to_owned(),
                    Expr::Str("БЖД".to_owned())
                )])],
                vec![Expr::Name("M".to_owned())],
            ]
        );
    }

    #[test]
    fn parses_true_call_and_truex_name() {
        // `true(x)` — вызов как раньше; `truex` — имя; `true` — литерал.
        let call = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [true(at) == 1] 0m: A.x(); } }";
        let cond = parse(call)
            .expect("вызов true обязан разбираться")
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        assert_eq!(
            cond,
            Cond::Cmp {
                op: CmpOp::Eq,
                left: Expr::Call {
                    name: "true".to_owned(),
                    args: vec![Expr::At],
                },
                right: CondRhs::One(Expr::Num("1".to_owned())),
            }
        );
        let src = "const T = true; const U = truex; schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 0m: A.x(); } }";
        let f = parse(src).expect("true/truex обязаны разбираться");
        let bodies: Vec<Expr> = f
            .decls
            .iter()
            .map(|d| match d {
                Decl::Const { body, .. } => body.clone(),
                d => panic!("ожидалась const, получено {d:?}"),
            })
            .collect();
        assert_eq!(
            bodies,
            vec![Expr::Bool(true), Expr::Name("truex".to_owned())]
        );
    }

    #[test]
    fn rejects_spaced_index() {
        // Индекс атомарный: пробелы внутри — синтаксис.
        for row in [
            "subj.tags[ 0] == \"a\"",
            "subj.tags[0 ] == \"a\"",
            "subj.tags[- 1] == \"a\"",
        ] {
            let src = format!(
                "schedule \"T\" {{ point A {{ actions = [x]; }} \
                root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ [{row}] 0m: A.x(); }} }}"
            );
            assert!(parse(&src).is_err(), "для {row:?}");
        }
        // Слитный минус разбирается (границы — E12 в ядре, не синтаксис).
        let src = "schedule \"T\" { point A { actions = [x]; } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [subj.tags[-1] == \"a\"] 0m: A.x(); } }";
        let cond = parse(src)
            .expect("слитный минус обязан разбираться")
            .schedule
            .root
            .stmts
            .into_iter()
            .next()
            .expect("строка есть")
            .condition
            .expect("условие есть");
        match cond {
            Cond::Cmp { left, .. } => match left {
                Expr::Index { index, .. } => assert_eq!(index, "-1"),
                e => panic!("ожидался индекс, получено {e:?}"),
            },
            c => panic!("ожидалось сравнение, получено {c:?}"),
        }
    }

    #[test]
    fn parse_rejects_missing_root_cycle() {
        // bad_syntax.cyclo: нет root_cycle → ошибка парсера без E-кода.
        let src = include_str!("../../../examples/invalid/bad_syntax.cyclo");
        assert!(parse(src).is_err());
    }

    #[test]
    fn parse_rejects_old_trailing_comma() {
        // Ревизия спеки: висячая запятая перед `{` запрещена строго.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h, { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        assert!(parse(src).is_err());
    }

    #[test]
    fn parse_accepts_validation_fixtures() {
        // Граница парсер/ядро: файлы bad_e01–e09 синтаксически корректны,
        // их ошибки — валидация (E01–E09), а не синтаксис.
        for src in [
            include_str!("../../../examples/invalid/bad_e01.cyclo"),
            include_str!("../../../examples/invalid/bad_e02.cyclo"),
            include_str!("../../../examples/invalid/bad_e03.cyclo"),
            include_str!("../../../examples/invalid/bad_e04.cyclo"),
            include_str!("../../../examples/invalid/bad_e05.cyclo"),
            include_str!("../../../examples/invalid/bad_e06.cyclo"),
            include_str!("../../../examples/invalid/bad_e07.cyclo"),
            include_str!("../../../examples/invalid/bad_e08.cyclo"),
            include_str!("../../../examples/invalid/bad_e09.cyclo"),
            include_str!("../../../examples/invalid/bad_e07_neg.cyclo"),
            include_str!("../../../examples/invalid/bad_e10_zero.cyclo"),
            include_str!("../../../examples/invalid/bad_e10_fill0.cyclo"),
            include_str!("../../../examples/invalid/bad_e10_action.cyclo"),
            include_str!("../../../examples/invalid/bad_e07_chain.cyclo"),
            include_str!("../../../examples/invalid/bad_e07_until.cyclo"),
            include_str!("../../../examples/invalid/bad_e11.cyclo"),
            include_str!("../../../examples/invalid/bad_e12.cyclo"),
            include_str!("../../../examples/invalid/bad_e12_div.cyclo"),
        ] {
            parse(src).expect("bad_e*.cyclo обязан разбираться грамматикой");
        }
    }

    /// Ожидаемый AST примера из §1 спеки (`route.cyclo`).
    /// Следующий шаг: `parse()` обязан строить ровно это.
    fn route_ast() -> Schedule {
        let dur = |raw: &str, items: Vec<(&str, DurationUnit)>| Duration {
            raw: raw.to_owned(),
            items: items
                .into_iter()
                .map(|(n, u)| DurationItem {
                    number: n.to_owned(),
                    unit: u,
                })
                .collect(),
        };
        let point_call = |offset: Duration, point: &str, action: &str| Stmt {
            offset,
            negative: false,
            repeat: Repeat::Once,
            condition: None,
            invocation: Invocation::PointAction {
                point: point.to_owned(),
                action: action.to_owned(),
                block: Vec::new(),
            },
        };
        Schedule {
            name: "Автобусный парк".to_owned(),
            points: vec![
                Point {
                    name: "DEPOT".to_owned(),
                    actions: vec!["depart".to_owned(), "arrive".to_owned()],
                    attrs: None,
                },
                Point {
                    name: "AIRPORT".to_owned(),
                    actions: vec!["arrive".to_owned(), "depart".to_owned()],
                    attrs: None,
                },
            ],
            cycles: vec![
                Cycle {
                    name: "CITY_ROUTE".to_owned(),
                    params: Vec::new(),
                    duration: dur(
                        "1h20m",
                        vec![("1", DurationUnit::Hour), ("20", DurationUnit::Minute)],
                    ),
                    stmts: vec![
                        point_call(
                            dur("0m", vec![("0", DurationUnit::Minute)]),
                            "DEPOT",
                            "depart",
                        ),
                        point_call(
                            dur("40m", vec![("40", DurationUnit::Minute)]),
                            "AIRPORT",
                            "arrive",
                        ),
                        point_call(
                            dur("50m", vec![("50", DurationUnit::Minute)]),
                            "AIRPORT",
                            "depart",
                        ),
                        Stmt {
                            offset: dur("0m", vec![("0", DurationUnit::Minute)]),
                            negative: true,
                            repeat: Repeat::Once,
                            condition: None,
                            invocation: Invocation::PointAction {
                                point: "DEPOT".to_owned(),
                                action: "arrive".to_owned(),
                                block: Vec::new(),
                            },
                        },
                    ],
                },
                Cycle {
                    name: "SHUTTLE".to_owned(),
                    params: Vec::new(),
                    duration: dur("20m", vec![("20", DurationUnit::Minute)]),
                    stmts: vec![
                        point_call(
                            dur("0m", vec![("0", DurationUnit::Minute)]),
                            "DEPOT",
                            "depart",
                        ),
                        point_call(
                            dur("20m", vec![("20", DurationUnit::Minute)]),
                            "DEPOT",
                            "arrive",
                        ),
                    ],
                },
            ],
            root: RootCycle {
                start_time: "2026-01-01T00:00:00".to_owned(),
                duration: dur("24h", vec![("24", DurationUnit::Hour)]),
                stmts: vec![
                    Stmt {
                        offset: dur("6h", vec![("6", DurationUnit::Hour)]),
                        negative: false,
                        repeat: Repeat::Once,
                        condition: None,
                        invocation: Invocation::CycleCall {
                            name: "CITY_ROUTE".to_owned(),
                            args: Vec::new(),
                        },
                    },
                    Stmt {
                        offset: dur("10h", vec![("10", DurationUnit::Hour)]),
                        negative: false,
                        repeat: Repeat::Times("2".to_owned()),
                        condition: Some(Cond::And(vec![
                            Cond::Cmp {
                                op: CmpOp::Ge,
                                left: Expr::Call {
                                    name: "hour".to_owned(),
                                    args: vec![Expr::At],
                                },
                                right: CondRhs::One(Expr::Call {
                                    name: "rush_top".to_owned(),
                                    args: vec![Expr::Name("MORNING".to_owned())],
                                }),
                            },
                            Cond::Not(Box::new(Cond::Pred {
                                name: "weekend".to_owned(),
                                args: vec![Expr::At],
                            })),
                        ])),
                        invocation: Invocation::CycleCall {
                            name: "SHUTTLE".to_owned(),
                            args: Vec::new(),
                        },
                    },
                    Stmt {
                        offset: dur("14h", vec![("14", DurationUnit::Hour)]),
                        negative: false,
                        repeat: Repeat::Fill {
                            until: Some(Until {
                                negative: false,
                                duration: dur("15h", vec![("15", DurationUnit::Hour)]),
                            }),
                        },
                        condition: Some(Cond::Not(Box::new(Cond::Pred {
                            name: "weekend".to_owned(),
                            args: vec![Expr::At],
                        }))),
                        invocation: Invocation::CycleCall {
                            name: "SHUTTLE".to_owned(),
                            args: Vec::new(),
                        },
                    },
                    Stmt {
                        offset: dur("18h", vec![("18", DurationUnit::Hour)]),
                        negative: false,
                        repeat: Repeat::Once,
                        condition: Some(Cond::And(vec![
                            Cond::Pred {
                                name: "commute".to_owned(),
                                args: vec![Expr::At],
                            },
                            Cond::Not(Box::new(Cond::Pred {
                                name: "weekend".to_owned(),
                                args: vec![Expr::At],
                            })),
                        ])),
                        invocation: Invocation::CycleCall {
                            name: "CITY_ROUTE".to_owned(),
                            args: Vec::new(),
                        },
                    },
                    Stmt {
                        offset: dur("12h", vec![("12", DurationUnit::Hour)]),
                        negative: false,
                        repeat: Repeat::Once,
                        condition: Some(Cond::Pred {
                            name: "weekend".to_owned(),
                            args: vec![Expr::At],
                        }),
                        invocation: Invocation::CycleCall {
                            name: "CITY_ROUTE".to_owned(),
                            args: Vec::new(),
                        },
                    },
                ],
            },
        }
    }

    #[test]
    fn parses_negative_offsets() {
        // Минус — только у смещения строки; `-1h20m` — минус целиком, не покомпонентно.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h20m { 0m: A.x(); -10m: A.x(); -0m: A.x(); -1h20m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { -6h: R(); } }";
        let s = parse(src).expect("отрицательные смещения обязаны разбираться");
        let flags: Vec<bool> = s.schedule.cycles[0]
            .stmts
            .iter()
            .map(|st| st.negative)
            .collect();
        assert_eq!(flags, vec![false, true, true, true]);
        assert_eq!(s.schedule.cycles[0].stmts[1].offset.raw, "10m");
        assert_eq!(s.schedule.cycles[0].stmts[1].offset_raw(), "-10m");
        assert_eq!(s.schedule.cycles[0].stmts[0].offset_raw(), "0m");
        assert!(s.schedule.root.stmts[0].negative);
    }

    #[test]
    fn rejects_space_after_minus() {
        // Минус пишется слитно: `- 10m` — синтаксическая ошибка без E-кода.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { - 10m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        assert!(parse(src).is_err());
    }

    #[test]
    fn rejects_minus_in_duration_header() {
        // В заголовках (`duration = …`) длительности неотрицательны.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = -1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        assert!(parse(src).is_err());
    }

    #[test]
    fn parses_repeat_fill_until() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            6h: repeat 3 R(); 7h: fill R(); 8h: fill until 12h R(); 9h: fill until -2h R(); } }";
        let s = parse(src).expect("повторы обязаны разбираться");
        assert_eq!(
            s.schedule.root.stmts[0].repeat,
            Repeat::Times("3".to_owned())
        );
        assert_eq!(
            s.schedule.root.stmts[1].repeat,
            Repeat::Fill { until: None }
        );
        match &s.schedule.root.stmts[2].repeat {
            Repeat::Fill { until: Some(u) } => {
                assert!(!u.negative);
                assert_eq!(u.duration.raw, "12h");
                assert_eq!(u.raw(), "12h");
            }
            r => panic!("ожидался fill until, получено {r:?}"),
        }
        match &s.schedule.root.stmts[3].repeat {
            Repeat::Fill { until: Some(u) } => {
                assert!(u.negative);
                assert_eq!(u.raw(), "-2h");
            }
            r => panic!("ожидался fill until -2h, получено {r:?}"),
        }
        // `repeat 0` — уровень парсера пропускает (валидация ядра, E10).
        let src0 = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: repeat 0 R(); } }";
        let s0 = parse(src0).expect("repeat 0 синтаксически корректен");
        assert_eq!(
            s0.schedule.root.stmts[0].repeat,
            Repeat::Times("0".to_owned())
        );
    }

    #[test]
    fn rejects_space_after_until_minus() {
        // Минус в `until` слитно: `fill until - 2h` — синтаксическая ошибка.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: fill until - 2h R(); } }";
        assert!(parse(src).is_err());
    }

    #[test]
    fn cycle_named_fill_still_callable() {
        // Позиционное распознавание: голый вызов цикла `fill` работает.
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle fill duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: fill(); } }";
        let s = parse(src).expect("вызов цикла fill обязан разбираться");
        assert_eq!(s.schedule.root.stmts[0].repeat, Repeat::Once);
        assert!(matches!(
            s.schedule.root.stmts[0].invocation,
            Invocation::CycleCall { .. }
        ));
    }

    #[test]
    fn parses_conditions() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [at >= 1000 and at < 2000] 6h: R(); \
            [not at == 0 or str(at) == \"0\" ++ \"\"] 7h: R(); \
            [at == (1 or 2)] 8h: fill R(); } }";
        let s = parse(src).expect("условия обязаны разбираться");
        assert!(s.schedule.root.stmts[0].condition.is_some());
        assert!(s.schedule.root.stmts[1].condition.is_some());
        assert!(s.schedule.root.stmts[2].condition.is_some());
        assert!(matches!(
            s.schedule.root.stmts[0].condition,
            Some(Cond::And(_))
        ));
        assert!(matches!(
            s.schedule.root.stmts[1].condition,
            Some(Cond::Or(_))
        ));
        match &s.schedule.root.stmts[2].condition {
            Some(Cond::Cmp {
                right: CondRhs::Alt(alts),
                ..
            }) => {
                assert_eq!(alts.len(), 2);
            }
            c => panic!("ожидалась альтернация, получено {c:?}"),
        }
    }

    #[test]
    fn parses_unary_minus() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { [-at >= -1000] 6h: R(); } }";
        let s = parse(src).expect("унарный минус обязан разбираться");
        match &s.schedule.root.stmts[0].condition {
            Some(Cond::Cmp {
                left: Expr::Neg(_), ..
            }) => {}
            c => panic!("ожидался унарный минус слева, получено {c:?}"),
        }
    }

    #[test]
    fn rejects_bad_conditions() {
        // Голое число, цепочка сравнений, `and` в альтернации — синтаксис.
        for row in [
            "[5] 6h: R();",
            "[at < 1 < 2] 6h: R();",
            "[at == (1 and 2)] 6h: R();",
        ] {
            let src = format!(
                "schedule \"T\" {{ point A {{ actions = [x]; }} \
                cycle R duration = 1h {{ 0m: A.x(); }} \
                root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ {row} }} }}"
            );
            assert!(parse(&src).is_err(), "для {row}");
        }
    }

    #[test]
    fn parses_decls_pred_call_and_truth() {
        let src = "const K = 2; fun double(x) = x * K; pred big(at) = at >= K; \
            schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { \
            [big(at)] 6h: R(); [double(at) == 2 * (at >= K)] 7h: R(); } }";
        let s = parse(src).expect("объявления обязаны разбираться");
        assert_eq!(s.decls.len(), 3);
        assert!(matches!(s.decls[0], Decl::Const { .. }));
        assert!(matches!(s.decls[1], Decl::Fun { .. }));
        assert!(matches!(s.decls[2], Decl::Pred { .. }));
        assert!(matches!(
            s.schedule.root.stmts[0].condition,
            Some(Cond::Pred { .. })
        ));
    }

    #[test]
    fn rejects_pred_param_not_at() {
        let src = "pred p(t) = t == 1; schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        assert!(parse(src).is_err());
    }

    #[test]
    fn parses_uses_before_decls() {
        let src = "use \"a.cyclo\"; use \"b/c.cyclo\"; const K = 1; schedule \"T\" { \
            point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let s = parse(src).expect("use обязаны разбираться");
        assert_eq!(s.uses, vec!["a.cyclo".to_owned(), "b/c.cyclo".to_owned()]);
        assert_eq!(s.decls.len(), 1);
    }

    #[test]
    fn rejects_use_after_schedule() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } } \
            use \"a.cyclo\";";
        assert!(parse(src).is_err());
    }

    #[test]
    fn rejects_use_between_decls_and_schedule() {
        let src = "const K = 1; use \"a.cyclo\"; schedule \"T\" { \
            point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        assert!(parse(src).is_err());
    }

    #[test]
    fn parses_string_bodied_fun() {
        let decls = parse_decls("fun datestr(t) = pad(year(t), 4) ++ \"-\" ++ pad(month(t), 2);")
            .expect("склейка в теле обязана разбираться");
        assert!(matches!(
            decls[0],
            Decl::Fun {
                body: Expr::Concat(_),
                ..
            }
        ));
    }

    #[test]
    fn parses_unit_with_schedule_flag() {
        let u = parse_unit("use \"a.cyclo\"; const K = 1;")
            .expect("единица без расписания обязана разбираться");
        assert_eq!(u.uses, vec!["a.cyclo".to_owned()]);
        assert_eq!(u.decls.len(), 1);
        assert!(!u.has_schedule);
        let sched = "schedule \"T\" { point A { actions = [x]; } \
            cycle R duration = 1h { 0m: A.x(); } \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: R(); } }";
        let u = parse_unit(&format!("const K = 1; {sched}"))
            .expect("единица с расписанием разбирается");
        assert!(u.has_schedule);
        assert_eq!(u.decls.len(), 1);
    }

    #[test]
    fn parses_system_prelude() {
        let src = include_str!("../../cyclorithm-core/src/std.cyclo");
        let decls = parse_decls(src).expect("прелюдия обязана разбираться");
        assert!(decls.len() >= 20, "в прелюдии десятки объявлений");
        assert!(decls.iter().any(|d| matches!(
            d,
            Decl::Pred { name, .. } if name == "weekend"
        )));
    }

    #[test]
    fn ast_fixture_covers_spec_example() {
        let s = route_ast();
        assert_eq!(s.name, "Автобусный парк");
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.cycles.len(), 2);
        assert_eq!(s.cycles[0].stmts.len(), 4);
        assert!(s.cycles[0].stmts[3].negative);
        assert_eq!(s.cycles[1].stmts.len(), 2);
        assert_eq!(s.root.stmts.len(), 5);
    }
}
