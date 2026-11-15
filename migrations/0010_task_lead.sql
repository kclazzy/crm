-- Ответственный исполнитель: первый выбранный; остальные — соисполнители.
ALTER TABLE task_assignees ADD COLUMN is_lead INTEGER NOT NULL DEFAULT 0;
UPDATE task_assignees SET is_lead = 1
WHERE rowid IN (SELECT min(rowid) FROM task_assignees GROUP BY task_id);
