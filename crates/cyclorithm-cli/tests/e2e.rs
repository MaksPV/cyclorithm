//! Сквозные тесты контракта CLI (см. docs/reference/cli.md): JSON в stdout, ошибки в stderr.

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

/// Прогон с переменными окружения (пин `TZ` для дефолтного окна:
/// иначе CI в UTC и dev в Москве разъедутся).
fn run_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = cyclo();
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("бинарь cyclo обязан запускаться")
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
fn free_headers_matches_expected_json() {
    // Обратный порядок полей шапки + timezone без висячей запятой:
    // то же событие, что при прямом порядке.
    let out = run(&[
        "run",
        "../../examples/valid/free_headers.cyclo",
        "--start",
        "2026-01-09T00:00:00",
        "--end",
        "2026-01-10T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/free_headers.expected.json");
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
fn julian_matches_expected_json() {
    // Юлианское Рождество (25.12 → 07.01) и roundtrip 30.03 → 12.04.
    let out = run(&[
        "run",
        "../../examples/valid/julian.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-05-11T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/julian.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn julian_1900_matches_expected_json() {
    // 29.02.1900 существует по-юлиански (13.03 н.ст.), mkdate его отверг бы.
    let out = run(&[
        "run",
        "../../examples/valid/julian_1900.cyclo",
        "--start",
        "1900-03-01T00:00:00",
        "--end",
        "1900-03-31T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/julian_1900.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn reverse_matches_expected_json() {
    // Зеркало FWD (0/20/50 при 1h) едет как 60/40/10: параметры и условие — с ним.
    let out = run(&[
        "run",
        "../../examples/valid/reverse.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/reverse.expected.json");
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
fn action_attrs_matches_expected_json() {
    // Блок вызова: JSON-литерал, ссылка на const-мапу и пустой `{}`.
    let out = run(&[
        "run",
        "../../examples/valid/action_attrs.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/action_attrs.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn dict_names_matches_expected_json() {
    // Имена/выражения в словарях: const-словарь, инлайновый литерал и
    // аргумент-литерал с параметром.
    let out = run(&[
        "run",
        "../../examples/valid/dict_names.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/dict_names.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn index_matches_expected_json() {
    // Индекс-выражения: `[0]`, `[-1]`, `[1 + 1]`, индекс-параметр, цепочка.
    let out = run(&[
        "run",
        "../../examples/valid/index.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/index.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn gaps_matches_expected_json() {
    // fill gaps: без until добивает до конца цикла, until ограничивает окно.
    let out = run(&[
        "run",
        "../../examples/valid/gaps.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/gaps.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn dependencies_matches_expected_json() {
    // Issue 45: метки as, зависимости after/at, курсор +/+:, функции
    // max/min/start/end, repeat с динамическим стартом, каскадный skip.
    let out = run(&[
        "run",
        "../../examples/valid/dependencies.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/dependencies.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    assert!(out.stderr.is_empty(), "при успехе stderr пуст");
}

#[test]
fn launch_labels_belong_to_each_cycle_instance() {
    let src = r#"
    schedule "Метки экземпляра" {
      point P { actions = [start, after]; }
      cycle WORK duration = 1h { 0h: P.start(); }
      cycle DAY(enabled) duration = 2h {
        [enabled] 0h: WORK() as L;
        after L: P.after();
      }
      root_cycle start_time = "2026-01-01T00:00:00", duration = 4h {
        0h: DAY(true);
        0h: DAY(false);
        2h: DAY(false);
      }
    }
    "#;
    let out = run_stdin(
        &[
            "run",
            "-",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-01T04:00",
        ],
        src,
    );
    let json = stdout_json(&out);
    let events = json["events"].as_array().unwrap();
    let actions: Vec<_> = events
        .iter()
        .map(|event| event["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["start", "after"]);
    assert_eq!(events[1]["time"], "2026-01-01T01:00:00");
}

#[test]
fn dynamic_fill_keeps_only_complete_instances() {
    let src = r#"
    schedule "Полные запуски" {
      point P { actions = [work, mark]; }
      cycle C duration = 1h { 0h: P.work(); }
      root_cycle start_time = "2026-01-01T00:00:00", duration = 4h {
        +30m: fill until 1h30m C();
        +: fill until 2h C();
        +: P.mark();
        +2h: fill C();
      }
    }
    "#;
    let out = run_stdin(
        &[
            "run",
            "-",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-01T04:00",
        ],
        src,
    );
    let json = stdout_json(&out);
    let events = json["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["time"], "2026-01-01T00:30:00");
    assert_eq!(events[1]["action"], "mark");
    assert_eq!(events[1]["time"], "2026-01-01T01:30:00");
}

#[test]
fn gap_fills_respect_dynamic_occupancy() {
    let src = r#"
    schedule "Занятость" {
      point P { actions = [work, breaktime]; }
      cycle C duration = 1h { 0h: P.work(); }
      cycle F duration = 1h { 0h: P.breaktime(); }
      cycle DAY duration = 5h {
        [false] 3h: C() as L;
        after L: C();
        +1h: C();
        [false] +: C();
        0h: fill gaps F();
        0h: fill gaps F();
        3h: C();
      }
      root_cycle start_time = "2026-01-01T00:00:00", duration = 5h {
        0h: DAY();
      }
    }
    "#;
    let out = run_stdin(
        &[
            "run",
            "-",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-01T05:00",
        ],
        src,
    );
    let json = stdout_json(&out);
    let events = json["events"].as_array().unwrap();
    let got: Vec<_> = events
        .iter()
        .map(|event| {
            (
                event["time"].as_str().unwrap(),
                event["action"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("2026-01-01T00:00:00", "breaktime"),
            ("2026-01-01T01:00:00", "work"),
            ("2026-01-01T03:00:00", "work"),
            ("2026-01-01T04:00:00", "breaktime"),
        ]
    );
}

#[test]
fn gap_fills_reserve_routine_slot_calls() {
    let src = r#"
    time_const T duration = 5h {
      reserved: 2h -> WORK();
    }

    schedule "Добивка в рутине" {
      point P { actions = [big, small, work]; }
      cycle BIG duration = 2h { 0h: P.big(); }
      cycle SMALL duration = 30m { 0h: P.small(); }
      cycle WORK duration = 1h { 0h: P.work(); }

      routine DAY(TC) {
        0h: fill gaps BIG();
        0h: fill gaps SMALL();
      }

      root_cycle start_time = "2026-01-01T00:00:00", duration = 5h {
        0h: DAY(T);
      }
    }
    "#;
    let run = run_stdin(&["run", "-", "--start", "2026-01-01", "--end", "+5h"], src);
    let next = run_stdin(
        &[
            "next",
            "-",
            "--from",
            "2026-01-01",
            "--within",
            "5h",
            "-n",
            "6",
        ],
        src,
    );
    let window = stdout_json(&run);
    let events = window["events"].as_array().unwrap();
    let got: Vec<_> = events
        .iter()
        .map(|event| {
            (
                event["time"].as_str().unwrap(),
                event["action"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("2026-01-01T00:00:00", "big"),
            ("2026-01-01T02:00:00", "work"),
            ("2026-01-01T03:00:00", "big"),
        ]
    );
    assert_eq!(stdout_json(&next)["events"], window["events"]);
}

#[test]
fn check_rejects_statically_invalid_dynamic_rows() {
    for (row, diagnostic) in [
        ("after L: C() as L;", "forward-label-reference"),
        ("+: repeat 0 C();", "invalid-repeat-count"),
        ("after OFF: repeat 0 C();", "invalid-repeat-count"),
        ("+: repeat 2 P.work();", "repeat-point-action"),
        ("+1h2h: C();", "invalid-duration"),
        ("max(end(OFF), +1h2h): C();", "invalid-duration"),
        ("after OFF: fill until 5h C();", "until-out-of-bounds"),
    ] {
        let src = format!(
            r#"
            schedule "Проверка" {{
              point P {{ actions = [work]; }}
              cycle C duration = 1h {{ 0h: P.work(); }}
              root_cycle start_time = "2026-01-01T00:00:00", duration = 4h {{
                [false] 0h: C() as OFF;
                {row}
              }}
            }}
            "#
        );
        let out = run_stdin(&["check", "-"], &src);
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert_eq!(out.status.code(), Some(1), "{row}: {stderr}");
        assert!(
            stderr.starts_with(&format!("{diagnostic}: ")),
            "{row}: {stderr}"
        );
    }
}

#[test]
fn gaps_pack_matches_expected_json() {
    // fill gaps pack: left — встык слева, right — справа, center — по центру.
    let out = run(&[
        "run",
        "../../examples/valid/gaps_pack.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/gaps_pack.expected.json");
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
fn float_matches_expected_json() {
    // Float-литералы: точка/экспонента в attrs и блоках, арифметика с
    // промоушеном, сравнения, str(), 0.0 — ложь в условии.
    let out = run(&[
        "run",
        "../../examples/valid/float.cyclo",
        "--start",
        "2026-01-01T00:00:00",
        "--end",
        "2026-01-02T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/float.expected.json");
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
fn empty_windows_do_not_evaluate_rows() {
    let src = r#"
    schedule "Пустое окно" {
      point P { actions = [x]; }
      root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {
        [1 / hour(at)] 0h: P.x();
        23h: P.x();
      }
    }
    "#;
    for end in ["2026-01-01T12:00", "2026-01-01T11:00"] {
        let out = run_stdin(
            &["run", "-", "--start", "2026-01-01T12:00", "--end", end],
            src,
        );
        assert_eq!(stdout_json(&out)["events"], serde_json::json!([]));
    }
    let out = run_stdin(
        &["next", "-", "--from", "2026-01-01T12:00", "--within", "0"],
        src,
    );
    assert_eq!(stdout_json(&out)["events"], serde_json::json!([]));
}

#[test]
fn next_reports_only_errors_within_the_requested_prefix() {
    let src = r#"
    schedule "Ближайшее событие" {
      point P { actions = [ok, bad]; }
      cycle CHILD duration = 2h {
        [1 / (hour(at) - 23)] 1h: P.bad();
      }
      root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {
        22h: CHILD();
        0h: P.ok();
        23h30m: P.ok();
      }
    }
    "#;
    for within in ["1h", "24h"] {
        let out = run_stdin(
            &[
                "next",
                "-",
                "--from",
                "2026-01-01",
                "--within",
                within,
                "-n",
                "1",
            ],
            src,
        );
        let json = stdout_json(&out);
        let events = json["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["time"], "2026-01-01T00:00:00");
    }

    let out = run_stdin(
        &[
            "next",
            "-",
            "--from",
            "2026-01-01",
            "--within",
            "24h",
            "-n",
            "2",
        ],
        src,
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("division-by-zero: "));

    let out = run_stdin(
        &[
            "next",
            "-",
            "--from",
            "2026-01-01T23:15",
            "--within",
            "1h",
            "-n",
            "1",
        ],
        src,
    );
    assert_eq!(
        stdout_json(&out)["events"][0]["time"],
        "2026-01-01T23:30:00"
    );
}

#[test]
fn rejects_invalid_dates_including_unicode_without_panicking() {
    let src = r#"
    schedule "Дата" {
      point P { actions = [x]; }
      root_cycle start_time = "2026-01-01T00:00:00", duration = 24h { 0h: P.x(); }
    }
    "#;
    for invalid in ["2026-02-30T00:00:00", "январь."] {
        let bad_src = src.replace("2026-01-01T00:00:00", invalid);
        let out = run_stdin(&["check", "-"], &bad_src);
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).starts_with("invalid-datetime: "));
    }
    let out = run_stdin(&["run", "-", "--start", "январь.", "--end", "+1h"], src);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("invalid-datetime: "));
}

#[test]
fn invalid_negative_moment_is_checked_before_resolving_labels() {
    for moment in ["min(end(L), -25h)", "min(-25h, end(L))"] {
        let src = format!(
            r#"
        schedule "Границы момента" {{
          point P {{ actions = [x]; }}
          cycle C duration = 1h {{ 0h: P.x(); }}
          root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {{
            [false] 0h: C() as L;
            {moment}: P.x();
          }}
        }}
        "#
        );
        let out = run_stdin(&["check", "-"], &src);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(
            String::from_utf8_lossy(&out.stderr).trim(),
            "offset-out-of-bounds: offset '-25h' out of bounds (duration 24h)"
        );
    }
}

#[test]
fn duration_shaped_launch_labels_work_in_moment_functions() {
    let src = r#"
    schedule "Числовая метка" {
      point P { actions = [x]; }
      cycle C duration = 1h {}
      root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {
        2h: C() as 1ms;
        4h: C() as 1msX;
        start(1ms): P.x();
        end(1ms): P.x();
        end(1msX): P.x();
        min(1ms, end(1ms)): P.x();
      }
    }
    "#;
    let out = run_stdin(&["run", "-", "--start", "2026-01-01", "--end", "+1d"], src);
    let json = stdout_json(&out);
    let events = json["events"].as_array().unwrap();
    let times: Vec<_> = events
        .iter()
        .map(|event| event["time"].as_str().unwrap())
        .collect();
    assert_eq!(
        times,
        [
            "2026-01-01T00:00:00.001",
            "2026-01-01T02:00:00",
            "2026-01-01T03:00:00",
            "2026-01-01T05:00:00",
        ]
    );
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
    for (file, slug, message) in [
        ("bad_unknown-point", "unknown-point", "unknown point 'PORT'"),
        (
            "bad_action-not-allowed",
            "action-not-allowed",
            "action 'arrive' not allowed for point 'DEPOT'",
        ),
        (
            "bad_unknown-cycle",
            "unknown-cycle",
            "unknown cycle 'NIGHT_ROUTE'",
        ),
        (
            "bad_reverse-unknown",
            "unknown-cycle",
            "unknown cycle 'NIGHT_ROUTE'",
        ),
        ("bad_duplicate", "duplicate", "duplicate point 'DEPOT'"),
        ("bad_reserved-name", "reserved-name", "reserved name 'at'"),
        (
            "bad_invalid-duration",
            "invalid-duration",
            "invalid duration '1h2h'",
        ),
        ("bad_recursive", "recursive", "recursive cycle 'A'"),
        (
            "bad_cycle-overruns",
            "cycle-overruns",
            "cycle 'CYCLE2' overruns 'CYCLE1' by 20m (80m > 60m)",
        ),
        (
            "bad_offset-out-of-bounds",
            "offset-out-of-bounds",
            "offset '-2h' out of bounds (duration 1h20m)",
        ),
        (
            "bad_cycle-overruns-chain",
            "cycle-overruns",
            "cycle 'R' overruns 'root_cycle' by 100m (1540m > 1440m)",
        ),
        (
            "bad_cycle-overruns-repeat",
            "cycle-overruns",
            "cycle 'D' overruns 'C' by 999999999930m (999999999990m > 60m)",
        ),
        (
            "bad_until-out-of-bounds",
            "until-out-of-bounds",
            "until '30h' out of bounds (duration 24h)",
        ),
        (
            "bad_invalid-repeat-count",
            "invalid-repeat-count",
            "invalid repeat count '0'",
        ),
        (
            "bad_fill-zero-duration",
            "fill-zero-duration",
            "fill of zero-duration cycle 'EMPTY'",
        ),
        (
            "bad_repeat-point-action",
            "repeat-point-action",
            "repeat of point action 'depart' not allowed",
        ),
        (
            "bad_gaps-until",
            "until-out-of-bounds",
            "until '30h' out of bounds (duration 24h)",
        ),
        (
            "bad_gaps-fill0",
            "fill-zero-duration",
            "fill of zero-duration cycle 'EMPTY'",
        ),
        (
            "bad_gaps-window",
            "until-out-of-bounds",
            "until '2h' out of bounds (duration 24h)",
        ),
        ("bad_unknown-name", "unknown-name", "unknown name 'banana'"),
        (
            "bad_unknown-name-dict",
            "unknown-name",
            "unknown name 'NOPE'",
        ),
        (
            "bad_unknown-name-private",
            "unknown-name",
            "unknown name '__z'",
        ),
        (
            "bad_recursive-definition",
            "recursive-definition",
            "recursive definition 'a'",
        ),
        (
            "bad_invalid-date",
            "invalid-date",
            "invalid date '2026-13-01'",
        ),
        (
            "bad_import-cycle",
            "import-cycle",
            "import cycle 'bad_import-cycle_a.cyclo'",
        ),
        (
            "bad_cannot-read-import",
            "cannot-read-import",
            "cannot read import 'no_such_lib.cyclo'",
        ),
        (
            "bad_schedule-in-import",
            "schedule-in-import",
            "schedule not allowed in import 'bad_schedule-in-import-lib.cyclo'",
        ),
        ("bad_duplicate-dup", "duplicate", "duplicate const 'K'"),
        (
            "bad_type-mismatch",
            "type-mismatch",
            "type mismatch: cannot mix number and string",
        ),
        (
            "bad_division-by-zero",
            "division-by-zero",
            "division by zero",
        ),
        (
            "bad_invalid-datetime",
            "invalid-datetime",
            "invalid datetime 'not-a-datetime'",
        ),
        (
            "bad_wrong-kind",
            "wrong-kind",
            "point 'DEPOT' is not a cycle",
        ),
        (
            "bad_duplicate-attribute",
            "duplicate-attribute",
            "duplicate attribute 'a'",
        ),
        (
            "bad_duplicate-attribute-block",
            "duplicate-attribute",
            "duplicate attribute 'a'",
        ),
        (
            "bad_index-out-of-bounds",
            "index-out-of-bounds",
            "index out of bounds '2'",
        ),
        (
            "bad_unknown-table",
            "unknown-table",
            "unknown table 'SHORT'",
        ),
        ("bad_unknown-slot", "unknown-slot", "unknown slot '8th'"),
        (
            "bad_string-too-long",
            "string-too-long",
            "string too long '1000000000'",
        ),
        (
            "bad_float-out-of-range",
            "float-out-of-range",
            "float out of range '1e400'",
        ),
        (
            "bad_integer-out-of-range",
            "integer-out-of-range",
            "integer out of range 'arithmetic overflow'",
        ),
        (
            "bad_integer-out-of-range_mkdate",
            "integer-out-of-range",
            "integer out of range '9223372036854775807-1-1T0:0:0.0'",
        ),
        (
            "bad_invalid-timezone",
            "invalid-timezone",
            "invalid timezone 'MSK'",
        ),
        (
            "bad_missing-argument",
            "missing-argument",
            "missing argument 'duration'",
        ),
        (
            "bad_duplicate-argument",
            "duplicate-argument",
            "duplicate argument 'duration'",
        ),
        ("bad_unknown-label", "unknown-label", "unknown label 'NOPE'"),
        (
            "bad_forward-label-reference",
            "forward-label-reference",
            "label 'W1' used before declaration",
        ),
        (
            "bad_label-not-cycle-call",
            "label-not-cycle-call",
            "label 'W1' is not a cycle call",
        ),
        (
            "bad_invalid-moment-function",
            "invalid-moment-function",
            "invalid moment function: 'start' takes a bare label",
        ),
        (
            "bad_gaps-dynamic-moment",
            "gaps-dynamic-moment",
            "fill gaps with dynamic moment 'after W1' is not supported",
        ),
        (
            "bad_dynamic-moment-after-gaps",
            "dynamic-moment-after-gaps",
            "dynamic moment '+' must precede fill gaps in the same cycle",
        ),
        (
            "bad_reverse-dynamic-moment",
            "reverse-dynamic-moment",
            "cannot reverse dynamic moment '+'",
        ),
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
        assert!(
            err.contains(&format!("{slug}: ")),
            "для {file}: нет префикса {slug:?} в {err:?}"
        );
    }
}

#[test]
fn syntax_error_has_no_slug() {
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
    let out = run(&["check", "../../examples/invalid/bad_unknown-table.cyclo"]);
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
fn tz_offsets_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/tz_offsets.cyclo",
        "--start",
        "2026-01-09T00:00:00",
        "--end",
        "2026-01-10T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/tz_offsets.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    // aware-окно на наивном файле — наследование кадра: стены стоят
    // (06:00 остаётся 06:00 в зоне окна), см. docs/reference/semantics.md
    let out_z = run(&[
        "run",
        "../../examples/valid/tz_offsets.cyclo",
        "--start",
        "2026-01-09T00:00:00+03:00",
        "--end",
        "2026-01-10T00:00:00+03:00",
    ]);
    let got_z = stdout_json(&out_z);
    assert_eq!(got_z["events"][0]["time"], "2026-01-09T06:00:00+03:00");
}

#[test]
fn tz_file_matches_expected_json() {
    let out = run(&[
        "run",
        "../../examples/valid/tz_file.cyclo",
        "--start",
        "2026-01-09T00:00:00",
        "--end",
        "2026-01-10T00:00:00",
    ]);
    let got = stdout_json(&out);
    let expected = include_str!("../../../examples/valid/tz_file.expected.json");
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
    // naive-окно с файлом +03:00 → fallback к зоне файла
    assert_eq!(got["events"][0]["time"], "2026-01-09T06:00:00+03:00");
}

#[test]
fn next_defaults_to_system_zone() {
    // Голый `next` (без `--from`): now с подписью системной зоны —
    // суффиксы `from` и событий из `TZ` (см. docs/reference/cli.md).
    // MSK круглый год +03:00 (без DST с 2014-го), tzdata есть и в CI.
    let out = run_env(
        &["next", "../../examples/valid/tz_offsets.cyclo", "-n", "1"],
        &[("TZ", "Europe/Moscow")],
    );
    let got = stdout_json(&out);
    assert!(
        got["from"].as_str().expect("строка").ends_with("+03:00"),
        "from в зоне TZ: {}",
        got["from"]
    );
    assert!(
        got["events"][0]["time"]
            .as_str()
            .expect("строка")
            .ends_with("+03:00"),
        "событие в зоне TZ: {}",
        got["events"][0]["time"]
    );
    let out_utc = run_env(
        &["next", "../../examples/valid/tz_offsets.cyclo", "-n", "1"],
        &[("TZ", "UTC")],
    );
    let got_utc = stdout_json(&out_utc);
    assert!(
        got_utc["from"].as_str().expect("строка").ends_with("Z"),
        "from в UTC: {}",
        got_utc["from"]
    );
    assert!(
        got_utc["events"][0]["time"]
            .as_str()
            .expect("строка")
            .ends_with("Z"),
        "событие в UTC: {}",
        got_utc["events"][0]["time"]
    );
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
