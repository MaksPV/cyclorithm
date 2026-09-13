# Установка

## Быстрый путь: готовый бинарь

Собранный `cyclo` (Linux/Windows, x86-64/ARM64) — в
[релизах](https://github.com/MaksPV/cyclorithm/releases). Проверка:

```console
$ cyclo --version
cyclo 0.3.0
```

## Из исходников

Требуется стабильный Rust (`rustup`):

```console
$ cargo run -p cyclorithm-cli -- run examples/valid/route.cyclo \
    --start 2026-01-09T00:00:00 --end 2026-01-10T00:00:00
```

## Проверка окружения

```console
$ cargo test --workspace
$ cargo fmt --all -- --check
$ cargo clippy --workspace --all-targets
```

Те же проверки выполняет CI на push и PR в `main`/`dev`.

Дальше — [туториал с нуля](tutorial.md).
