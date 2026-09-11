# Туториал: первое расписание с нуля

Цель — за 5 шагов получить собственный список событий. Технические детали
намеренно опущены — за ними в [Справочник](../reference/concepts.md).

## Шаг 1. Точка `point`

Точка — место, где происходят действия:

```text
point DEPOT {
  actions = [depart, arrive];
}
```

## Шаг 2. Цикл `cycle`

Цикл — именованная цепочка строк со смещениями от старта:

```text
cycle CITY_ROUTE duration = 1h20m {
  0m: DEPOT.depart();
  40m: DEPOT.arrive();
}
```

## Шаг 3. Расписание и корневой цикл `root_cycle`

Корневой цикл задаёт, когда запускать циклы. Период — `24h`,
опорная точка — `start_time`:

```text
schedule "Мой парк" {
  point DEPOT {
    actions = [depart, arrive];
  }

  cycle CITY_ROUTE duration = 1h20m {
    0m: DEPOT.depart();
    40m: DEPOT.arrive();
  }

  root_cycle start_time = "2026-01-09T00:00:00", duration = 24h {
    6h: CITY_ROUTE();
  }
}
```

Сохраните как `my.cyclo`.

## Шаг 4. Запуск

```console
$ cyclo run my.cyclo --start 2026-01-09T00:00:00 --end 2026-01-10T00:00:00
```

## Шаг 5. Читаем вывод

Каждое событие — `{time, action, point, point_attrs, action_attrs}`:

```json
{"time": "2026-01-09T06:00:00", "action": "depart", "point": "DEPOT", "point_attrs": {}, "action_attrs": {}}
```

При ошибке сверьтесь с разделом [ошибок](../reference/errors.md) — там каждый
код с примером некорректного файла и исправлением.

Следующий шаг — [концепты](../reference/concepts.md): чем цикл отличается
от рутины и таблицы времени.
