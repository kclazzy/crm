# CRM

CRM-система с таск-трекером и мессенджером для небольшой команды (5 человек): Rust (axum + SQLite) + Tauri + React.

- [Техническое задание v2](docs/TZ.md)
- [Замечания к исходному ТЗ](docs/REVIEW.md)

## Состояние

Этап 0 (каркас) в работе:

- [x] `crm-core` — бизнес-правила: индикатор дедлайна, рабочее время и тихие часы, расписание напоминаний, ротация бэкапов, оценка диска
- [x] `crm-server` — конфиг, SQLite (WAL) с миграциями, `/api/health`
- [x] вход: первый владелец (только с машины хоста), приглашения сотрудников, логин, сессии, выход, отзыв сессий, сброс доступа, защита от подбора пароля
- [ ] HTTPS (собственный CA), установка как системная служба
- [ ] веб-интерфейс, `crm-desktop` (Tauri) с работой в трее

## Запуск для разработки

Нужен Rust (stable).

```sh
cargo test --workspace          # все тесты
cargo run -p crm-server -- --config dev.toml
```

Сервер по умолчанию слушает только `127.0.0.1:8443` (пока нет HTTPS). Первый владелец создаётся запросом с этой же машины:

```sh
curl -H 'content-type: application/json' \
  -d '{"username":"owner","display_name":"Владелец","password":"очень-надёжный-пароль"}' \
  http://127.0.0.1:8443/api/setup
```

Пример `dev.toml`:

```toml
listen = "127.0.0.1:8443"

[storage]
db_dir = "./.dev/data"
attachments_dir = "./.dev/attachments"
backup_dirs = ["./.dev/backups"]
```

Проверка: `curl http://127.0.0.1:8443/api/health` → `{"status":"ok","version":"0.1.0"}`.

## Структура

```
crates/crm-core     доменные типы и чистые бизнес-правила (без I/O)
crates/crm-server   axum API, SQLite, воркеры
migrations/         схема БД (единственный источник правды)
docs/               ТЗ и ревью
```
