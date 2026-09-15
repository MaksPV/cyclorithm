// C++-обёртка над C API Cyclorithm (header-only, C++17).
//
// Тонкий класс поверх функций (тот же выход и те же ошибки, что у C API);
// конструктор лёгкий и не валидирует — ошибка только в check()/run().
#pragma once

#include <fstream>
#include <sstream>
#include <stdexcept>
#include <string>

#include "cyclorithm.h"

namespace cyclo {

// Ошибка движка: code — слаг главы ошибок (парсер — "syntax"),
// what() — "слаг: сообщение", как stderr CLI.
class Error : public std::runtime_error {
 public:
  std::string code;
  Error(std::string code_, const std::string& message)
      : std::runtime_error(code_ + ": " + message), code(std::move(code_)) {}
};

class Schedule {
 public:
  // Программа из файла; use — от директории файла.
  // Отсутствие файла — std::runtime_error (не Error движка).
  explicit Schedule(const std::string& path) {
    std::ifstream f(path);
    if (!f) {
      throw std::runtime_error("cannot read '" + path + "'");
    }
    std::ostringstream ss;
    ss << f.rdbuf();
    text_ = ss.str();
    const auto slash = path.find_last_of("/\\");
    base_ = slash == std::string::npos ? "" : path.substr(0, slash);
  }

  // Программа строкой; use — от base_dir (по умолчанию cwd, как stdin в CLI).
  static Schedule from_text(
      const std::string& text, const std::string& base_dir = "") {
    Schedule s;
    s.text_ = text;
    s.base_ = base_dir;
    return s;
  }

  // Валидация: молча OK или cyclo::Error.
  void check() const {
    const cyclo_result_t r = cyclo_check(text_.c_str(), base_.c_str());
    if (r.json == nullptr) {
      throw_if_error(r);
    }
    cyclo_result_free(r);
  }

  // Окно событий: JSON-строка объекта `cyclo run`.
  // Даты — короткие формы CLI ("2026-09-07", "+1d").
  std::string run(const std::string& start, const std::string& end) const {
    const cyclo_result_t r =
        cyclo_run(text_.c_str(), base_.c_str(), start.c_str(), end.c_str());
    if (r.json == nullptr) {
      throw_if_error(r);
    }
    std::string out(r.json);
    cyclo_result_free(r);
    return out;
  }

 private:
  Schedule() = default;
  std::string text_;
  std::string base_;

  // Копирует code/message из результата, освобождает его и бросает Error.
  // Только для результатов с json == nullptr.
  static void throw_if_error(cyclo_result_t r) {
    std::string code(r.code != nullptr ? r.code : "syntax");
    std::string message(r.message != nullptr ? r.message : "");
    cyclo_result_free(r);
    throw Error(std::move(code), message);
  }
};

}  // namespace cyclo
