-- Устройства сотрудников в VPN WireGuard. Закрытые ключи устройств не хранятся.
CREATE TABLE vpn_peers (
    id           TEXT PRIMARY KEY,
    user_id      TEXT REFERENCES users(id) ON DELETE SET NULL,
    name         TEXT NOT NULL,
    public_key   TEXT NOT NULL UNIQUE,
    address      TEXT NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER
);
