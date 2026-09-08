<div align="center">

# Cyclorithm

[![CI](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
![Rust](https://img.shields.io/badge/rust-stable-orange.svg)

**DSL и движок для циклических расписаний: повторяющиеся процессы описываются как циклы, на выходе — плоский список событий.**

</div>

Канон — `docs/spec.md`: грамматика, семантика, коды ошибок и формат вывода. Ниже — вход за пять минут.

## Быстрый старт

Требуется стабильный Rust (`rustup`).

```console
$ cargo run -p cyclorithm-cli -- run examples/valid/route.cyclo --start 2026-01-09T00:00:00 --end 2026-01-10T00:00:00
{"schedule":"Автобусный парк","start":"2026-01-09T00:00:00","end":"2026-01-10T00:00:00","events":[{"time":"2026-01-09T06:00:00","action":"depart","point":"DEPOT"},{"time":"2026-01-09T06:40:00","action":"arrive","point":"AIRPORT"},{"time":"2026-01-09T06:50:00","action":"depart","point":"AIRPORT"},{"time":"2026-01-09T07:20:00","action":"arrive","point":"DEPOT"}, ... ]}
```

Окно — пятница 09.01: работают все ветки маршрута (будние рейсы, повторы, рейс в 18:00). Всего 18 событий, выше — первые 4; полный вывод — в `examples/valid/route.expected.json`. В субботу останутся только рейсы в 6:00 и 12:00 — так работают условия (см. мини-тур).

Формат вызова: `cyclo run FILE --start DATETIME --end DATETIME` (даты — наивный ISO8601 `YYYY-MM-DDTHH:MM:SS`, миллисекунды опциональны). Успех — один JSON-объект в stdout, код `0`. Ошибка — текст в stderr, в stdout ничего: код `1` (ввод, парсинг, валидация), код `2` (неверные аргументы, текст usage).

## Мини-тур по языку

Точки объявляют действия, циклы — цепочки вызовов со смещениями от старта:

```text
point DEPOT {
  actions = [depart, arrive];
}

cycle CITY_ROUTE
  duration = 1h20m
{
  0m: DEPOT.depart();
  40m: AIRPORT.arrive();
  50m: AIRPORT.depart();
  -0m: DEPOT.arrive();
}
```

Отрицательное смещение `-0m` — «встык к концу цикла» (здесь ≡ `80m`); ниже нуля — ошибка. Подробности — в §4 спеки.

Строки могут иметь условие `[…]` — оно вычисляется для времени строки (`at`):

```text
[hour(at) >= 7 and not weekend(at)] 10h: repeat 2 SHUTTLE();
[weekend(at)] 12h: CITY_ROUTE();
```

Время суток без даты-условия — мёртвый код (константа во всех экземплярах), поэтому будние ветки всегда идут в паре с `weekend`. Условие на цепочке повторов проверяется на каждый экземпляр: ложные выпадают дырами, остальные не сдвигаются.

Свои объявления — только верхний уровень файла, до `schedule`:

```text
const MORNING = 6;
fun rush_top(x) = x + 1;
pred commute(at) = morning(at) or evening(at);
```

`const` — число, `fun` — число от аргумента, `pred` — истина/ложь от времени вызова. Плюс неявная прелюдия (календарь и словарь: `morning`, `evening`, `weekend`, `hour`, …) — подключать не нужно. Выражения: целочисленная арифметика, сравнения чисел и строк, `not`/`and`/`or`, `str`/`pad` (§4.17 спеки).

Общее выносится в библиотеки и подключается первой строкой файла:

```text
use "libs/route_lib.cyclo";
```

Путь — от директории импортирующего, транзитивно; `schedule` внутри библиотеки запрещён. Полный список кодов ошибок (§5 спеки): `E01–E14`.

Повторы — только для вызовов циклов:

```text
10h: repeat 2 SHUTTLE();
14h: fill until 15h SHUTTLE();
```

`repeat N` — ровно `N` экземпляров, `fill` — сколько влезет в объемлющий цикл, `fill until T` — сколько влезет до смещения `T` от старта объемлющего цикла (не от строки).

## Примеры

- `examples/valid/` — контрактные: `route.cyclo` (+ `libs/route_lib.cyclo`, `route.expected.json` — дословно §1 спеки), `neg_offsets`, `repeat`, `conditions`, `imports`. Запуск: `cargo run -p cyclorithm-cli -- run examples/valid/<name>.cyclo --start … --end …`, сверка с `.expected.json`.
- `examples/real/` — живые расписания: `101.cyclo`, автопарк `fleet.cyclo` (+ `fleet_lib.cyclo`, 5 бортов, 218 событий за неделю).
- `examples/invalid/bad_*.cyclo` — негативные кейсы: минимум один файл на код ошибки (`bad_syntax.cyclo` — ошибка парсера без кода).

## Разработка

```console
$ cargo test --workspace
$ cargo fmt --all -- --check
$ cargo clippy --workspace --all-targets
```

То же гоняет CI на push/PR в `main`/`dev`. Правила работы с репозиторием (конвейер фаз, тесты, как менять спеку, оформление коммитов) — в `AGENTS.md`. Рабочая ветка — `dev`, коммиты и пуши — только вручную.

## Дорожная карта

Уже готово: условия и объявления, `use`-импорты, повторы `repeat`/`fill`, отрицательные смещения, выражения §4.17.

Дальше:

- **Атрибуты точек** (`point_attrs`, черновик согласован): произвольные данные точки прямо в `.cyclo`, в выводе — поле `point_attrs` у каждого события; дублям — новый код `E15` (коды `E10`/`E11` уже заняты).
- **`next-event` в CLI**: ближайшее будущее событие к текущему моменту — задел под аналог cron.
- **GUI-редактор**: таймлайн + редактор кода + таблица событий в реальном времени. Идея, scope под вопросом — возможно, отдельный репозиторий.

## Лицензия

MIT. См. `LICENSE`.
