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
```

- Даты — короткие формы CLI (см. главу CLI): `2026-09-07`, `+1d` (для `end` — от `start`).
- Возврат `run_*` — dict, побайтово равный JSON-объекту `cyclo run` (см. главу вывода).
- Ошибки — `CycloError` с полями `code` (слаг главы ошибок; ошибка парсера — `syntax`)
  и `message`; `str` — `слаг: сообщение`, как stderr CLI. Отсутствие файла — штатный `OSError`.

Семантика — в справочнике (главы условий, вывода, ошибок); здесь только форма вызова.
