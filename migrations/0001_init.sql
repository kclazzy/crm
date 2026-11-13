-- Начальная схема CRM. Источник правды для схемы БД (ТЗ §6).
-- Времена — INTEGER, Unix-миллисекунды UTC. ID — UUIDv7 в TEXT.

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
    display_name  TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('host','manager','employee')),
    is_active     INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0,1)),
    timezone      TEXT,
    notify_always INTEGER NOT NULL DEFAULT 0 CHECK (notify_always IN (0,1)), -- игнорировать тихие часы
    created_at    INTEGER NOT NULL
);

CREATE TABLE groups (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE TABLE group_members (
    group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id  TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);

CREATE TABLE invites (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    expires_at  INTEGER NOT NULL,
    used_at     INTEGER
);

CREATE TABLE sessions (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash    TEXT NOT NULL UNIQUE,
    device_id     TEXT NOT NULL,
    device_name   TEXT NOT NULL,
    ip            TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    revoked_at    INTEGER
);

CREATE TABLE clients (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL CHECK (kind IN ('person','company')),
    name         TEXT NOT NULL,
    phone        TEXT,
    email        TEXT,
    description  TEXT NOT NULL DEFAULT '',     -- Markdown: описание клиента, реквизиты, адрес
    group_id     TEXT NOT NULL REFERENCES groups(id),
    manager_id   TEXT REFERENCES users(id),
    import_batch_id TEXT REFERENCES import_batches(id) ON DELETE SET NULL,
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   INTEGER NOT NULL,
    deleted_at   INTEGER                       -- корзина
);

CREATE TABLE import_batches (
    id          TEXT PRIMARY KEY,
    file_name   TEXT NOT NULL,
    created_by  TEXT NOT NULL REFERENCES users(id),
    created_at  INTEGER NOT NULL,
    stats       TEXT NOT NULL DEFAULT '{}',    -- JSON: создано/обновлено/пропущено
    undone_at   INTEGER
);

CREATE TABLE holidays (
    day  TEXT PRIMARY KEY,                     -- 'YYYY-MM-DD' в часовом поясе организации
    name TEXT
);

CREATE TABLE tags (
    id    TEXT PRIMARY KEY,
    name  TEXT NOT NULL UNIQUE COLLATE NOCASE,
    color TEXT NOT NULL
);

CREATE TABLE client_tags (
    client_id TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    tag_id    TEXT NOT NULL REFERENCES tags(id)    ON DELETE CASCADE,
    PRIMARY KEY (client_id, tag_id)
);

CREATE TABLE tasks (
    id             TEXT PRIMARY KEY,
    client_id      TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    parent_task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    title          TEXT NOT NULL,
    description    TEXT,
    priority       INTEGER NOT NULL CHECK (priority IN (1,2,3)),
    status         TEXT NOT NULL CHECK (status IN ('new','in_progress','completed')),
    created_by     TEXT NOT NULL REFERENCES users(id),
    created_at     INTEGER NOT NULL,
    deadline_at    INTEGER NOT NULL,
    completed_at   INTEGER,
    archived_at    INTEGER,
    deleted_at     INTEGER,
    CHECK ((status = 'completed') = (completed_at IS NOT NULL)),
    CHECK (archived_at IS NULL OR status = 'completed')
);
CREATE INDEX tasks_client   ON tasks(client_id);
CREATE INDEX tasks_parent   ON tasks(parent_task_id);
CREATE INDEX tasks_deadline ON tasks(deadline_at) WHERE status <> 'completed';

CREATE TABLE task_assignees (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, user_id)
);

CREATE TABLE task_stages (
    id           TEXT PRIMARY KEY,
    task_id      TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    completed_at INTEGER,
    completed_by TEXT REFERENCES users(id),
    sort_order   INTEGER NOT NULL
);

CREATE TABLE attachments (
    sha256     TEXT PRIMARY KEY,
    size_bytes INTEGER NOT NULL,
    mime       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE messages (
    id          TEXT PRIMARY KEY,
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    sender_id   TEXT NOT NULL REFERENCES users(id),
    kind        TEXT NOT NULL CHECK (kind IN ('user','system')),
    content     TEXT NOT NULL DEFAULT '',
    reply_to_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL,
    edited_at   INTEGER
);
CREATE INDEX messages_task ON messages(task_id, created_at);

CREATE TABLE message_attachments (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    sha256     TEXT NOT NULL REFERENCES attachments(sha256),
    file_name  TEXT NOT NULL,
    PRIMARY KEY (message_id, sha256)
);

CREATE TABLE chat_reads (
    user_id              TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    task_id              TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    last_read_message_id TEXT NOT NULL,
    PRIMARY KEY (user_id, task_id)
);

CREATE TABLE notifications (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    task_id    TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    read_at    INTEGER
);
CREATE INDEX notifications_unread ON notifications(user_id) WHERE read_at IS NULL;

CREATE TABLE reminders_sent (
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('h24','h4','h1','overdue')),
    deadline_at INTEGER NOT NULL,
    sent_at     INTEGER NOT NULL,
    PRIMARY KEY (task_id, user_id, kind, deadline_at)
);

CREATE TABLE audit_log (
    id         TEXT PRIMARY KEY,
    actor_id   TEXT REFERENCES users(id),
    action     TEXT NOT NULL,       -- 'task.status_changed', 'login.failed', ...
    entity     TEXT,
    entity_id  TEXT,
    details    TEXT NOT NULL DEFAULT '{}',
    ip         TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE disk_usage_snapshots (
    taken_at   INTEGER NOT NULL,
    volume_id  TEXT NOT NULL,          -- идентификатор тома (точка монтирования)
    total_bytes INTEGER NOT NULL,
    free_bytes  INTEGER NOT NULL,
    crm_bytes   TEXT NOT NULL,         -- JSON: {"db":…, "fts":…, "attachments":…, "backups_hourly":…, ...}
    PRIMARY KEY (volume_id, taken_at)
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Поиск (trigram — подстрока, кириллица, без учёта регистра)
CREATE VIRTUAL TABLE search_index USING fts5(
    entity UNINDEXED, entity_id UNINDEXED, title, body,
    tokenize = 'trigram'
);

-- Триггеры индекса поиска. Удалённые в корзину отфильтровываются при запросе.
CREATE TRIGGER clients_ai AFTER INSERT ON clients BEGIN
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('client', new.id, new.name, new.description);
END;
CREATE TRIGGER clients_au AFTER UPDATE OF name, description ON clients BEGIN
    DELETE FROM search_index WHERE entity = 'client' AND entity_id = old.id;
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('client', new.id, new.name, new.description);
END;
CREATE TRIGGER clients_ad AFTER DELETE ON clients BEGIN
    DELETE FROM search_index WHERE entity = 'client' AND entity_id = old.id;
END;

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

CREATE TRIGGER stages_ai AFTER INSERT ON task_stages BEGIN
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('stage', new.id, new.title, '');
END;
CREATE TRIGGER stages_au AFTER UPDATE OF title ON task_stages BEGIN
    DELETE FROM search_index WHERE entity = 'stage' AND entity_id = old.id;
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('stage', new.id, new.title, '');
END;
CREATE TRIGGER stages_ad AFTER DELETE ON task_stages BEGIN
    DELETE FROM search_index WHERE entity = 'stage' AND entity_id = old.id;
END;

CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('message', new.id, '', new.content);
END;
CREATE TRIGGER messages_au AFTER UPDATE OF content ON messages BEGIN
    DELETE FROM search_index WHERE entity = 'message' AND entity_id = old.id;
    INSERT INTO search_index(entity, entity_id, title, body) VALUES ('message', new.id, '', new.content);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    DELETE FROM search_index WHERE entity = 'message' AND entity_id = old.id;
END;
