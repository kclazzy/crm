-- Метка «Новое»: когда сотрудник впервые открыл назначенную ему задачу.
CREATE TABLE task_views (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    seen_at INTEGER NOT NULL,
    PRIMARY KEY (task_id, user_id)
) WITHOUT ROWID;
-- Задачи, назначенные до обновления, новыми не считаются.
INSERT OR IGNORE INTO task_views(task_id, user_id, seen_at)
SELECT task_id, user_id, CAST(strftime('%s','now') AS INTEGER) * 1000 FROM task_assignees;
