# C++-библиотека

Ядро доступно из C и C++ через динамическую библиотеку `libcyclorithm`.
Интерфейс C описан в `include/cyclorithm.h`, а класс C++ `cyclo::Schedule`
целиком определён в `include/cyclorithm.hpp`. Библиотека и заголовки
поставляются в архиве релиза `cyclo-cpp-<версия>-<триплет>.tar.gz`.

## Сборка из исходников

Для сборки библиотеки нужна стабильная версия Rust (`rustup`). Выполните команду из корня репозитория:

```sh
cargo build --release -p cyclorithm-cpp
# lib: target/release/libcyclorithm.so (.dll на Windows),
# заголовки: crates/cyclorithm-cpp/include/
```

Чтобы собрать и запустить проверочный пример, понадобится компилятор C++:

```sh
cd crates/cyclorithm-cpp
g++ -std=c++17 -I include tests/smoke.cpp ../../target/release/libcyclorithm.a -ldl -lpthread -lm -o /tmp/smoke
/tmp/smoke
```

Чтобы связать свою программу с динамической библиотекой, выполните:

```sh
g++ -std=c++17 -I path/to/include prog.cpp -L path/to/lib -lcyclorithm -o prog
```

## API

```cpp
#include "cyclorithm.hpp"

cyclo::Schedule sched("route.cyclo");  // use — от директории файла
sched.check();                          // молча OK или cyclo::Error
std::string json =
    sched.run("2026-09-07", "+1d");     // JSON-объект как в `cyclo run`
```

- Функции C `cyclo_check` и `cyclo_run` возвращают `cyclo_result_t`.
  При успехе заполнено поле `json`, при ошибке — `code` и `message`.
  Один вызов `cyclo_result_free` освобождает все заполненные поля результата.
  Для дат используются [форматы CLI](../reference/cli.md).
- В C++ ошибки программы вызывают исключение `cyclo::Error`, унаследованное
  от `std::runtime_error`. Поле `code` содержит код ошибки, а `what()`
  возвращает строку `код: сообщение`. Для ошибки разбора код равен
  `syntax`. Если файл недоступен, возникает `std::runtime_error`.
- Метод `run` возвращает строку с [JSON-объектом `cyclo run`](../reference/output.md).

## Разбор JSON

Для разбора строки с результатом используйте любую библиотеку JSON.
В примере используется `nlohmann/json`:

```cpp
#include <iostream>
#include <nlohmann/json.hpp>
#include "cyclorithm.hpp"

int main() {
  cyclo::Schedule sched("route.cyclo");
  sched.check();
  const nlohmann::json w =
      nlohmann::json::parse(sched.run("2026-09-07", "+1d"));
  for (const auto& e : w["events"]) {
    std::cout << e["time"].get<std::string>() << " "
              << e["point"].get<std::string>() << "."
              << e["action"].get<std::string>() << "\n";
  }
}
```

```sh
g++ -std=c++17 -I path/to/include prog.cpp -L path/to/lib -lcyclorithm -o prog
```
