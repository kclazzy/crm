import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { ArchiveRestore, Building2, CalendarDays, ChevronLeft, ChevronRight, List, Plus, Search, Trash2, X } from 'lucide-react';
import { useAuth } from '../auth';
import { useLive, useLiveReload } from '../live';
import { childrenOf, dateTime, type TaskSummary, work } from '../work';
import { Button, ErrorText, Modal } from '../components/ui';
import { TaskForm, TaskRow, deadlineBar, deadlineLegend } from '../components/work';

export { TaskPage } from './TaskPage';

type Role = 'assignee' | 'creator' | 'involved';

const roles = [
  { v: '', label: 'Все' },
  { v: 'assignee', label: 'Исполняю' },
  { v: 'creator', label: 'Поставил(а)' },
] as const;

function count(n: number) {
  return n > 99 ? '99+' : String(n);
}

/** Шапка раздела задач: заголовок, «Создать», роль, поиск; ниже — вид и быстрые фильтры. */
function Header({ title, baseRole, onCreate }: { title: string; baseRole?: Role; onCreate?: () => void }) {
  const [params, setParams] = useSearchParams();
  const { unread, refreshUnread } = useLive();
  const [overdueCount, setOverdueCount] = useState(0);
  const [q, setQ] = useState(params.get('q') ?? '');
  const view = params.get('view') === 'calendar' ? 'calendar' : params.get('view') === 'clients' ? 'clients' : 'list';
  const role = params.get('role') ?? '';
  const overdue = params.get('overdue') === '1';

  const set = useCallback((k: string, v: string) => {
    setParams((p) => {
      const n = new URLSearchParams(p);
      if (v) n.set(k, v); else n.delete(k);
      return n;
    }, { replace: true });
  }, [setParams]);

  useEffect(() => {
    const t = setTimeout(() => set('q', q.trim()), 250);
    return () => clearTimeout(t);
  }, [q, set]);
  const loadOverdue = useCallback(
    () => work.tasks({ overdue: true, role: (role || baseRole) as Role | undefined }).then((l) => setOverdueCount(l.length)).catch(() => {}),
    [role, baseRole],
  );
  useEffect(() => void loadOverdue(), [loadOverdue]);
  useLiveReload(loadOverdue);

  const seg = (active: boolean) => `inline-flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition ${active ? 'bg-white text-brand-700 shadow-sm dark:bg-slate-700 dark:text-white' : 'text-slate-600 hover:text-slate-900 dark:text-slate-300 dark:hover:text-white'}`;
  const chip = (active = false) => `inline-flex shrink-0 items-center gap-1.5 rounded-xl border px-3 py-1.5 text-sm font-medium transition ${active ? 'border-brand-500 bg-brand-50 text-brand-700 dark:bg-brand-500/15 dark:text-brand-100' : 'border-slate-200 bg-white text-slate-700 hover:border-brand-400 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200'}`;

  return (
    <div className="mb-5">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="mr-auto text-2xl font-bold tracking-tight">{title}</h1>
        <div className="relative order-last w-full sm:order-none sm:w-72">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-slate-400" />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Поиск по задачам и клиентам" className="input py-2 pl-9 pr-9" />
          {q && <button onClick={() => setQ('')} className="absolute right-3 top-1/2 -translate-y-1/2 text-slate-400" aria-label="Очистить"><X className="size-4" /></button>}
        </div>
        {onCreate && <Button onClick={onCreate}><Plus className="size-4" />Создать</Button>}
      </div>
      {/* Одна строка фильтров; на телефоне листается вбок, а не съедает пол-экрана. */}
      <div className="-mx-1 mt-3 flex items-center gap-2 overflow-x-auto px-1 pb-1">
        <div className="flex shrink-0 rounded-xl bg-slate-100 p-0.5 dark:bg-slate-800">
          <button className={seg(view === 'list')} onClick={() => set('view', '')}><List className="size-4" />Список</button>
          <button className={seg(view === 'clients')} onClick={() => set('view', 'clients')}><Building2 className="size-4" />Клиенты</button>
          <button className={seg(view === 'calendar')} onClick={() => set('view', 'calendar')}><CalendarDays className="size-4" />Календарь</button>
        </div>
        <div className="flex shrink-0 rounded-xl bg-slate-100 p-0.5 dark:bg-slate-800">
          {roles.map((r) => <button key={r.v} className={seg(role === r.v)} onClick={() => set('role', r.v)}>{r.label}</button>)}
        </div>
        <button className={chip(overdue)} onClick={() => set('overdue', overdue ? '' : '1')} title="Показать только просроченные">
          Просрочены{overdueCount > 0 && <span className="rounded-full bg-red-500 px-1.5 text-[11px] font-bold text-white">{count(overdueCount)}</span>}
        </button>
        <Link to="/messenger" className={chip()}>
          Чаты задач{unread > 0 && <span className="rounded-full bg-brand-600 px-1.5 text-[11px] font-bold text-white">{count(unread)}</span>}
        </Link>
        {unread > 0 && <button className={chip()} onClick={async () => { await work.readAll(); refreshUnread(); }}>Прочитать все</button>}
      </div>
    </div>
  );
}

/** По клиентам: первый уровень — клиент, второй — его задачи (подзадачи с отступом). */
function ByClients({ tasks }: { tasks: TaskSummary[] }) {
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const groups = useMemo(() => {
    const m = new Map<string, { id: string; name: string; tasks: TaskSummary[] }>();
    for (const t of tasks) {
      const g = m.get(t.client_id) ?? { id: t.client_id, name: t.client_name, tasks: [] };
      g.tasks.push(t);
      m.set(t.client_id, g);
    }
    return [...m.values()].sort((a, b) => a.name.localeCompare(b.name, 'ru'));
  }, [tasks]);
  const toggle = (id: string) => setClosed((s) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  const allClosed = groups.length > 0 && groups.every((g) => closed.has(g.id));
  return (
    <div className="flex flex-col gap-3">
      <div className="flex justify-end">
        <button className="text-sm text-brand-600 hover:underline" onClick={() => setClosed(allClosed ? new Set() : new Set(groups.map((g) => g.id)))}>
          {allClosed ? 'Развернуть всех' : 'Свернуть всех'}
        </button>
      </div>
      {groups.map((g) => {
        const open = g.tasks.filter((t) => t.status && t.status !== 'completed').length;
        const overdue = g.tasks.filter((t) => t.deadline_state === 'overdue').length;
        const unread = g.tasks.reduce((n, t) => n + (t.unread ?? 0), 0);
        const by = childrenOf(g.tasks);
        const rows = (parent: string | null, depth: number): React.ReactNode[] =>
          (by.get(parent) ?? [])
            .sort((a, b) => Number(a.status === 'completed') - Number(b.status === 'completed') || (a.deadline_at ?? 0) - (b.deadline_at ?? 0))
            .flatMap((t) => [<TaskRow key={t.id} t={t} depth={depth} />, ...rows(t.id, depth + 1)]);
        const isOpen = !closed.has(g.id);
        return (
          <section key={g.id} className="card overflow-hidden">
            <div className="flex items-center gap-3 bg-slate-50 px-4 py-2.5 dark:bg-slate-800/60">
              <button onClick={() => toggle(g.id)} className="flex min-w-0 flex-1 items-center gap-2 text-left" aria-expanded={isOpen}>
                <ChevronRight className={`size-4 shrink-0 text-slate-400 transition ${isOpen ? 'rotate-90' : ''}`} />
                <Building2 className="size-4 shrink-0 text-brand-500" />
                <span className="truncate font-semibold">{g.name}</span>
                <span className="shrink-0 text-xs text-slate-500">задач: {g.tasks.length} · открыто: {open}</span>
                {overdue > 0 && <span className="shrink-0 rounded-full bg-red-100 px-2 text-xs font-semibold text-red-700 dark:bg-red-500/15 dark:text-red-300">просрочено {overdue}</span>}
                {unread > 0 && <span className="shrink-0 rounded-full bg-brand-600 px-2 text-xs font-bold text-white">{unread}</span>}
              </button>
              <Link to={`/clients/${g.id}`} className="shrink-0 text-xs text-brand-600 hover:underline">Карточка клиента</Link>
            </div>
            {isOpen && <div className="divide-y divide-slate-100 dark:divide-slate-800">{rows(null, 0)}</div>}
          </section>
        );
      })}
    </div>
  );
}

const weekDays = ['Пн', 'Вт', 'Ср', 'Чт', 'Пт', 'Сб', 'Вс'];

/** «Ноябрь 2026» — без «г.», который браузер добавляет к месяцу с годом. */
export function monthTitle(d: Date) {
  const m = d.toLocaleDateString('ru-RU', { month: 'long' });
  return `${m[0].toUpperCase()}${m.slice(1)} ${d.getFullYear()}`;
}

/** Месяц: задачи по дню крайнего срока. */
function Calendar({ tasks }: { tasks: TaskSummary[] }) {
  const [month, setMonth] = useState(() => { const d = new Date(); return new Date(d.getFullYear(), d.getMonth(), 1); });
  const days = useMemo(() => {
    const first = (month.getDay() + 6) % 7;
    return Array.from({ length: 42 }, (_, i) => new Date(month.getFullYear(), month.getMonth(), 1 - first + i));
  }, [month]);
  const byDay = useMemo(() => {
    const m = new Map<string, TaskSummary[]>();
    for (const t of tasks) {
      if (!t.deadline_at) continue;
      const k = new Date(t.deadline_at).toDateString();
      m.set(k, [...(m.get(k) ?? []), t]);
    }
    return m;
  }, [tasks]);
  const today = new Date().toDateString();
  const shift = (n: number) => setMonth(new Date(month.getFullYear(), month.getMonth() + n, 1));
  return (
    <div className="card overflow-hidden">
      <div className="flex items-center gap-2 border-b border-slate-200 px-4 py-3 dark:border-slate-800">
        <button className="rounded-lg p-1.5 hover:bg-slate-100 dark:hover:bg-slate-800" onClick={() => shift(-1)} aria-label="Предыдущий месяц"><ChevronLeft className="size-5" /></button>
        <p className="min-w-40 text-center font-semibold">{monthTitle(month)}</p>
        <button className="rounded-lg p-1.5 hover:bg-slate-100 dark:hover:bg-slate-800" onClick={() => shift(1)} aria-label="Следующий месяц"><ChevronRight className="size-5" /></button>
        <button className="ml-2 rounded-lg px-2.5 py-1 text-sm text-brand-600 hover:bg-brand-50 dark:hover:bg-brand-500/10" onClick={() => { const d = new Date(); setMonth(new Date(d.getFullYear(), d.getMonth(), 1)); }}>Сегодня</button>
      </div>
      <div className="grid grid-cols-7 border-b border-slate-200 text-center text-xs font-semibold text-slate-500 dark:border-slate-800">
        {weekDays.map((d) => <div key={d} className="py-2">{d}</div>)}
      </div>
      <div className="grid grid-cols-7">
        {days.map((d) => {
          const list = byDay.get(d.toDateString()) ?? [];
          const other = d.getMonth() !== month.getMonth();
          return (
            <div key={d.toISOString()} className={`min-h-24 border-b border-r border-slate-100 p-1.5 dark:border-slate-800 ${other ? 'bg-slate-50/60 dark:bg-slate-900/40' : ''}`}>
              <p className={`mb-1 text-right text-xs ${d.toDateString() === today ? 'font-bold text-brand-600' : other ? 'text-slate-400' : 'text-slate-500'}`}>{d.getDate()}</p>
              <div className="flex flex-col gap-1">
                {list.slice(0, 4).map((t) => (
                  <Link key={t.id} to={`/tasks/${t.id}`} title={`${t.title} · ${t.client_name}`}
                    className={`truncate rounded border-l-[3px] bg-white px-1.5 py-0.5 text-[11px] font-medium shadow-sm hover:bg-brand-50 dark:bg-slate-800 ${deadlineBar(t)} ${t.status === 'completed' ? 'line-through opacity-60' : ''}`}>
                    {t.title}
                  </Link>
                ))}
                {list.length > 4 && <span className="px-1 text-[11px] text-slate-500">ещё {list.length - 4}</span>}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function TaskList({ mode }: { mode: 'mine' | 'all' }) {
  const { me } = useAuth();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const [tasks, setTasks] = useState<TaskSummary[] | null>(null);
  const [error, setError] = useState('');
  const [creating, setCreating] = useState(false);
  // «Мои задачи» — где я исполнитель или постановщик; роль в шапке сужает.
  const baseRole: Role | undefined = mode === 'mine' ? 'involved' : undefined;
  const role = ((params.get('role') || undefined) as Role | undefined) ?? baseRole;
  const q = params.get('q') || undefined;
  const overdue = params.get('overdue') === '1';
  const calendar = params.get('view') === 'calendar';
  const grouped = params.get('view') === 'clients';

  const load = useCallback(() => {
    work.tasks({ role, q, overdue }).then(setTasks).catch((e) => setError(e.message));
  }, [role, q, overdue]);
  useEffect(load, [load]);
  useLiveReload(load);

  const [showDone, setShowDone] = useState(false);
  const [archivedNote, setArchivedNote] = useState('');
  const sorted = [...(tasks ?? [])].sort((a, b) => Number(a.status === 'completed') - Number(b.status === 'completed') || (a.deadline_at ?? 0) - (b.deadline_at ?? 0));
  const canCreate = me?.role !== 'employee';
  const open = sorted.filter((t) => t.status !== 'completed');
  const done = sorted.filter((t) => t.status === 'completed');

  return (
    <>
      <Header title={mode === 'mine' ? 'Мои задачи' : 'Все задачи'} baseRole={baseRole} onCreate={canCreate ? () => setCreating(true) : undefined} />
      <ErrorText>{error}</ErrorText>
      {calendar ? <Calendar tasks={sorted} /> : (
        <>
          {tasks && tasks.length === 0 && (
            <div className="card px-8 py-16 text-center text-slate-500">
              {q ? 'По запросу ничего не нашлось.' : overdue ? 'Просроченных задач нет.' : 'Задач нет.'}
            </div>
          )}
          {tasks && tasks.length > 0 && (grouped ? <ByClients tasks={sorted} /> : (
            <>
              {open.length > 0 && (
                <div className="card divide-y divide-slate-100 overflow-hidden dark:divide-slate-800">
                  {open.map((t) => <TaskRow key={t.id} t={t} showClient />)}
                </div>
              )}
              {done.length > 0 && (
                <details className="mt-4" open={showDone} onToggle={(e) => setShowDone((e.target as HTMLDetailsElement).open)}>
                  <summary className="flex cursor-pointer items-center gap-2 text-sm font-semibold text-slate-600 dark:text-slate-300">
                    Завершены · {done.length}
                    <button type="button" className="ml-auto rounded-lg px-2 py-1 text-xs font-medium text-brand-600 hover:bg-brand-50 dark:hover:bg-brand-500/10"
                      onClick={async (e) => {
                        e.preventDefault();
                        const r = await work.archiveCompleted();
                        setArchivedNote(r.archived ? `Убрано в архив: ${r.archived}` : 'Нечего убирать: завершённые убирает ответственный или владелец.');
                        load();
                      }}>В архив все завершённые</button>
                  </summary>
                  <div className="card mt-2 divide-y divide-slate-100 overflow-hidden dark:divide-slate-800">
                    {done.map((t) => <TaskRow key={t.id} t={t} showClient />)}
                  </div>
                </details>
              )}
            </>
          ))}
          {archivedNote && <p className="mt-2 text-sm text-emerald-700 dark:text-emerald-300">{archivedNote}</p>}
          <p className="mt-3 text-xs text-slate-500">{deadlineLegend} Завершённые уходят в <Link to="/archive" className="text-brand-600">архив</Link>, когда их уберёт ответственный или владелец.</p>
        </>
      )}
      <TaskForm open={creating} onClose={() => setCreating(false)} onCreated={(id) => { setCreating(false); navigate(`/tasks/${id}`); }} />
    </>
  );
}

/** Архив: выполненные задачи, которые убрали исполнитель или владелец. */
export function Archive() {
  const { me } = useAuth();
  const [tasks, setTasks] = useState<TaskSummary[] | null>(null);
  const [q, setQ] = useState('');
  const [error, setError] = useState('');
  const [purging, setPurging] = useState<TaskSummary | null>(null);
  const load = useCallback(() => work.tasks({ archived: true, q: q.trim() || undefined }).then(setTasks).catch((e) => setError(e.message)), [q]);
  useEffect(() => { const t = setTimeout(load, 200); return () => clearTimeout(t); }, [load]);
  useLiveReload(load);
  const act = (f: () => Promise<unknown>) => { setError(''); f().then(load).catch((e) => setError(e.message)); };
  const owner = me?.role === 'host';

  return (
    <>
      <div className="mb-5 rounded-2xl bg-gradient-to-r from-slate-700 to-slate-500 p-5 text-white shadow-lg">
        <h1 className="text-2xl font-bold tracking-tight">Архив</h1>
        <p className="mt-1 text-sm text-white/80">Выполненные задачи, которые убрали исполнитель или владелец. «Восстановить» — вернуть в список завершённой, «В работу» — сразу возобновить{owner ? '; удалить навсегда — только владелец' : ''}.</p>
        <div className="relative mt-3 max-w-md">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-white/70" />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Поиск в архиве" className="w-full rounded-xl bg-white/15 py-2 pl-9 pr-3 text-sm text-white placeholder-white/70 outline-none focus:bg-white/25" />
        </div>
      </div>
      <ErrorText>{error}</ErrorText>
      {tasks?.length === 0 && <div className="card px-8 py-16 text-center text-slate-500">Архив пуст.</div>}
      {tasks && tasks.length > 0 && (
        <div className="card divide-y divide-slate-100 overflow-hidden dark:divide-slate-800">
          {tasks.map((t) => (
            <TaskRow key={t.id} t={t} showClient extra={
              <span className="flex items-center gap-1" onClick={(e) => e.preventDefault()}>
                <button className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-brand-600 hover:bg-brand-50 dark:hover:bg-brand-500/10" title="Восстановить из архива"
                  onClick={() => act(() => work.unarchive(t.id))}><ArchiveRestore className="size-3.5" />Восстановить</button>
                <button className="rounded-lg px-2 py-1 text-xs font-medium text-emerald-700 hover:bg-emerald-50 dark:text-emerald-300 dark:hover:bg-emerald-500/10" title="Восстановить и сразу вернуть в работу"
                  onClick={() => act(() => work.unarchive(t.id, true))}>В работу</button>
                {owner && <button className="rounded-lg p-1 text-slate-400 hover:bg-red-50 hover:text-red-600 dark:hover:bg-red-500/10" title="Удалить навсегда"
                  onClick={() => setPurging(t)}><Trash2 className="size-4" /></button>}
              </span>
            } />
          ))}
        </div>
      )}
      <Modal open={!!purging} onClose={() => setPurging(null)} title="Удалить задачу навсегда?">
        <p className="text-sm text-slate-600 dark:text-slate-300">«{purging?.title}» — вместе с подзадачами, чатом и файлами. Вернуть можно будет только из резервной копии.</p>
        <p className="mt-2 text-xs text-slate-500">{purging?.completed_at ? `Завершена ${dateTime(purging.completed_at)}` : ''}</p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setPurging(null)}>Отмена</Button>
          <Button variant="danger" onClick={() => { const t = purging; setPurging(null); if (t) act(() => work.purgeTask(t.id)); }}>Удалить навсегда</Button>
        </div>
      </Modal>
    </>
  );
}
