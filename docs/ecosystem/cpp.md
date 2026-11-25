# C++-библиотека

Нативный C ABI поверх ядра (крейт `crates/cyclorithm-cpp`): shared-библиотека
`libcyclorithm` + заголовки `include/cyclorithm.h` (C API) и
`include/cyclorithm.hpp` (класс `cyclo::Schedule`, header-only).
В релизах — архив `cyclo-cpp-<версия>-<триплет>.tar.gz` рядом с бинарниками
и Python-колёсами.

## Сборка из исходников

Нужен только стабильный Rust (`rustup`). Команды — из корня репо:

```sh
cargo build --release -p cyclorithm-cpp
# lib: target/release/libcyclorithm.so (.dll на Windows),
# заголовки: crates/cyclorithm-cpp/include/
```

Проверка:

```sh
cd crates/cyclorithm-cpp
g++ -std=c++17 -I include tests/smoke.cpp ../../target/debug/libcyclorithm.a -ldl -lpthread -lm -o /tmp/smoke
/tmp/smoke
```

Линковка своей программы с shared-библиотекой:

```sh
g++ -std=c++17 -I <путь к include> prog.cpp -L <путь к lib> -lcyclorithm -o prog
```

## API

```cpp
#include "cyclorithm.hpp"

cyclo::Schedule sched("route.cyclo");  // use — от директории файла
sched.check();                          // молча OK или cyclo::Error
std::string json =
    sched.run("2026-09-07", "+1d");     // JSON-объект как в `cyclo run`
```

- C API (`cyclorithm.h`): `cyclo_check` / `cyclo_run` возвращают
  `cyclo_result_t` — успех (`json != NULL`) или слаг ошибки (`code`)
  с текстом (`message`, парсер — `syntax`); всё не-NULL освобождается
  одним `cyclo_result_free`. Даты — короткие формы CLI (см. главу CLI).
- Ошибки C++: `cyclo::Error : std::runtime_error` с полем `code`
  (слаг главы ошибок); `what()` — `слаг: сообщение`, как stderr CLI.
  Отсутствие файла — `std::runtime_error`.
- Возврат `run` — строка JSON-объекта `cyclo run` (см. главу вывода).

## Разбор JSON

Свой парсер библиотека не везёт: окно — обычная JSON-строка, разбирайте
чем удобно (ниже — системный `nlohmann/json`, проверено сборкой):

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
g++ -std=c++17 -I <путь к include> prog.cpp -L <путь к lib> -lcyclorithm -o prog
```

Семантика — в справочнике (главы условий, вывода, ошибок); здесь только форма вызова.
