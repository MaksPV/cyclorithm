<div align="center">

# Cyclorithm

[![CI](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml)
[![build](https://github.com/MaksPV/cyclorithm/actions/workflows/build.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/build.yml)
[![docs](https://github.com/MaksPV/cyclorithm/actions/workflows/docs.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/docs.yml)
[![Release](https://img.shields.io/github/v/release/MaksPV/cyclorithm)](https://github.com/MaksPV/cyclorithm/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
![Rust](https://img.shields.io/badge/rust-stable-orange.svg)

**Cyclorithm — язык для описания циклических расписаний. Движок проверяет расписание и возвращает список событий за выбранный период.**

</div>

Документация доступна на [сайте проекта](https://makspv.github.io/cyclorithm/).

## Быстрый старт

Требуется стабильный Rust (`rustup`).

```console
$ cargo run -p cyclorithm-cli -- run examples/valid/route.cyclo --start 2026-01-09T00:00:00 --end 2026-01-10T00:00:00
```

Команда выводит JSON-объект с 18 событиями за пятницу, 9 января 2026 года.
Синтаксис языка, примеры и команды `next` и `check` описаны в [документации](https://makspv.github.io/cyclorithm/).
Готовые сборки для Linux и Windows (x86-64 и ARM64) доступны на [странице релизов](https://github.com/MaksPV/cyclorithm/releases).

## Разработка

Правила работы с репозиторием — в [`AGENTS.md`](AGENTS.md).

## Лицензия

MIT. См. `LICENSE`.
