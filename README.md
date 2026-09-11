<div align="center">

# Cyclorithm

[![CI](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml)
[![build](https://github.com/MaksPV/cyclorithm/actions/workflows/build.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/build.yml)
[![docs](https://github.com/MaksPV/cyclorithm/actions/workflows/docs.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/docs.yml)
[![Release](https://img.shields.io/github/v/release/MaksPV/cyclorithm)](https://github.com/MaksPV/cyclorithm/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
![Rust](https://img.shields.io/badge/rust-stable-orange.svg)

**DSL для циклических расписаний: процессы описываются циклами так, чтобы их читал человек, а проверял движок. На выходе — плоский список событий.**

</div>

Документация — вики: [makspv.github.io/cyclorithm](https://makspv.github.io/cyclorithm/).

## Быстрый старт

Требуется стабильный Rust (`rustup`).

```console
$ cargo run -p cyclorithm-cli -- run examples/valid/route.cyclo --start 2026-01-09T00:00:00 --end 2026-01-10T00:00:00
```

Вывод — объект со списком событий (пятница 09.01, 18 событий). Формат,
команды `next`/`check`, примеры и язык — в [вики](https://makspv.github.io/cyclorithm/).
Готовые бинарники (Linux/Windows, x86-64/ARM64) — в [релизах](https://github.com/MaksPV/cyclorithm/releases).

## Разработка

Правила работы с репозиторием — в [`AGENTS.md`](AGENTS.md).

## Лицензия

MIT. См. `LICENSE`.
