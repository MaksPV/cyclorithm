# Рецепты

Коллекция фрагментов «задача → рабочий код». Каждый копируется как есть.

## Будни и выходные разными рейсами

```text
root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {
  [not weekend(at)] 6h: CITY_ROUTE();
  [weekend(at)] 12h: CITY_ROUTE();
}
```

## Повторы `repeat` и `fill`

```text
[hour(at) >= 7 and not weekend(at)] 10h: repeat 2 SHUTTLE();
[not weekend(at)] 14h: fill until 15h SHUTTLE();
```

Горизонт `until` отсчитывается от старта родителя. Условие проверяется
для каждого экземпляра в его старте (разрывы, не сдвиг сетки).

## Завершение в границу цикла

```text
cycle CITY_ROUTE duration = 1h20m {
  0m: DEPOT.depart();
  -0m: DEPOT.arrive();
}
```

Смещение `-0m` эквивалентно `80m`. Уход ниже нуля — ошибка `E07`.

## Расписание по звонкам

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

Развёрнутый пример на семестр — `examples/real/bvt231.cyclo`.

## Праздники одной строкой

```text
pred holiday(at) = datestr(at) == ("2026-11-04" or "2026-12-31");
```

## Задачи по расписанию

Команды задач в `action_attrs` — `examples/real/cron.cyclo`,
опрос ближайших — командой `next`.
