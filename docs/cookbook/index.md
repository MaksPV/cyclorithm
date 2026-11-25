# Рецепты

Примеры для типовых задач. При переносе в своё расписание подставьте имена объявленных циклов, точек и данных.

## Будни и выходные разными рейсами

```cyclo
root_cycle start_time = "2026-01-01T00:00:00", duration = 24h {
  [not weekend(at)] 6h: CITY_ROUTE();
  [weekend(at)] 12h: CITY_ROUTE();
}
```

## Повторы `repeat` и `fill`

```cyclo
[hour(at) >= 7 and not weekend(at)] 10h: repeat 2 SHUTTLE();
[not weekend(at)] 14h: fill until 15h SHUTTLE();
```

Граница `until` отсчитывается от начала текущего цикла. Условие
проверяется в момент каждого запуска. Если один запуск пропущен,
время следующих запусков сохраняется.

## Действие в конце цикла

```cyclo
cycle CITY_ROUTE duration = 1h20m {
  0m: DEPOT.depart();
  -0m: DEPOT.arrive();
}
```

Смещение `-0m` эквивалентно `80m`. Смещение, которое указывает на момент до начала цикла, вызывает `offset-out-of-bounds`.

## Расписание по звонкам

```cyclo
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

```cyclo
pred holiday(at) = datestr(at) == ("2026-11-04" or "2026-12-31");
```

## Задачи по расписанию

В `examples/real/cron.cyclo` команды для задач хранятся в `action_attrs`.
Чтобы получить ближайшие события расписания, выполните `cyclo next`.
