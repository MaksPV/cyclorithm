# Планы

Что задумано, но не реализовано в движке. Вики описывает целевое состояние;
здесь — список расхождений и что нужно сделать в коде.

## Cron-демон

Хост-программа поверх `next`: поллинг окна, выполнение команд из
`action_attrs`. Пример расписания задач — `examples/real/cron.cyclo`.

## Экспорт в календарь (ICS)
`--format ics` в `run`/`next`, функция `events_ics` в core, golden `examples/valid/route.ics`. Доки: `docs/reference/output.md`.

## HTTP-сервер
Крейт `cyclorithm-serve` на `tiny_http` (`POST /run`, `POST /next`, `GET /health`), Docker multi-stage. Интеграция описана в `docs/ecosystem/integrations.md:3-14`.

