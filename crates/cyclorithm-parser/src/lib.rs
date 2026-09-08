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
    Point { name, actions }
}

fn build_cycle(pair: Pair<Rule>) -> Result<Cycle, pest::error::Error<Rule>> {
    let mut inner = pair.into_inner();
    let name = inner.next().expect("cycle: имя").as_str().to_owned();
    let duration = build_duration(inner.next().expect("cycle: duration"));
    let stmts = inner.map(build_stmt).collect::<Result<_, _>>()?;
    Ok(Cycle {
        name,
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
            Invocation::PointAction {
                point: parts.next().expect("вызов: точка").as_str().to_owned(),
                action: parts.next().expect("вызов: действие").as_str().to_owned(),
            }
        }
        Rule::cycle_call => Invocation::CycleCall {
            name: call
                .into_inner()
                .next()
                .expect("вызов: цикл")
                .as_str()
                .to_owned(),
        },
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
        let mut values = vec![build_arith(alts.next().expect("alternation: ветка"))];
        while alts.next().is_some() {
            values.push(build_arith(alts.next().expect("alternation: ветка")));
        }
        CondRhs::Alt(values)
    } else {
        CondRhs::One(build_operand(right))
    };
    Cond::Cmp { op, left, right }
}

/// Операнд сравнения: склейка или арифметика. Обёртки (`cmp_side`,
/// `cmp_right`, `cond_arg`) снимает вызывающий.
fn build_operand(pair: Pair<Rule>) -> Expr {
    match pair.as_rule() {
        Rule::concat => Expr::Concat(pair.into_inner().map(build_concat_term).collect()),
        Rule::arith => build_arith(pair),
        r => unreachable!("операнд: неожиданное правило {r:?}"),
    }
}

fn build_concat_term(pair: Pair<Rule>) -> Expr {
    debug_assert_eq!(pair.as_rule(), Rule::concat_term);
    build_value(pair.into_inner().next().expect("concat_term: значение"))
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
    let expr = build_value(value);
    if negated {
        Expr::Neg(Box::new(expr))
    } else {
        expr
    }
}

/// Лист выражения: число, строка, вызов, имя (`at` — значение, остальное
/// проверит ядро) или скобки.
fn build_value(pair: Pair<Rule>) -> Expr {
    match pair.as_rule() {
        Rule::number => Expr::Num(pair.as_str().to_owned()),
        Rule::string => {
            let s = pair.as_str();
            Expr::Str(s[1..s.len() - 1].to_owned())
        }
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
        Rule::arith => build_arith(pair),
        Rule::concat => Expr::Concat(pair.into_inner().map(build_concat_term).collect()),
        Rule::truth => {
            let cmp = pair.into_inner().next().expect("truth: сравнение");
            Expr::Truth(Box::new(build_comparison(cmp)))
        }
        r => unreachable!("значение: неожиданное правило {r:?}"),
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

/// `point DEPOT { actions = [depart, arrive]; }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub name: String,
    pub actions: Vec<String>,
}

/// `cycle CITY_ROUTE duration = 1h20m { ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    pub name: String,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Num(String),
    Str(String),
    At,
    Name(String),
    Neg(Box<Expr>),
    Bin {
        op: ArithOp,
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
/// Вызов: `DEPOT.depart()` — действие точки, `CITY_ROUTE()` — вызов цикла.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    PointAction { point: String, action: String },
    CycleCall { name: String },
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
            },
        };
        Schedule {
            name: "Автобусный парк".to_owned(),
            points: vec![
                Point {
                    name: "DEPOT".to_owned(),
                    actions: vec!["depart".to_owned(), "arrive".to_owned()],
                },
                Point {
                    name: "AIRPORT".to_owned(),
                    actions: vec!["arrive".to_owned(), "depart".to_owned()],
                },
            ],
            cycles: vec![
                Cycle {
                    name: "CITY_ROUTE".to_owned(),
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
                            },
                        },
                    ],
                },
                Cycle {
                    name: "SHUTTLE".to_owned(),
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
