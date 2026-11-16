-- Устройства, подключающиеся по ключу сервера не из сети офиса.
-- Пока владелец не одобрил устройство, ему не показывается даже страница входа.
CREATE TABLE p2p_devices (
    id          TEXT PRIMARY KEY,          -- публичный ключ устройства
    name        TEXT NOT NULL DEFAULT '',  -- подпись владельца: «Ноутбук Ивана»
    approved    INTEGER NOT NULL DEFAULT 0,
    first_seen  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
);
