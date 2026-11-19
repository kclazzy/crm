-- «Удалить» в колокольчике и журнале безопасности скрывает запись, а не стирает:
-- блокировка за подбор пароля и защита от повторных тревог считают и скрытые.
ALTER TABLE notifications ADD COLUMN hidden_at INTEGER;
ALTER TABLE audit_log ADD COLUMN hidden_at INTEGER;
