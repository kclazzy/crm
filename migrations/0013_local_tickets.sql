-- Одноразовые билеты входа владельца на самом сервере (без пароля):
-- сразу после создания владельца и по подтверждению администратора Windows.
CREATE TABLE local_tickets (
    hash       TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
