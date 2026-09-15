//! Бинарь `cyclo` — тонкая обёртка над парсером и ядром (контракт — см. docs/reference/cli.md).
//!
//! - `cyclo run FILE|- --start T --end T [--ndjson]` → окно в stdout, код 0;
//! - `cyclo next FILE|- [--from T] [--within D] [-n K] [--ndjson]` →
//!   первые `K` событий от `from`, код 0;
//! - `cyclo check FILE|-` → `ok`, код 0;
//! - `cyclo --version`, `cyclo --help` (и без аргументов) → код 0;
//! - ошибка ввода/валидации → текст в stderr, в stdout ничего, код 1;
//! - неверные аргументы → usage в stderr, код 2.
//!
//! Источник `-` — stdin до конца; `use` тогда резолвится от cwd.
//! Даты CLI — короткие формы (см. docs/reference/cli.md): `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM`,
//! `+DURATION` (для `--end` — от `--start`, иначе — от now).

use cyclorithm_core::datetime::{format_datetime_tz, parse_cli_datetime_zoned, parse_cli_duration};
use cyclorithm_core::pipeline::{
    PipelineError, check_source, event_to_json_zoned, expand_window, next_window,
};

/// Дефолтный горизонт `next`: 366 дней в мс.
const DEFAULT_WITHIN_MS: i64 = 366 * 86_400_000;

const HELP: &str = "\
usage:
  cyclo run FILE|- --start DATETIME --end DATETIME [--ndjson]
  cyclo next FILE|- [--from DATETIME] [--within DURATION] [-n K] [--ndjson]
  cyclo check FILE|-
  cyclo --version
  cyclo --help

DATETIME: 2026-09-07T09:30:00[.mmm], короче — 2026-09-07 (=T00:00:00)
или 2026-09-07T09:30 (=:00); +1d/+2h30m — дельта (для --end от --start,
иначе от текущего момента). DURATION: 7d, 2h30m, 1w2d3h4m5s6ms;
голое число — миллисекунды. --from по умолчанию — now, --within —
366d, -n — 1 (считает события). --ndjson — по событию на строку.
Источник - — stdin, use тогда от cwd. Коды: 0 успех, 1 ввод/валидация,
2 неверные аргументы.";

/// Источник программы: файл или stdin (`-`).
enum Src {
    File(String),
    Stdin,
}

enum Cmd {
    Run {
        src: Src,
        start_raw: String,
        end_raw: String,
        ndjson: bool,
    },
    Next {
        src: Src,
        from_raw: Option<String>,
        within_raw: Option<String>,
        n_raw: Option<String>,
        ndjson: bool,
    },
    Check {
        src: Src,
    },
    Version,
    Help,
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match parse_cli(&args) {
        Ok(cmd) => cmd,
        Err(usage) => {
            eprintln!("{usage}");
            return 2;
        }
    };
    match cmd {
        Cmd::Version => {
            println!("cyclo {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Cmd::Help => {
            println!("{HELP}");
            0
        }
        Cmd::Run {
            src,
            start_raw,
            end_raw,
            ndjson,
        } => cmd_run(src, &start_raw, &end_raw, ndjson),
        Cmd::Next {
            src,
            from_raw,
            within_raw,
            n_raw,
            ndjson,
        } => cmd_next(
            src,
            from_raw.as_deref(),
            within_raw.as_deref(),
            n_raw.as_deref(),
            ndjson,
        ),
        Cmd::Check { src } => cmd_check(src),
    }
}

/// Разбор аргументов; ошибка — короткий usage (код 2 у вызывателя).
fn parse_cli(args: &[String]) -> Result<Cmd, &'static str> {
    if args.is_empty() {
        return Ok(Cmd::Help);
    }
    match args.first().map(String::as_str) {
        Some("--version") if args.len() == 1 => return Ok(Cmd::Version),
        Some("--help") | Some("help") if args.len() == 1 => return Ok(Cmd::Help),
        _ => {}
    }
    let sub = args.first().map(String::as_str).ok_or(HELP)?;
    let src = |i: usize| {
        args.get(i).ok_or(HELP).map(|s| match s.as_str() {
            "-" => Src::Stdin,
            _ => Src::File(s.clone()),
        })
    };
    match sub {
        "run" => {
            let src = src(1)?;
            let (start, end, ndjson) = flags_run(&args[2..])?;
            Ok(Cmd::Run {
                src,
                start_raw: start.ok_or(HELP)?,
                end_raw: end.ok_or(HELP)?,
                ndjson,
            })
        }
        "next" => {
            let src = src(1)?;
            let flags = flags_next(&args[2..])?;
            Ok(Cmd::Next {
                src,
                from_raw: flags.from,
                within_raw: flags.within,
                n_raw: flags.n,
                ndjson: flags.ndjson,
            })
        }
        "check" if args.len() == 2 => Ok(Cmd::Check { src: src(1)? }),
        _ => Err(HELP),
    }
}

/// Флаги `run`: `--start T --end T [--ndjson]`; значения обязательны.
fn flags_run(rest: &[String]) -> Result<(Option<String>, Option<String>, bool), &'static str> {
    let mut start = None;
    let mut end = None;
    let mut ndjson = false;
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--ndjson" => ndjson = true,
            "--start" => start = Some(it.next().ok_or(HELP)?.clone()),
            "--end" => end = Some(it.next().ok_or(HELP)?.clone()),
            _ => return Err(HELP),
        }
    }
    Ok((start, end, ndjson))
}

/// Флаги `next`: всё опционально.
/// Флаги `next`: всё опционально.
struct NextFlags {
    from: Option<String>,
    within: Option<String>,
    n: Option<String>,
    ndjson: bool,
}

fn flags_next(rest: &[String]) -> Result<NextFlags, &'static str> {
    let mut flags = NextFlags {
        from: None,
        within: None,
        n: None,
        ndjson: false,
    };
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--ndjson" => flags.ndjson = true,
            "--from" => flags.from = Some(it.next().ok_or(HELP)?.clone()),
            "--within" => flags.within = Some(it.next().ok_or(HELP)?.clone()),
            "-n" => flags.n = Some(it.next().ok_or(HELP)?.clone()),
            _ => return Err(HELP),
        }
    }
    Ok(flags)
}

/// Текущий момент (UTC, наивный): мс epoch.
fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// Текст программы и база `use`: файл (база — его директория)
/// или stdin (база — cwd).
fn read_source(src: &Src) -> Result<(String, std::path::PathBuf), i32> {
    match src {
        Src::File(file) => match std::fs::read_to_string(file) {
            Ok(text) => {
                let base = std::path::Path::new(file)
                    .parent()
                    .unwrap_or(std::path::Path::new(""))
                    .to_path_buf();
                Ok((text, base))
            }
            Err(e) => {
                eprintln!("cannot read '{file}': {e}");
                Err(1)
            }
        },
        Src::Stdin => {
            use std::io::Read as _;
            let mut text = String::new();
            match std::io::stdin().lock().read_to_string(&mut text) {
                Ok(_) => Ok((text, std::path::PathBuf::new())),
                Err(e) => {
                    eprintln!("cannot read stdin: {e}");
                    Err(1)
                }
            }
        }
    }
}

/// Ошибка конвейера в stderr: парсер — текст pest как есть, без слага
/// (глава ошибок); валидация — `слаг: сообщение`.
fn print_error(e: &PipelineError) {
    match e {
        PipelineError::Syntax(text) => eprintln!("{text}"),
        PipelineError::Core(_) => eprintln!("{e}"),
    }
}

fn cmd_check(src: Src) -> i32 {
    let (text, base) = match read_source(&src) {
        Ok(v) => v,
        Err(code) => return code,
    };
    match check_source(&text, &base) {
        Ok(()) => {
            println!("ok");
            0
        }
        Err(e) => {
            print_error(&e);
            1
        }
    }
}

fn cmd_run(src: Src, start_raw: &str, end_raw: &str, ndjson: bool) -> i32 {
    let (text, base) = match read_source(&src) {
        Ok(v) => v,
        Err(code) => return code,
    };
    // `--start`/`--end`: короткие формы (см. docs/reference/cli.md), якорь дельты `--end` — старт;
    // битые значения — invalid-datetime; в объекте — эхо как передали.
    // Зона окна — офсет `start` (aware → с суффиксом, наивное → без).
    let now = now_ms();
    let (start_ms, zone) = match parse_cli_datetime_zoned(start_raw, now) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let (end_ms, _) = match parse_cli_datetime_zoned(end_raw, start_ms) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let window = match expand_window(&text, &base, start_ms, end_ms, zone) {
        Ok(window) => window,
        Err(e) => {
            print_error(&e);
            return 1;
        }
    };
    let effective = zone.or(window.file_zone);
    if ndjson {
        for e in &window.events {
            println!("{}", event_to_json_zoned(e, effective));
        }
        return 0;
    }
    let out = serde_json::json!({
        "schedule": window.schedule,
        "start": start_raw,
        "end": end_raw,
        "events": window.events.iter().map(|e| event_to_json_zoned(e, effective)).collect::<Vec<_>>(),
    });
    println!("{out}");
    0
}

fn cmd_next(
    src: Src,
    from_raw: Option<&str>,
    within_raw: Option<&str>,
    n_raw: Option<&str>,
    ndjson: bool,
) -> i32 {
    let (text, base) = match read_source(&src) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let now = now_ms();
    // `--from` по умолчанию — now (UTC, зона None); битый — invalid-datetime.
    // Зона окна — офсет `from` (aware → с суффиксом).
    let (from_ms, from_zone) = match from_raw {
        None => (now, None),
        Some(raw) => match parse_cli_datetime_zoned(raw, now) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        },
    };
    // `--within` по умолчанию — 366d; битый — неверные аргументы (код 2).
    let within_ms = match within_raw {
        None => DEFAULT_WITHIN_MS,
        Some(raw) => match parse_cli_duration(raw.strip_prefix('+').unwrap_or(raw)) {
            Some(v) => v,
            None => {
                eprintln!("{HELP}");
                return 2;
            }
        },
    };
    let n = match n_raw {
        None => 1,
        Some(raw) => match raw.parse::<usize>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("{HELP}");
                return 2;
            }
        },
    };
    let window = match next_window(&text, &base, from_ms, within_ms, n, from_zone) {
        Ok(window) => window,
        Err(e) => {
            print_error(&e);
            return 1;
        }
    };
    let effective = from_zone.or(window.file_zone);
    if ndjson {
        for e in &window.events {
            println!("{}", event_to_json_zoned(e, effective));
        }
        return 0;
    }
    let out = serde_json::json!({
        "schedule": window.schedule,
        "from": format_datetime_tz(from_ms, effective),
        "within": within_ms,
        "events": window.events.iter().map(|e| event_to_json_zoned(e, effective)).collect::<Vec<_>>(),
    });
    println!("{out}");
    0
}
