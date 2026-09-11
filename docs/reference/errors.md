# Ошибки

Формат: текст ошибки — в stderr, stdout остаётся пустым. Коды возврата:
`0` — успех, `1` — ошибка ввода, парсинга или валидации,
`2` — некорректные аргументы CLI (текст usage).

Каждая ошибка ниже названа по ситуации, а не по номеру: имя стабильно,
новые ситуации вставляются без перенумерации. Тексты сообщений дословные.
В движке пока живут коды `E01–E16` из legacy-спеки — переименование в коде
позже; на поведение это не влияет, пользователь кодов не видит.

Ошибки синтаксиса (файл не разбирается по грамматике, включая отсутствие
корневого цикла) возвращают код `1` без имени ситуации. Импорты
(`cannot-read-import`, `schedule-in-import`) разрешаются раньше проверок
решётки. Фазы валидации идут в фиксированном порядке, первая ошибка
побеждает: имена → рекурсия → таблицы → длительности и границы → условия →
даты. Внутри фазы проверки идут в порядке объявления.

## Имена

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `unknown-point` | `unknown point 'PORT'` | Объявить точку `point` или исправить имя |
| `action-not-allowed` | `action 'arrive' not allowed for point 'DEPOT'` | Добавить действие в `actions` точки |
| `unknown-cycle` | `unknown cycle 'NIGHT_ROUTE'` | Объявить цикл `cycle`; проверить форму вызова `A()` или `A.b()` |
| `duplicate` | `duplicate point 'DEPOT'`, `duplicate routine 'M'`, `duplicate table 'T'`, `duplicate slot '1st'`, `duplicate param 'a'` | Устранить повтор имени внутри файла |
| `wrong-kind` | `point 'X' is not a cycle`, `cycle 'X' is not a point`, `point 'X' is not a routine`, `routine 'X' is not a point`, `routine 'X' is not a cycle` | Переименовать: имя занято сущностью другого рода |

## Рекурсия

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `recursive` | `recursive cycle 'A'`, `recursive routine 'M'`, `recursive table 'T'` | Разорвать цепочку вызовов |

## Таблицы

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `unknown-table` | `unknown table 'SHORT'` | Передать существующую таблицу литеральным именем |
| `unknown-slot` | `unknown slot '8th'` | Убедиться, что метка существует в таблице вызова |
| `invalid-table-argument` | `invalid table argument for 'M'` | Первый аргумент рутины — имя таблицы, не выражение |

## Длительности и границы

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `invalid-duration` | `invalid duration '1h2h'` | Порядок `w>d>h>m>s>ms`, уникальность компонентов, вместимость в `i64` |
| `cycle-overruns` | `cycle 'C2' overruns 'C1' by 20m (80m > 60m)` | Увеличить `duration` или сдвинуть строку |
| `action-overruns` | `action 'x' overruns 'R' by 1m (61m > 60m)` | Увеличить `duration` или сдвинуть строку |
| `offset-out-of-bounds` | `offset '-2h' out of bounds (duration 1h20m)` | Держать смещение внутри `[0, duration]` |
| `until-out-of-bounds` | `until '30h' out of bounds (duration 24h)` | Держать горизонт внутри `[0, duration]` |
| `invalid-repeat-count` | `invalid repeat count '0'` | `N ≥ 1` |
| `fill-zero-duration` | `fill of zero-duration cycle 'LONG'` | Повторять только циклы ненулевой длительности |
| `repeat-point-action` | `repeat of point action 'depart' not allowed` | Повторы — только для циклов |

## Условия

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `unknown-name` | `unknown name 'banana'` | Проверить параметр, объявление, `at`, системное имя; данные — только через параметр |
| `type-mismatch` | `type mismatch: cannot mix number and string` | Не смешивать числа и строки |
| `wrong-arguments` | `wrong arguments for 'pad'` | Проверить число и типы аргументов |
| `no-table-parameter` | `routine 'M' has no table parameter` | Первый параметр рутины — всегда таблица |
| `division-by-zero` | `division by zero` | Исключить нулевой делитель |
| `recursive-definition` | `recursive definition 'a'` | Разорвать рекурсию в объявлениях |
| `not-a-predicate` | `'hour' is not a predicate` | В позиции условия вызывать только предикат |
| `integer-out-of-range` | `integer out of range '...'` | Уложиться в `i64` |
| `invalid-date` | `invalid date '...'` | Проверить компоненты даты |
| `unknown-field` | `unknown field 'name'` | Проверить ключ и цепочку доступа |
| `index-out-of-bounds` | `index out of bounds '5'` | Держать индекс внутри массива |
| `maps-not-comparable` | `cannot compare maps or arrays` | Сравнивать только числа и строки |

## Импорты

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `cannot-read-import` | `cannot read import '...'` | Путь от директории файла; исключить циклические импорты |
| `import-cycle` | `import cycle '...'` | Разорвать цикл импортов |
| `schedule-in-import` | `schedule not allowed in import '...'` | Оставить в библиотеке только объявления |

## Атрибуты

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `duplicate-attribute` | `duplicate attribute 'a'` | Устранить повтор ключа в словаре или атрибутах действия |

## Даты

| Ситуация | Сообщение | Исправление |
|----------|-----------|-------------|
| `invalid-datetime` | `invalid datetime '...'` | В файле — строгая форма `YYYY-MM-DDTHH:MM:SS[.mmm]` |

Негативные примеры: `examples/invalid/bad_*.cyclo` — не менее одного файла
на ситуацию (имена файлов пока со старыми кодами `bad_eXX` — переименуются
вместе с движком). Правило проекта: новая ситуация требует пример, строку
в таблице и тест.
