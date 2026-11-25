# Установка

## Готовая сборка

Скачайте `cyclo` для Linux или Windows (x86-64 или ARM64) со
[страницы релизов](https://github.com/MaksPV/cyclorithm/releases).
После распаковки проверьте версию:

```console
$ cyclo --version
cyclo 0.3.2
```

## Из исходников

Требуется стабильная версия Rust (`rustup`). Выполните команду из корня репозитория:

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

Те же проверки выполняются в CI при отправке изменений и открытии pull request в `main` или `dev`.

Далее можно составить [первое расписание](tutorial.md).
