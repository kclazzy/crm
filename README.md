# CRM

CRM-система с таск-трекером и мессенджером для небольшой команды (5 человек): Rust (axum + SQLite) + Tauri + React.

- [Техническое задание v2](docs/TZ.md)
- [Замечания к исходному ТЗ](docs/REVIEW.md)

## Состояние

Этап 0 (каркас) в работе:

- [x] `crm-core` — бизнес-правила: индикатор дедлайна, рабочее время и тихие часы, расписание напоминаний, ротация бэкапов, оценка диска
- [x] `crm-server` — конфиг, SQLite (WAL) с миграциями, `/api/health`
- [x] вход: первый владелец (только с машины хоста), приглашения сотрудников, логин, сессии, выход, отзыв сессий, сброс доступа, защита от подбора пароля
- [x] HTTPS: собственный CA сервера, сертификат на все адреса ПК, автоперевыпуск при смене IP, `/api/tls/ca` для браузера
- [ ] установка как служба Windows
- [ ] веб-интерфейс, `crm-desktop` (Tauri) с работой в трее

## Запуск для разработки

Нужен Rust (stable).

```sh
cargo test --workspace          # все тесты
cargo run -p crm-server -- --config dev.toml
```

Сервер по умолчанию слушает `0.0.0.0:8443` по HTTPS. При первом запуске в папке данных создаётся `tls/ca.crt` — сертификат, которому доверяют клиенты. Без HTTPS сервер разрешено запускать только на `127.0.0.1`. Первый владелец создаётся запросом с этой же машины:

```sh
curl -H 'content-type: application/json' \
  -d '{"username":"owner","display_name":"Владелец","password":"очень-надёжный-пароль"}' \
  --cacert .dev/data/tls/ca.crt https://localhost:8443/api/setup
```

Пример `dev.toml`:

```toml
listen = "127.0.0.1:8443"

# [tls]
# enabled = false   # только для 127.0.0.1

[storage]
db_dir = "./.dev/data"
attachments_dir = "./.dev/attachments"
backup_dirs = ["./.dev/backups"]
```

Проверка: `curl --cacert .dev/data/tls/ca.crt https://localhost:8443/api/health` → `{"status":"ok","version":"0.1.0"}`.

## Структура

```
crates/crm-core     доменные типы и чистые бизнес-правила (без I/O)
crates/crm-server   axum API, SQLite, воркеры
migrations/         схема БД (единственный источник правды)
docs/               ТЗ и ревью
```
