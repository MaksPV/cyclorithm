# CLI

```text
usage:
  cyclo run FILE|- --start DATETIME --end DATETIME [--ndjson] [--format json|ics]
  cyclo next FILE|- [--from DATETIME] [--within DURATION] [-n K] [--ndjson] [--format json|ics]
  cyclo check FILE|-
  cyclo --version
  cyclo --help
```

- Команда `run` — окно `[start, end)`. При `end <= start` возвращается пустой
  список `events`, код возврата `0`.
- Команда `next` — первые `K` событий от момента `from` (включительно)
  в пределах `[from, from+within)`. Значения по умолчанию: `from` — текущий
  момент (UTC, наивный), `within` — `366d`, `n` — `1`.
- Команда `check` — только проверки без окна: при успехе печатает `ok`,
  код возврата `0`.
- Источник `-` вместо файла — стандартный ввод; импорты `use` разрешаются
  от текущего каталога.
- Сокращённые даты: `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM`, `+DURATION`
  (для `--end` опорной точкой служит `--start`). Поле `start_time` в файлах —
  только строгая форма `YYYY-MM-DDTHH:MM:SS[.mmm]`.
- Коды возврата: `0` — успех; `1` — ошибка ввода, парсинга или валидации
  (текст в stderr, stdout пуст); `2` — некорректные аргументы (usage в stderr).
- Нечитаемый файл — `cannot read 'FILE': …`, код возврата `1`.
