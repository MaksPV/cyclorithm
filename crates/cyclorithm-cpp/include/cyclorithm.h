/* C API движка расписаний Cyclorithm.
 *
 * Граница — строки UTF-8 с NUL-терминатором (должны быть не-NULL).
 * Окно — JSON-объект как в `cyclo run` (см. docs/reference/output.md).
 * Ошибки — слаг главы ошибок (`cycle-overruns`, …; парсер — `syntax`).
 * Контракт — docs/ecosystem/cpp.md.
 */
#ifndef CYCLORITHM_H
#define CYCLORITHM_H

#ifdef __cplusplus
extern "C" {
#endif

/* Результат вызова: успех — json != NULL (объект как в `cyclo run`);
 * ошибка — json == NULL, code/message заполнены.
 * Все не-NULL поля освобождаются одним cyclo_result_free. */
typedef struct {
  char* json;
  char* code;
  char* message;
} cyclo_result_t;

/* Валидация текста программы (base — директория для `use`).
 * Успех — {"ok":true} в json. */
cyclo_result_t cyclo_check(const char* text, const char* base);

/* Окно событий. Даты — короткие формы CLI (см. docs/reference/cli.md):
 * "2026-09-07", "+1d" (для end — от start). */
cyclo_result_t cyclo_run(
    const char* text, const char* base, const char* start, const char* end);

/* Освобождение всех не-NULL полей результата (и только их). */
void cyclo_result_free(cyclo_result_t r);

/* Версия ядра (синхронна с версией workspace).
 * Статическая строка, освобождать не нужно. */
const char* cyclo_version(void);

#ifdef __cplusplus
}
#endif

#endif /* CYCLORITHM_H */
