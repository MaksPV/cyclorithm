# Python-биндинги

Нативные биндинги ядра (PyO3, крейт `crates/cyclorithm-python`): `check`
и `run` без subprocess. Только локальная сборка, в PyPI не публикуется.

## Установка

```sh
python3 -m venv crates/cyclorithm-python/.venv
crates/cyclorithm-python/.venv/bin/pip install maturin pytest
cd crates/cyclorithm-python
.venv/bin/maturin develop --release
```

## API

```python
import cyclorithm as c

c.check_file("route.cyclo")                    # молча OK или CycloError
c.check_text(text, base_dir="examples/valid")  # use — от base_dir (по умолчанию cwd)

window = c.run_file("route.cyclo", "2026-09-07", "+1d")
window = c.run_text(text, "2026-09-07T00:00:00", "2026-09-08T00:00:00")

from datetime import date, datetime, timedelta

window = c.run_file("route.cyclo", datetime(2026, 9, 7), timedelta(days=1))
```

- Границы окна — `str` (короткие формы CLI: `2026-09-07`, `+1d`),
  `datetime` (aware — в UTC), `date` (полночь) или `timedelta` (дельта:
  для `start` — от now, для `end` — от `start`, как `+DURATION` в CLI).
  Микросекунды усекаются до миллисекунд (точность движка); чужие типы
  и отрицательный `timedelta` — `TypeError` до вызова ядра.
- Возврат `run_*` — dict, побайтово равный JSON-объекту `cyclo run` (см. главу вывода).
- Ошибки — `CycloError` с полями `code` (слаг главы ошибок; ошибка парсера — `syntax`)
  и `message`; `str` — `слаг: сообщение`, как stderr CLI. Отсутствие файла — штатный `OSError`.

## Класс Schedule

ООП-обёртка поверх функций (тот же выход и те же ошибки); текст программы
читается один раз, конструктор лёгкий и не валидирует:

```python
sched = c.Schedule("route.cyclo")
sched = c.Schedule.from_text(text, base_dir="examples/valid")
sched.check()
window = sched.run(datetime(2026, 9, 7), timedelta(days=1))
```

Семантика — в справочнике (главы условий, вывода, ошибок); здесь только форма вызова.
