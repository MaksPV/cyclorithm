//! Сквозные тесты контракта CLI (§1 спеки): JSON в stdout, ошибки в stderr.

use std::process::{Command, Output};

fn cyclo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cyclo"))
}

fn run(args: &[&str]) -> Output {
    cyclo()
        .args(args)
        .output()
        .expect("бинарь cyclo обязан запускаться")
}

fn stdout_json(out: &Output) -> serde_json::Value {
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout.clone()).expect("stdout — UTF-8");
    serde_json::from_str(&text).expect("stdout — один JSON-объект")
}

#[test]
fn route_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/route.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/route.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn neg_offsets_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/neg_offsets.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/neg_offsets.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn repeat_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/repeat.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/repeat.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn conditions_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/conditions.cyclo",
        "--start",
        "2026-01-10T00:00:00",
        "--end",
        "2026-01-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/conditions.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn imports_matches_expected_json() {
    // 1 января — праздник из holidays.cyclo: рейс в 10:00 есть, в 12:00 нет.
    let out = run(&[
        "run",
        "../../examples/imports.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/imports.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn empty_window_gives_empty_events() {
    let out = run(&[
        "run",
        "../../examples/route.cyclo",
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
    ] {
        let path = format!("../../examples/{file}.cyclo");
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
        "../../examples/bad_syntax.cyclo",
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
        vec![],
        vec!["run"],
        vec!["run", "../../examples/route.cyclo"],
        vec![
            "run",
            "../../examples/route.cyclo",
            "--start",
            "2026-01-10T00:00:00",
        ],
        vec![
            "run",
            "../../examples/route.cyclo",
            "--start",
            "not-a-datetime",
            "--end",
            "2026-01-11T00:00:00",
        ],
    ] {
        let out = run(&args);
        assert!(!out.status.success(), "для {args:?}");
        assert!(out.stdout.is_empty(), "для {args:?}");
        assert!(!out.stderr.is_empty(), "для {args:?}");
    }
}
