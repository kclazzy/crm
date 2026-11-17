-- Короткие коды подключения программы в офисе (crm-core::enroll).
-- Секрет хранится как есть: он нужен серверу для обмена SPAKE2; код живёт
-- 72 часа и сгорает после 10 попыток.
CREATE TABLE enroll_codes (
    id         TEXT PRIMARY KEY,       -- открытая часть кода: 4 символа
    secret     TEXT NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX enroll_codes_user ON enroll_codes(user_id);
