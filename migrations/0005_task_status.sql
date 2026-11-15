-- no-transaction
-- Фиксированные статусы задачи: «Ждёт выполнения», «Выполняется», «Отложена», «Завершена».
-- Архив — вручную (кто убрал — archived_by). Ограничение CHECK в SQLite меняется
-- только пересозданием таблицы; внешние ключи на время отключаются (рецепт SQLite).
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE tasks_new (
    id             TEXT PRIMARY KEY,
    client_id      TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    parent_task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    title          TEXT NOT NULL,
    description    TEXT,
    priority       INTEGER NOT NULL CHECK (priority IN (1,2,3)),
    status         TEXT NOT NULL CHECK (status IN ('waiting','in_progress','postponed','completed')),
    created_by     TEXT NOT NULL REFERENCES users(id),
    created_at     INTEGER NOT NULL,
    deadline_at    INTEGER NOT NULL,
    completed_at   INTEGER,
    archived_at    INTEGER,
    archived_by    TEXT REFERENCES users(id),
    deleted_at     INTEGER,
    CHECK ((status = 'completed') = (completed_at IS NOT NULL)),
    CHECK (archived_at IS NULL OR status = 'completed'),
    CHECK (archived_by IS NULL OR archived_at IS NOT NULL)
);

INSERT INTO tasks_new (id, client_id, parent_task_id, title, description, priority, status,
                       created_by, created_at, deadline_at, completed_at, archived_at, archived_by, deleted_at)
SELECT id, client_id, parent_task_id, title, description, priority,
       CASE status WHEN 'new' THEN 'waiting' ELSE status END,
       created_by, created_at, deadline_at, completed_at, archived_at, NULL, deleted_at
FROM tasks;

DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

CREATE INDEX tasks_client   ON tasks(client_id);
CREATE INDEX tasks_parent   ON tasks(parent_task_id);
CREATE INDEX tasks_deadline ON tasks(deadline_at) WHERE status <> 'completed';

CREATE TRIGGER tasks_ai AFTER INSERT ON tasks BEGIN
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('task', new.id, new.title, coalesce(new.description, ''));
END;
CREATE TRIGGER tasks_au AFTER UPDATE OF title, description ON tasks BEGIN
    DELETE FROM search_index WHERE entity = 'task' AND entity_id = old.id;
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('task', new.id, new.title, coalesce(new.description, ''));
END;
CREATE TRIGGER tasks_ad AFTER DELETE ON tasks BEGIN
    DELETE FROM search_index WHERE entity = 'task' AND entity_id = old.id;
END;

-- Напоминание о сроке пишется и в чат задачи — один раз на задачу и срок.
CREATE TABLE task_alerts (
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('soon','overdue')),
    deadline_at INTEGER NOT NULL,
    PRIMARY KEY (task_id, kind, deadline_at)
);

COMMIT;
PRAGMA foreign_keys = ON;
