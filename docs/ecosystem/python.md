# Библиотека Python

Пакет `cyclorithm` вызывает ядро через PyO3 в том же процессе Python.
Пакет можно установить из файла `.whl` на странице релизов или собрать
из исходников в `crates/cyclorithm-python`.

## Установка

Скачайте `cyclorithm-*-abi3-*.whl` для своей платформы со
[страницы релизов](https://github.com/MaksPV/cyclorithm/releases) и
установите пакет в виртуальное окружение. Готовая сборка подходит для
CPython 3.10 и новее. Для установки Rust не требуется.

```sh
python3 -m venv .venv
.venv/bin/pip install cyclorithm-0.3.2-cp310-abi3-manylinux_2_34_x86_64.whl
.venv/bin/python -c "import cyclorithm; print(cyclorithm.__version__)"
```

## Сборка для разработки

Для разработки нужны стабильная версия Rust (`rustup`) и Python 3.10
или новее. Выполните команды из корня репозитория:

```sh
python3 -m venv crates/cyclorithm-python/.venv
crates/cyclorithm-python/.venv/bin/pip install maturin pytest ruff
cd crates/cyclorithm-python
.venv/bin/maturin develop --release
```

Чтобы проверить установленный в виртуальное окружение пакет, выполните:

```sh
.venv/bin/python -c "import cyclorithm; print(cyclorithm.__version__)"
.venv/bin/python -m pytest python/tests/ -q
.venv/bin/ruff check python/
```

## API

```python
import cyclorithm as c

c.check_file("route.cyclo")                    # при ошибке возбуждает CycloError
c.check_text(text, base_dir="examples/valid")  # пути use отсчитываются от base_dir

window = c.run_file("route.cyclo", "2026-09-07", "+1d")
window = c.run_text(text, "2026-09-07T00:00:00", "2026-09-08T00:00:00")

from datetime import date, datetime, timedelta

window = c.run_file("route.cyclo", datetime(2026, 9, 7), timedelta(days=1))
```

Границы окна можно передать в виде строки, `datetime`, `date` или `timedelta`:

- Строка использует те же форматы, что и [CLI](../reference/cli.md), например
  `"2026-09-07"` или `"+1d"`.
- `datetime` сохраняет местное время и, если есть, смещение от UTC.
  Микросекунды отбрасываются после миллисекунд.
- `date` означает полночь указанного дня.
- `timedelta` задаёт длительность. Для `start` она отсчитывается от текущего
  момента с системным смещением от UTC, для `end` — от `start`.

Отрицательный `timedelta` и неподдерживаемые типы вызывают `TypeError`.
Правила расчёта времени описаны в [семантике](../reference/semantics.md#time-zones).

Функции `run_*` возвращают словарь с теми же данными, что и
[JSON-вывод `cyclo run`](../reference/output.md). При ошибке программы
библиотека возбуждает `CycloError` с полями `code` и `message`.
Для ошибки разбора `code` равен `syntax`. Преобразование исключения в
строку даёт `код: сообщение`. Если файл недоступен, возникает `OSError`.

## Класс Schedule

`Schedule` хранит текст программы и позволяет проверять или рассчитывать
её несколько раз. Конструктор читает текст один раз. Для проверки
вызовите `check()`:

```python
sched = c.Schedule("route.cyclo")
sched = c.Schedule.from_text(text, base_dir="examples/valid")
sched.check()
window = sched.run(datetime(2026, 9, 7), timedelta(days=1))
```

Класс возвращает те же данные и исключения, что функции модуля.
