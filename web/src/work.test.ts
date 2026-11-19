import { describe, expect, it } from 'vitest';
import { deadlineText, todayFirst, type TaskSummary } from './work';

// Среда, 18 ноября 2026, 10:00 по местному времени.
const now = new Date(2026, 10, 18, 10, 0).getTime();
const at = (d: number, h: number, m = 0) => new Date(2026, 10, d, h, m).getTime();

describe('срок словами', () => {
  it('сегодня и завтра — со временем', () => {
    expect(deadlineText(at(18, 15), 'due_soon', now)).toBe('Сегодня до 15:00');
    expect(deadlineText(at(19, 9, 30), 'ok', now)).toBe('Завтра до 09:30');
  });
  it('на этой неделе — через сколько дней', () => {
    expect(deadlineText(at(21, 18), 'ok', now)).toBe('Через 3 дня');
    expect(deadlineText(at(24, 18), 'ok', now)).toBe('Через 6 дней');
  });
  it('дальше — дата', () => {
    expect(deadlineText(at(30, 18), 'ok', now)).toBe('До 30 нояб.');
  });
  it('меньше часа — минуты; просрочено — на сколько', () => {
    expect(deadlineText(at(18, 10, 40), 'due_soon', now)).toBe('Осталось 40 мин');
    expect(deadlineText(at(18, 8), 'overdue', now)).toBe('Просрочено на 2 ч');
    expect(deadlineText(at(15, 10), 'overdue', now)).toBe('Просрочено на 3 дня');
  });
  it('завершённая', () => {
    expect(deadlineText(at(18, 15), 'done', now)).toBe('Завершена');
  });
});

const task = (id: string, o: Partial<TaskSummary>): TaskSummary => ({
  id, client_id: 'c', client_name: 'К', parent_id: null, title: id, access: 'participant', status: 'waiting', deadline_state: 'ok', ...o,
});

describe('«Сегодня» сверху', () => {
  it('просроченные, срок сегодня и новые — в «Сегодня», остальные дальше', () => {
    const { today, later } = todayFirst([
      task('потом', { deadline_at: at(25, 12) }),
      task('просрочена', { deadline_at: at(17, 12), deadline_state: 'overdue' }),
      task('сегодня', { deadline_at: at(18, 23, 59) }),
      task('новая', { deadline_at: at(28, 12), is_new: true }),
      task('завтра', { deadline_at: at(19, 0, 1) }),
    ], now);
    expect(today.map((t) => t.id)).toEqual(['просрочена', 'сегодня', 'новая']);
    expect(later.map((t) => t.id)).toEqual(['потом', 'завтра']);
  });
});
