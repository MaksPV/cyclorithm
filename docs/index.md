# Cyclorithm

**Cyclorithm — DSL для циклических расписаний: повторяющиеся процессы описываются как циклы, на выходе — плоский список событий.**

```text
cycle CITY_ROUTE duration = 1h20m {
  0m: DEPOT.depart();
  40m: AIRPORT.arrive();
}
```

```console
$ cyclo run route.cyclo --start 2026-01-09T00:00:00 --end 2026-01-10T00:00:00
{"schedule":"Автобусный парк","events":[...18 событий...]}
```

Первый результат за 2 минуты — в [туториале](getting-started/tutorial.md).

## Карта документации

| Раздел | Что внутри |
|--------|-----------|
| [Установка](getting-started/install.md) | Rust, бинарь `cyclo`, релизные архивы |
| [Туториал](getting-started/tutorial.md) | Первый проект с нуля, без деталей |
| [Концепты](reference/concepts.md) | Сущности: расписание, точка, цикл, рутина, событие |
| [Синтаксис](reference/syntax.md) | Грамматика EBNF, имена, длительности, условия |
| [Семантика](reference/semantics.md) | Решётка корневого цикла, повторы, данные |
| [Ошибки](reference/errors.md) | Ситуации поименно: сообщение и исправление |
| [Формат вывода](reference/output.md) | JSON Schema и календарь ICS |
| [CLI](reference/cli.md) | Команды `run` / `next` / `check`, коды 0/1/2 |
| [Стандартная библиотека](reference/stdlib.md) | Календарь, `day_of_week`, `rand` |
| [Рецепты](cookbook/index.md) | Готовые фрагменты на типовые задачи |
| [Интеграции](ecosystem/integrations.md) | HTTP-сервер, ICS, WASM-плейграунд |
| [Глоссарий](glossary.md) | Один термин — одно значение |

## До и после

Императивный подсчёт на Python — дни, выходные и праздники вручную:

```python
for day in january:
    if day.weekday() < 5:
        add_route(day.at(6, 0))
        add_route(day.at(18, 0))
    else:
        add_route(day.at(12, 0))
```

То же на Cyclorithm — декларативно, условия и повторы встроены:

```text
root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {
  [not weekend(at)] 6h: CITY_ROUTE();
  [commute(at) and not weekend(at)] 18h: CITY_ROUTE();
  [weekend(at)] 12h: CITY_ROUTE();
}
```

!!! note "Подсветка кода"
    Отдельного лексера `.cyclo` для Pygments нет — блоки языка
    оформляются как ` ```text `, JSON — как ` ```json `.
