-- Рабочая часть CRM: группа по умолчанию и индексы для прав и непрочитанных.

-- Клиент всегда принадлежит группе; в маленькой команде хватает одной.
INSERT INTO groups(id, name, created_at) VALUES ('default', 'Основная', 0);
INSERT INTO group_members(group_id, user_id) SELECT 'default', id FROM users;

CREATE INDEX task_assignees_user ON task_assignees(user_id);
CREATE INDEX clients_group ON clients(group_id);
CREATE INDEX stages_task ON task_stages(task_id, sort_order);
CREATE INDEX messages_task_id ON messages(task_id, id);

-- Новый сотрудник сразу в основной группе (владелец может перенести).
CREATE TRIGGER users_default_group AFTER INSERT ON users
WHEN EXISTS (SELECT 1 FROM groups WHERE id = 'default') BEGIN
    INSERT OR IGNORE INTO group_members(group_id, user_id) VALUES ('default', new.id);
END;
