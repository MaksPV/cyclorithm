// Smoke-тест C/C++ биндинга (слой вызовов, не семантика движка).
// Сборка: cargo build -p cyclorithm-cpp &&
//   g++ -std=c++17 -I include tests/smoke.cpp target/debug/libcyclorithm.a \
//     -ldl -lpthread -lm -o /tmp/opencode/smoke
// Запуск из crates/cyclorithm-cpp (пути к примерам относительные).
#include <cassert>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>

#include "cyclorithm.h"
#include "cyclorithm.hpp"

namespace {

std::string read_file(const std::string& path) {
  std::ifstream f(path);
  assert(f && "example file must exist");
  std::ostringstream ss;
  ss << f.rdbuf();
  return ss.str();
}

int count(const std::string& haystack, const std::string& needle) {
  int n = 0;
  for (size_t p = 0; (p = haystack.find(needle, p)) != std::string::npos; ++p) {
    ++n;
  }
  return n;
}

void test_version() {
  const std::string v = cyclo_version();
  assert(!v.empty());
  std::cout << "version: " << v << "\n";
}

void test_c_check_ok() {
  const std::string text = read_file("../../examples/valid/route.cyclo");
  const cyclo_result_t r =
      cyclo_check(text.c_str(), "../../examples/valid");
  assert(r.json != nullptr);
  std::string out(r.json);
  cyclo_result_free(r);
  assert(out == "{\"ok\":true}");
}

void test_c_run_matches_cli_shape() {
  const std::string text = read_file("../../examples/valid/route.cyclo");
  const cyclo_result_t r = cyclo_run(
      text.c_str(), "../../examples/valid", "2026-09-07T00:00:00",
      "2026-09-08T00:00:00");
  assert(r.json != nullptr);
  std::string out(r.json);
  cyclo_result_free(r);
  // 18 событий за окно (как в CLI), имя расписания в конверте.
  assert(count(out, "\"time\"") == 18);
  assert(out.find("Автобусный парк") != std::string::npos);
}

void test_c_error_has_slug() {
  const std::string text =
      read_file("../../examples/invalid/bad_cycle-overruns-repeat.cyclo");
  const cyclo_result_t r = cyclo_run(
      text.c_str(), "../../examples/invalid", "2026-01-01", "2026-01-02");
  assert(r.json == nullptr);
  std::string code(r.code);
  std::string message(r.message);
  cyclo_result_free(r);
  assert(code == "cycle-overruns");
  assert(message.find("overruns") != std::string::npos);
}

void test_cpp_class() {
  cyclo::Schedule sched("../../examples/valid/route.cyclo");
  sched.check();
  const std::string out =
      sched.run("2026-09-07T00:00:00", "2026-09-08T00:00:00");
  assert(count(out, "\"time\"") == 18);
  try {
    cyclo::Schedule bad(
        "../../examples/invalid/bad_cycle-overruns-repeat.cyclo");
    bad.run("2026-01-01", "2026-01-02");
    assert(false && "must throw");
  } catch (const cyclo::Error& e) {
    assert(e.code == "cycle-overruns");
  }
  try {
    cyclo::Schedule missing("../../examples/invalid/nope.cyclo");
    assert(false && "must throw");
  } catch (const std::runtime_error& e) {
    assert(std::string(e.what()).find("cannot read") != std::string::npos);
  }
}

}  // namespace

int main() {
  test_version();
  test_c_check_ok();
  test_c_run_matches_cli_shape();
  test_c_error_has_slug();
  test_cpp_class();
  std::cout << "smoke: ok\n";
  return 0;
}
