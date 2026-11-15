-- Второй фактор: код из приложения-аутентификатора (TOTP) и резервные коды.
ALTER TABLE users ADD COLUMN totp_secret TEXT;
-- Секрет, который показали, но ещё не подтвердили кодом.
ALTER TABLE users ADD COLUMN totp_pending TEXT;
-- Последний принятый интервал: один код дважды не принимается.
ALTER TABLE users ADD COLUMN totp_last_step INTEGER;

CREATE TABLE totp_backup (
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used_at   INTEGER,
    PRIMARY KEY (user_id, code_hash)
);

-- Вход разрешён, но до настройки кода доступен только профиль.
ALTER TABLE sessions ADD COLUMN needs_2fa_setup INTEGER NOT NULL DEFAULT 0;
