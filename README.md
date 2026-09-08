<div align="center">

# Cyclorithm

[![CI](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
![Rust](https://img.shields.io/badge/rust-stable-orange.svg)

**DSL и движок для циклических расписаний: повторяющиеся процессы описываются как циклы, на выходе — плоский список событий.**

</div>

## Статус

Рабочий CLI. Канон — `docs/spec.md`

## Быстрый старт

Требуется стабильный Rust (`rustup`).

```console
$ cargo run -p cyclorithm-cli -- run examples/valid/route.cyclo --start 2026-01-10T00:00:00 --end 2026-01-11T00:00:00
{"schedule":"Автобусный парк","start":"2026-01-10T00:00:00","end":"2026-01-11T00:00:00","events":[{"time":"2026-01-10T06:00:00","action":"depart","point":"DEPOT"}, ... ]}
```

Формат вызова: `cyclo run FILE --start DATETIME --end DATETIME` (даты — наивный ISO8601 `YYYY-MM-DDTHH:MM:SS`, миллисекунды опциональны). Успех — один JSON-объект в stdout, код `0`. Ошибка — текст в stderr, в stdout ничего: код `1` (ввод, парсинг, валидация), код `2` (неверные аргументы, текст usage).

## Ошибки валидации

| Код | Случай |
|-----|--------|
| E01 | Неизвестная точка |
| E02 | Действие не разрешено для точки |
| E03 | Неизвестный цикл |
| E04 | Дублирующее объявление |
| E05 | Некорректная длительность (формат, порядок, переполнение, нулевой период `root_cycle`) |
| E06 | Рекурсивный вызов цикла |
| E07 | Вызов выходит за границу объемлющего цикла |
| E08 | Некорректная дата/время |
| E09 | Перепутан род имени (точку вызвали как цикл и наоборот) |

Подробности и точные тексты — в `docs/spec.md` §5. На каждый код есть минимальный пример: `examples/invalid/bad_e01.cyclo` … (плюс `bad_syntax.cyclo` — ошибка парсера без кода).

## Устройство репозитория

- `docs/spec.md` — спецификация языка и поведения.
- `crates/cyclorithm-parser` — грамматика (`pest`) и AST, без валидации.
- `crates/cyclorithm-core` — валидация, развёртка решётки `root_cycle`, сортировка `(time, k, порядок объявления)`.
- `crates/cyclorithm-cli` — бинарь `cyclo` (`run`).
- `examples/` — примеры: `valid/` (контракт + `.expected.json`), `invalid/` (негативные кейсы), `real/` (живые расписания).

Проверки: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` (то же гоняет CI).

## Планы

- Язык: параметры циклов, выражения и вычисляемые поля событий.
- FFI-слой и интеграции: Python, Kotlin.
- Публикация: `crates.io`, готовые сборки `cyclo`.
- Инструменты: `check`, форматирование, позже — плейграунд.

## Лицензия

MIT. См. `LICENSE`.
