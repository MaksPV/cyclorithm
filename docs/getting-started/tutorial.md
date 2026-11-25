# Первое расписание

За пять шагов вы составите расписание одного рейса и получите список его событий.
Термины и правила языка описаны в [справочнике](../reference/concepts.md).

## Шаг 1. Точка `point`

Точка — место, где происходят действия:

```cyclo
point DEPOT {
  actions = [depart, arrive];
}
```

## Шаг 2. Цикл `cycle`

Цикл — именованная цепочка строк со смещениями от старта:

```cyclo
cycle CITY_ROUTE duration = 1h20m {
  0m: DEPOT.depart();
  40m: DEPOT.arrive();
}
```

## Шаг 3. Расписание и корневой цикл `root_cycle`

Корневой цикл задаёт, когда запускать циклы. Период — `24h`,
опорная точка — `start_time`:

```cyclo
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

## Шаг 5. Проверьте вывод

Команда выведет два события: отправление в 06:00 и прибытие в 06:40.
Первое событие выглядит так:

```json
{"time": "2026-01-09T06:00:00", "action": "depart", "point": "DEPOT", "point_attrs": {}, "action_attrs": {}}
```

Если команда вернула ошибку, найдите её код и способ исправления в [справочнике ошибок](../reference/errors.md).

Отличия цикла, рутины и таблицы времени описаны в разделе [«Основные понятия»](../reference/concepts.md).
