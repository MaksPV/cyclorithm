<div align="center">

# Cyclorithm

[![CI](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/ci.yml)
[![build](https://github.com/MaksPV/cyclorithm/actions/workflows/build.yml/badge.svg)](https://github.com/MaksPV/cyclorithm/actions/workflows/build.yml)
[![Release](https://img.shields.io/github/v/release/MaksPV/cyclorithm)](https://github.com/MaksPV/cyclorithm/releases)
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

Команды: `cyclo run FILE --start T --end T` (окно), `cyclo next FILE [--from T] [--within D] [-n K]` (первые `K` событий от `from`, по умолчанию — now), `cyclo check FILE` (только проверки, печатает `ok`). Вместо файла — `-` (stdin, `use` тогда от cwd). Даты короткие: `2026-09-07`, `2026-09-07T09:30`, `+7d` (для `--end` — от `--start`). Флаг `--ndjson` (`run`, `next`) — по событию на строку для пайпов. Успех — вывод в stdout, код `0`. Ошибка — текст в stderr, в stdout ничего: код `1` (ввод, парсинг, валидация), код `2` (неверные аргументы, текст usage). Готовые бинарники (Linux/Windows, x86-64/ARM64) — в [релизах](https://github.com/MaksPV/cyclorithm/releases).

## Мини-тур по языку

### Точки и циклы

Точки объявляют действия, циклы — цепочки вызовов со смещениями от старта:

```text
point DEPOT {
  actions = [depart, arrive];
}

cycle CITY_ROUTE duration = 1h20m {
  0m: DEPOT.depart();
  40m: AIRPORT.arrive();
  50m: AIRPORT.depart();
  -0m: DEPOT.arrive();
}
```

Отрицательное смещение `-0m` — «встык к концу цикла» (здесь ≡ `80m`); ниже нуля — ошибка. Корень расписания — `root_cycle` с `start_time` и `duration`, его окно пересекают с `--start/--end`. Подробности — в §4 спеки.

### Условия и календарь

Строки могут иметь условие `[…]` — оно вычисляется для времени строки (`at`):

```text
[hour(at) >= 7 and not weekend(at)] 10h: repeat 2 SHUTTLE();
[weekend(at)] 12h: CITY_ROUTE();
```

Время суток без даты-условия — мёртвый код (константа во всех экземплярах), поэтому будние ветки всегда идут в паре с `weekend`. Условие на цепочке повторов проверяется на каждый экземпляр: ложные выпадают дырами, остальные не сдвигаются.

Календарь встроен, подключать не нужно: `weekend`, `workday`, `morning`/`afternoon`/`evening`/`night`, `spring`–`winter`, `hour`/`minute`/`day`/`month`/`year`/`quarter`, `dow` (0 = пн … 6 = вс), `day_of_week` (1 = пн … 7 = вс, ISO), `datestr`/`datetimestr`, `start_of_day`/`start_of_month`, `is_leap`/`days_in_month`, `rand`, константы `mon`–`sun`, `DAY`. Списки дат — строковой альтернацией:

```text
pred holiday(at) = datestr(at) == ("2026-11-04" or "2026-12-31");
```

### Свои объявления

Только верхний уровень файла, до `schedule`:

```text
const MORNING = 6;
fun rush_top(x) = x + 1;
pred commute(at) = morning(at) or evening(at);
```

`const` — число или данные, `fun` — число от аргумента, `pred` — истина/ложь от времени вызова. Плюс конструктор даты `mkdate(2026, 9, 7, 9, 0, 0, 0)`, строки `str`/`pad`, целочисленные `floordiv`/`floormod`. Выражения: целочисленная и битовая арифметика, сравнения чисел и строк, `not`/`and`/`or` (§4.17 спеки).

### Данные и атрибуты

Данные — мапы, массивы и `true`/`false` (только литералы, в питоновском духе). Едут в циклы параметрами и видны в условиях:

```text
const LEC = {"subject": "БЖД", "type": "лек", "room": "233/А", "tags": ["поток"]};
const CORPUS = {"building": "Л"};

point BELL {
  actions = [ring];
  attrs = CORPUS;
}

cycle LESSON(subj) duration = 1h35m {
  0m: BELL.ring() { subject = subj.subject, event = "start" };
  [subj.type == "лек"] 55m: BELL.ring();
}
```

Доступ — `subj.subject`, `tags[0]`. Атрибуты точки (`attrs` — литерал или ссылка на константу-мапу) попадают в каждое событие полем `point_attrs`, блок действий `{...}` — полем `action_attrs`. Оба поля есть всегда; нет данных — пустой `{}`:

```json
{"time": "2026-09-07T09:00:00", "action": "ring", "point": "BELL",
 "point_attrs": {"building": "Л"}, "action_attrs": {"subject": "БЖД", "event": "start"}}
```

### Библиотеки

Общее выносится в библиотеки и подключается первой строкой файла:

```text
use "libs/route_lib.cyclo";
```

Путь — от директории импортирующего, транзитивно; `schedule` внутри библиотеки запрещён. Полный список кодов ошибок (§5 спеки): `E01–E16`.

### Повторы

```text
10h: repeat 2 SHUTTLE();
14h: fill until 15h SHUTTLE();
```

`repeat N` — ровно `N` экземпляров, `fill` — сколько влезет в объемлющий цикл, `fill until T` — сколько влезет до смещения `T` от старта объемлющего цикла (не от строки).

### Таблицы времени и рутины

Расписание «по звонкам» — таблица слотов плюс рутина-день, вызываемая с таблицей и данными:

```text
time_const DAY duration = 24h {
  1st: 9h;
  [workday(at)] lunch: 12h -> LUNCH();
}

routine MONDAY(TC, subj) {
  1st: LESSON(subj);
}

root_cycle start_time = "2026-09-07T00:00:00", duration = 24h {
  [day_of_week(at) == 1] 0h: MONDAY(DAY, LEC);
}
```

Метки тела заменяются смещениями таблицы (нет метки — `E16`), первый параметр — всегда таблица, пожары таблицы (`->`) добавляются после строк тела. Так один шаблон дня едет на разных сетках звонков — см. живое расписание группы в `examples/real/bvt231.cyclo`.

## Примеры

- `examples/valid/` — контрактные: `route.cyclo` (+ `libs/route_lib.cyclo`, `route.expected.json` — дословно §1 спеки), `attrs`, `routines`, `conditions`, `repeat`, `neg_offsets`, `imports`, `bitwise`, `bool_groups`, `mkdate`, `rand`. Запуск: `cargo run -p cyclorithm-cli -- run examples/valid/<name>.cyclo --start … --end …`, сверка с `.expected.json`.
- `examples/real/` — живые расписания: автобус `101.cyclo`, автопарк `fleet.cyclo` (+ `fleet_lib.cyclo`, 5 бортов, 218 событий за неделю), учебная группа `bvt231.cyclo` (рутины, семестр, госпраздники).
- `examples/invalid/bad_*.cyclo` — негативные кейсы: минимум один файл на код ошибки (`bad_syntax.cyclo` — ошибка парсера без кода).

## Разработка

```console
$ cargo test --workspace
$ cargo fmt --all -- --check
$ cargo clippy --workspace --all-targets
```

То же гоняет CI на push/PR в `main`/`dev`. Правила работы с репозиторием (конвейер фаз, тесты, как менять спеку, оформление коммитов) — в `AGENTS.md`. Рабочая ветка — `dev`, коммиты и пуши — только вручную.

## Дорожная карта

Уже готово (0.3.0): условия и объявления, `use`-импорты, повторы `repeat`/`fill`, отрицательные смещения, выражения §4.17, календарь прелюдии (`workday`, `day_of_week`, `datestr`, `mkdate`, `rand`), атрибуты точек и действий (`E15`), таблицы времени и рутины (`E16`), команды `next`/`check`, короткие даты, stdin, `--ndjson`.

Дальше:

- **cron-демон**: хост-программа поверх `next` (поллинг окна) — пример расписания задач уже есть (`examples/real/cron.cyclo`, команды в `action_attrs`).
- **Плейграунд**: таймлайн + редактор кода + таблица событий в реальном времени (прототип — отдельный репозиторий `cyclorithm-playground`, запуск — `run.sh`).

## Лицензия

MIT. См. `LICENSE`.
