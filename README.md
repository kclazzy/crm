# CRM

CRM-система с таск-трекером и мессенджером для небольшой команды (5 человек): Rust (axum + SQLite) + Tauri + React.

- [Техническое задание v2](docs/TZ.md)
- [Замечания к исходному ТЗ](docs/REVIEW.md)

## Состояние

Этап 0 (каркас) в работе:

- [x] `crm-core` — бизнес-правила: индикатор дедлайна, рабочее время и тихие часы, расписание напоминаний, ротация бэкапов, оценка диска
- [x] `crm-server` — конфиг, SQLite (WAL) с миграциями, `/api/health`
- [ ] HTTPS (собственный CA), установка как системная служба
- [ ] веб-интерфейс, `crm-desktop` (Tauri)

## Запуск для разработки

Нужен Rust (stable).

```sh
cargo test --workspace          # все тесты
cargo run -p crm-server -- --config dev.toml
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
