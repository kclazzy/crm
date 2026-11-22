-- Когда в этой сессии последний раз вводили код из приложения:
-- владелец может просить код заново раз в N дней при работе из интернета.
ALTER TABLE sessions ADD COLUMN mfa_at INTEGER;
UPDATE sessions SET mfa_at = last_seen_at WHERE mfa_ok = 1;
