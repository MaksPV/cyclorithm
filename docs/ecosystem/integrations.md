# Интеграции

## HTTP-сервер (`feature/serve`)

Крейт `cyclorithm-serve` на `tiny_http`, без состояния (исходник передаётся
в теле запроса):

- `POST /run` — `{"source","start","end","libs?"}` → объект как у команды `run`.
- `POST /next` — `{"source","from?","within?","n?"}` → объект `next_steps`.
- `GET /health` → `ok`. Ошибка движка — тело `{error}` и HTTP 422.
- Конфигурация: `CYCLO_PORT` (по умолчанию 8080). Docker: многостадийная
  сборка, непривилегированный пользователь, `HEALTHCHECK` на `/health`,
  compose с опросом `next`.

## Календари (ICS)

Флаг `--format ics` для `run`/`next`: события — записи `VEVENT` (`UID`,
`DTSTART-Z`, `SUMMARY: action @ point`). Детали — в
[формате вывода](../reference/output.md).

## WASM-плейграунд

Отдельный репозиторий `cyclorithm-playground`: парсер и ядро выполняются
в браузере, фасад — `run_schedule(src, start, end, libs)`, интерфейс —
таймлайн и таблица. Ошибки валидации позиций не несут — визуально
подсвечивается только синтаксис.
