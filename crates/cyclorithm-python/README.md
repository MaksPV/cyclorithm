# cyclorithm (Python)

Нативные биндинги движка расписаний Cyclorithm (PyO3).

Канон — вики репозитория (`docs/`, страница `docs/ecosystem/python.md`).
Установка для разработки из корня репозитория:

```sh
python3 -m venv crates/cyclorithm-python/.venv
crates/cyclorithm-python/.venv/bin/pip install maturin pytest
cd crates/cyclorithm-python && ../.venv/bin/maturin develop --release
```
