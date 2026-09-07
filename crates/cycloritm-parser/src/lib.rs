//! Grammar and AST for the Cycloritm DSL.

use pest::iterators::Pair;
use pest::Parser as _;
use pest_derive::Parser;

/// Парсер грамматики из §3 спеки (см. `grammar.pest`).
#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct CycloParser;

/// Разбор исходника в AST. Ошибка — синтаксическая, без E-кода
/// (коды E01–E09 — только валидация уже разобранного AST в ядре).
pub fn parse(src: &str) -> Result<Schedule, pest::error::Error<Rule>> {
    let file = CycloParser::parse(Rule::file, src)?
        .next()
        .expect("file непуст");
    debug_assert_eq!(file.as_rule(), Rule::file);
    let schedule = file
        .into_inner()
        .next()
        .expect("file содержит ровно schedule");
    build_schedule(schedule)
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
    let first = inner.next().expect("stmt: смещение или минус");
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
        invocation,
    })
}

/// Модификатор повтора (§3 спеки): `repeat N` / `fill` / `fill until [−]T`.
/// `repeat 0` здесь принимается (валидация ядра, E10); минус в `until`
/// обязан быть слитным — проверка по спанам, как у смещения строки.
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

/// Корень файла: `schedule "имя" { point* cycle* root_cycle }`.
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

/// Одна строка цикла: `[<минус>] <смещение>: [<повтор>] <вызов>;`.
/// `negative` — минус из §3 спеки (только у смещения строки, слитно);
/// разрешается ядром как `duration(родителя) − смещение`.
/// `repeat` — модификатор повторов (§3–§4 спеки), по умолчанию `Once`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub offset: Duration,
    pub negative: bool,
    pub repeat: Repeat,
    pub invocation: Invocation,
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

/// Модификатор повторов строки (§3 спеки).
/// `Times` хранит число сырым текстом: в `u64` переводит ядро
/// (невлезающее — E10 `invalid repeat count`).
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
        let src = include_str!("../../../examples/route.cyclo");
        let got = parse(src).expect("route.cyclo обязан разбираться");
        assert_eq!(got, route_ast());
    }

    #[test]
    fn parse_rejects_missing_root_cycle() {
        // bad_syntax.cyclo: нет root_cycle → ошибка парсера без E-кода.
        let src = include_str!("../../../examples/bad_syntax.cyclo");
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
            include_str!("../../../examples/bad_e01.cyclo"),
            include_str!("../../../examples/bad_e02.cyclo"),
            include_str!("../../../examples/bad_e03.cyclo"),
            include_str!("../../../examples/bad_e04.cyclo"),
            include_str!("../../../examples/bad_e05.cyclo"),
            include_str!("../../../examples/bad_e06.cyclo"),
            include_str!("../../../examples/bad_e07.cyclo"),
            include_str!("../../../examples/bad_e08.cyclo"),
            include_str!("../../../examples/bad_e09.cyclo"),
            include_str!("../../../examples/bad_e07_neg.cyclo"),
            include_str!("../../../examples/bad_e10_zero.cyclo"),
            include_str!("../../../examples/bad_e10_fill0.cyclo"),
            include_str!("../../../examples/bad_e10_action.cyclo"),
            include_str!("../../../examples/bad_e07_chain.cyclo"),
            include_str!("../../../examples/bad_e07_until.cyclo"),
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
            cycles: vec![Cycle {
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
                    point_call(
                        dur("80m", vec![("80", DurationUnit::Minute)]),
                        "DEPOT",
                        "arrive",
                    ),
                ],
            }],
            root: RootCycle {
                start_time: "2026-01-01T00:00:00".to_owned(),
                duration: dur("24h", vec![("24", DurationUnit::Hour)]),
                stmts: vec![
                    Stmt {
                        offset: dur("6h", vec![("6", DurationUnit::Hour)]),
                        negative: false,
                        repeat: Repeat::Once,
                        invocation: Invocation::CycleCall {
                            name: "CITY_ROUTE".to_owned(),
                        },
                    },
                    Stmt {
                        offset: dur("18h", vec![("18", DurationUnit::Hour)]),
                        negative: false,
                        repeat: Repeat::Once,
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
        let flags: Vec<bool> = s.cycles[0].stmts.iter().map(|st| st.negative).collect();
        assert_eq!(flags, vec![false, true, true, true]);
        assert_eq!(s.cycles[0].stmts[1].offset.raw, "10m");
        assert_eq!(s.cycles[0].stmts[1].offset_raw(), "-10m");
        assert_eq!(s.cycles[0].stmts[0].offset_raw(), "0m");
        assert!(s.root.stmts[0].negative);
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
        assert_eq!(s.root.stmts[0].repeat, Repeat::Times("3".to_owned()));
        assert_eq!(s.root.stmts[1].repeat, Repeat::Fill { until: None });
        match &s.root.stmts[2].repeat {
            Repeat::Fill { until: Some(u) } => {
                assert!(!u.negative);
                assert_eq!(u.duration.raw, "12h");
                assert_eq!(u.raw(), "12h");
            }
            r => panic!("ожидался fill until, получено {r:?}"),
        }
        match &s.root.stmts[3].repeat {
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
        assert_eq!(s0.root.stmts[0].repeat, Repeat::Times("0".to_owned()));
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
        assert_eq!(s.root.stmts[0].repeat, Repeat::Once);
        assert!(matches!(
            s.root.stmts[0].invocation,
            Invocation::CycleCall { .. }
        ));
    }

    #[test]
    fn ast_fixture_covers_spec_example() {
        let s = route_ast();
        assert_eq!(s.name, "Автобусный парк");
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.cycles.len(), 1);
        assert_eq!(s.cycles[0].stmts.len(), 4);
        assert_eq!(s.root.stmts.len(), 2);
    }
}
