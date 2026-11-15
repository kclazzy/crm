-- Сессия подтверждена кодом из приложения: только такая работает из интернета,
-- если пользователю там нужен код.
ALTER TABLE sessions ADD COLUMN mfa_ok INTEGER NOT NULL DEFAULT 0;
