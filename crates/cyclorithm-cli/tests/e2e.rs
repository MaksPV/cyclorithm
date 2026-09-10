//! Сквозные тесты контракта CLI (§1 спеки): JSON в stdout, ошибки в stderr.

use std::process::{Command, Output, Stdio};

fn cyclo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cyclo"))
}

fn run(args: &[&str]) -> Output {
    cyclo()
        .args(args)
        .output()
        .expect("бинарь cyclo обязан запускаться")
}

/// Прогон со stdin вместо файла (`-`): вход подаётся в поток.
fn run_stdin(args: &[&str], input: &str) -> Output {
    use std::io::Write as _;
    let mut child = cyclo()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("бинарь cyclo обязан запускаться");
    child
        .stdin
        .take()
        .expect("stdin обязан открыться")
        .write_all(input.as_bytes())
        .expect("запись в stdin обязана удаваться");
    child
        .wait_with_output()
        .expect("ожидание обязано удаваться")
}

fn stdout_json(out: &Output) -> serde_json::Value {
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout.clone()).expect("stdout — UTF-8");
    serde_json::from_str(&text).expect("stdout — один JSON-объект")
}

#[test]
fn route_matches_expected_json() {
    // Пятница 09.01 — полное расписание (будние ветки), 18 событий.
    let out = run(&[
        "run",
        "../../examples/valid/route.cyclo",
        "--start",
        "2026-01-09T00:00:00",
        "--end",
        "2026-01-10T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/route.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn neg_offsets_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/neg_offsets.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/neg_offsets.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn bitwise_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/bitwise.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-01T01:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/bitwise.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn bool_groups_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/bool_groups.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/bool_groups.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn rand_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/rand.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-01T01:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/rand.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn mkdate_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/mkdate.cyclo",
        "--start",
        "2026-09-01T00:00:00",
        "--end",
        "2026-10-01T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/mkdate.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn repeat_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/repeat.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/repeat.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn conditions_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/conditions.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/conditions.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn attrs_matches_expected_json() {
    // Словари точек и блоков: 9 событий, tick только у лекции.
    let out = run(&[
        "run",
        "../../examples/valid/attrs.cyclo",
        "--start",
        "2026-09-07T00:00:00",
        "--end",
        "2026-09-08T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/attrs.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn routines_matches_expected_json() {
    // Рутины по таблице: понедельник — пара 9:00 и обед 12:00,
    // суббота — тишина (пустая рутина, обед только по будням).
    let out = run(&[
        "run",
        "../../examples/valid/routines.cyclo",
        "--start",
        "2026-09-07T00:00:00",
        "--end",
        "2026-09-14T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/routines.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn imports_matches_expected_json() {
    // 1 января — праздник из holidays.cyclo: рейс в 10:00 есть, в 12:00 нет.
    let out = run(&[
        "run",
        "../../examples/valid/imports.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/imports.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn empty_window_gives_empty_events() {
    let out = run(&[
        "run",
        "../../examples/valid/route.cyclo",
        "--start",
        "2026-01-11T00:00:00",
        "--end",
        "2026-01-10T00:00:00",
    ]);
    let got = stdout_json(&out);
    assert_eq!(got["events"], serde_json::Value::Array(vec![]));
}

#[test]
fn validation_errors_go_to_stderr() {
    // (файл, фрагмент stderr). В stdout при ошибке — ничего.
    for (file, message) in [
        ("bad_e01", "unknown point 'PORT'"),
        ("bad_e02", "action 'arrive' not allowed for point 'DEPOT'"),
        ("bad_e03", "unknown cycle 'NIGHT_ROUTE'"),
        ("bad_e04", "duplicate point 'DEPOT'"),
        ("bad_e05", "invalid duration '1h2h'"),
        ("bad_e06", "recursive cycle 'A'"),
        (
            "bad_e07",
            "cycle 'CYCLE2' overruns 'CYCLE1' by 20m (80m > 60m)",
        ),
        ("bad_e07_neg", "offset '-2h' out of bounds (duration 1h20m)"),
        (
            "bad_e07_chain",
            "cycle 'R' overruns 'root_cycle' by 100m (1540m > 1440m)",
        ),
        ("bad_e07_until", "until '30h' out of bounds (duration 24h)"),
        ("bad_e10_zero", "invalid repeat count '0'"),
        ("bad_e10_fill0", "fill of zero-duration cycle 'EMPTY'"),
        (
            "bad_e10_action",
            "repeat of point action 'depart' not allowed",
        ),
        ("bad_e11", "unknown name 'banana'"),
        ("bad_e11_private", "unknown name '__z'"),
        ("bad_e12_recursive", "recursive definition 'a'"),
        ("bad_e12_datestr", "invalid date '2026-13-01'"),
        ("bad_e13_cycle", "import cycle 'bad_e13_cycle_a.cyclo'"),
        ("bad_e13_missing", "cannot read import 'no_such_lib.cyclo'"),
        (
            "bad_e14_schedule",
            "schedule not allowed in import 'bad_e14_lib.cyclo'",
        ),
        ("bad_e04_dup", "duplicate const 'K'"),
        ("bad_e12", "type mismatch: cannot mix number and string"),
        ("bad_e12_div", "division by zero"),
        ("bad_e08", "invalid datetime 'not-a-datetime'"),
        ("bad_e09", "point 'DEPOT' is not a cycle"),
        ("bad_e15", "duplicate attribute 'a'"),
        ("bad_e16", "unknown table 'SHORT'"),
        ("bad_e16_slot", "unknown slot '8th'"),
    ] {
        let path = format!("../../examples/invalid/{file}.cyclo");
        let out = run(&[
            "run",
            &path,
            "--start",
            "2026-01-10T00:00:00",
            "--end",
            "2026-01-11T00:00:00",
        ]);
        assert!(!out.status.success(), "для {file}");
        assert!(out.stdout.is_empty(), "для {file}: в stdout ничего");
        let err = String::from_utf8(out.stderr.clone()).expect("stderr — UTF-8");
        assert!(
            err.contains(message),
            "для {file}: нет {message:?} в {err:?}"
        );
    }
}

#[test]
fn syntax_error_has_no_e_code() {
    let out = run(&[
        "run",
        "../../examples/invalid/bad_syntax.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn bad_cli_args_give_usage() {
    for args in [
        vec!["run"],
        vec!["run", "../../examples/valid/route.cyclo"],
        vec![
            "run",
            "../../examples/valid/route.cyclo",
            "--start",
            "2026-01-10T00:00:00",
        ],
        vec![
            "run",
            "../../examples/valid/route.cyclo",
            "--start",
            "not-a-datetime",
            "--end",
            "2026-01-11T00:00:00",
        ],
        vec!["next", "../../examples/real/cron.cyclo", "--within", "1x"],
        vec!["next", "../../examples/real/cron.cyclo", "-n", "много"],
        vec!["check"],
        vec!["bogus"],
    ] {
        let out = run(&args);
        assert!(!out.status.success(), "для {args:?}");
        assert!(out.stdout.is_empty(), "для {args:?}");
        assert!(!out.stderr.is_empty(), "для {args:?}");
    }
}

#[test]
fn help_and_version() {
    // Без аргументов и --help — справка в stdout, код 0.
    for args in [vec![], vec!["--help"]] {
        let out = run(&args);
        assert!(out.status.success(), "для {args:?}");
        let text = String::from_utf8(out.stdout.clone()).expect("stdout — UTF-8");
        assert!(text.starts_with("usage:"), "для {args:?}");
    }
    let out = run(&["--version"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout.clone()).expect("stdout — UTF-8");
    assert!(text.starts_with("cyclo "), "версия: {text:?}");
}

#[test]
fn check_command() {
    let out = run(&["check", "../../examples/valid/route.cyclo"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).expect("stdout — UTF-8"),
        "ok\n"
    );
    let out = run(&["check", "../../examples/invalid/bad_e16.cyclo"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let err = String::from_utf8(out.stderr).expect("stderr — UTF-8");
    assert!(err.contains("unknown table 'SHORT'"), "{err:?}");
}

#[test]
fn short_dates_and_deltas() {
    // --start датой, --end дельтой от старта: те же 18 событий, что в контракте.
    let out = run(&[
        "run",
        "../../examples/valid/route.cyclo",
        "--start",
        "2026-01-09",
        "--end",
        "+1d",
    ]);
    let got = stdout_json(&out);
    assert_eq!(got["events"].as_array().expect("массив").len(), 18);
    assert_eq!(got["events"][0]["time"], "2026-01-09T06:00:00");
}

#[test]
fn stdin_source_matches_file() {
    // `-` читает программу из stdin (без импортов — не зависит от cwd).
    let args = [
        "run",
        "-",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ];
    let from_file = run(&[
        "run",
        "../../examples/valid/neg_offsets.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    assert!(from_file.status.success());
    let input = include_str!("../../../examples/valid/neg_offsets.cyclo");
    let from_stdin = run_stdin(&args, input);
    assert!(from_stdin.status.success());
    assert_eq!(from_stdin.stdout, from_file.stdout);
    // check и next тоже едят stdin.
    let out = run_stdin(&["check", "-"], input);
    assert!(out.status.success());
    let out = run_stdin(
        &["next", "-", "--from", "2026-01-10T00:00:00", "-n", "2"],
        input,
    );
    assert!(out.status.success());
    let got: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout — один JSON-объект");
    assert_eq!(got["events"].as_array().expect("массив").len(), 2);
}

#[test]
fn next_command() {
    let cron = "../../examples/real/cron.cyclo";
    // Первые 3 события понедельника — часовые пинги с полуночи.
    let out = run(&["next", cron, "--from", "2026-09-07T00:00:00", "-n", "3"]);
    let got = stdout_json(&out);
    assert_eq!(got["from"], "2026-09-07T00:00:00");
    let times: Vec<&str> = got["events"]
        .as_array()
        .expect("массив")
        .iter()
        .map(|e| e["time"].as_str().expect("строка"))
        .collect();
    assert_eq!(
        times,
        vec![
            "2026-09-07T00:00:00",
            "2026-09-07T01:00:00",
            "2026-09-07T02:00:00"
        ]
    );
    // n считает события: отчёт 09:30 + чистка 10:00.
    let out = run(&["next", cron, "--from", "2026-09-07T09:30:00", "-n", "2"]);
    let got = stdout_json(&out);
    let cmds: Vec<&str> = got["events"]
        .as_array()
        .expect("массив")
        .iter()
        .map(|e| e["action_attrs"]["cmd"].as_str().expect("строка"))
        .collect();
    assert_eq!(cmds, vec!["/opt/jobs/report.py", "/opt/jobs/cleanup.sh"]);
    // Нулевой горизонт — пусто без ошибки.
    let out = run(&[
        "next",
        cron,
        "--from",
        "2026-09-07T00:00:00",
        "--within",
        "0",
    ]);
    let got = stdout_json(&out);
    assert_eq!(got["events"].as_array().expect("массив").len(), 0);
    // --ndjson — по событию на строку.
    let out = run(&[
        "next",
        cron,
        "--from",
        "2026-09-07T00:00:00",
        "-n",
        "2",
        "--ndjson",
    ]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).expect("stdout — UTF-8");
    assert_eq!(text.lines().count(), 2);
    let first: serde_json::Value =
        serde_json::from_str(text.lines().next().expect("строка")).expect("JSON");
    assert_eq!(first["time"], "2026-09-07T00:00:00");
}
