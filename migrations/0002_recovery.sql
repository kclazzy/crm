-- Одноразовые коды восстановления владельца (хранятся только хеши).
CREATE TABLE recovery_codes (
    code_hash  TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    used_at    INTEGER
);
