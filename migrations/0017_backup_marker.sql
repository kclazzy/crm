-- Счётчик изменений рабочих данных: копия по расписанию не делается, если с прошлой
-- копии ничего не менялось (ночь, выходные). Триггеры на таблицы ставит сервер при
-- запуске — так новые таблицы попадают под счёт сами.
CREATE TABLE backup_marker (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    n  INTEGER NOT NULL DEFAULT 0
);
INSERT INTO backup_marker (id, n) VALUES (1, 0);
