import { useCallback, useEffect, useState } from 'react';
import { Link, NavLink, useNavigate, useParams, useSearchParams } from 'react-router';
import { Archive, ArchiveRestore, Check, ChevronRight, Download, LayoutGrid, List, Pencil, Plus, Trash2, X } from 'lucide-react';
import { useAuth } from '../auth';
import { useLiveReload } from '../live';
import { dateTime, fromLocalInput, type Priority, type Status, statusName, type TaskCard, type TaskSummary, toLocalInput, work } from '../work';
import { Button, ErrorText, Modal, PageHeader } from '../components/ui';
import { DeadlineText, Md, PeoplePicker, PriorityBadge, Progress, StatusBadge, TaskForm, TaskRow, UnreadDot, deadlineBar, usePeople } from '../components/work';
import { Chat } from '../components/Chat';

const tabs = [
  { to: '/', label: 'Мои задачи' },
  { to: '/tasks', label: 'Все задачи' },
  { to: '/archive', label: 'Архив' },
];

function Tabs() {
  return (
    <div className="mb-5 flex gap-1 overflow-x-auto border-b border-slate-200 dark:border-slate-800">
      {tabs.map((t) => (
        <NavLink key={t.to} to={t.to} end
          className={({ isActive }) => `whitespace-nowrap border-b-2 px-3 py-2 text-sm font-medium ${isActive ? 'border-brand-600 text-brand-700 dark:text-brand-100' : 'border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200'}`}>
          {t.label}
        </NavLink>
      ))}
    </div>
  );
}

const columns: Status[] = ['new', 'in_progress', 'completed'];

function Board({ tasks, reload }: { tasks: TaskSummary[]; reload: () => void }) {
  const [error, setError] = useState('');
  async function move(id: string, to: Status) {
    setError('');
    try {
      const r = await work.setStatus(id, to);
      if (to === 'completed' && r.open_stages > 0) setError(`Задача завершена, но этапов не отмечено: ${r.open_stages}.`);
    } catch (e) {
      setError((e as Error).message);
    }
    reload();
  }
  return (
    <>
      <ErrorText>{error}</ErrorText>
      <div className="mt-2 grid gap-4 md:grid-cols-3">
        {columns.map((s) => (
          <div key={s} className="flex min-h-40 flex-col gap-2 rounded-2xl bg-slate-100/70 p-3 dark:bg-slate-900/60"
            onDragOver={(e) => e.preventDefault()}
            onDrop={(e) => { const id = e.dataTransfer.getData('text/task'); if (id) void move(id, s); }}>
            <p className="px-1 text-sm font-semibold text-slate-600 dark:text-slate-300">{statusName[s]} · {tasks.filter((t) => t.status === s).length}</p>
            {tasks.filter((t) => t.status === s).map((t) => (
              <Link key={t.id} to={`/tasks/${t.id}`} draggable onDragStart={(e) => e.dataTransfer.setData('text/task', t.id)}
                className={`card flex flex-col gap-1.5 border-l-4 p-3 transition hover:shadow-md ${deadlineBar(t)}`}>
                <div className="flex items-center gap-2">{t.priority && <PriorityBadge p={t.priority} />}<span className="truncate text-xs text-slate-500">{t.client_name}</span><span className="ml-auto"><UnreadDot n={t.unread} /></span></div>
                <p className="font-medium">{t.title}</p>
                <div className="flex flex-wrap items-center gap-2">{t.progress && <Progress {...t.progress} />}<DeadlineText t={t} /></div>
                {/* На телефоне перетаскивания нет — кнопка «дальше». */}
                {s !== 'completed' && (
                  <button className="self-start text-xs font-medium text-brand-600 md:hidden" onClick={(e) => { e.preventDefault(); void move(t.id, s === 'new' ? 'in_progress' : 'completed'); }}>
                    → {statusName[s === 'new' ? 'in_progress' : 'completed']}
                  </button>
                )}
              </Link>
            ))}
          </div>
        ))}
      </div>
    </>
  );
}

export function TaskList({ mode }: { mode: 'mine' | 'all' | 'archive' }) {
  const [params, setParams] = useSearchParams();
  const board = params.get('view') === 'board';
  const status = (params.get('status') || undefined) as Status | undefined;
  const priority = params.get('priority') ? Number(params.get('priority')) : undefined;
  const [tasks, setTasks] = useState<TaskSummary[] | null>(null);
  const [error, setError] = useState('');

  const load = useCallback(() => {
    work.tasks({ view: mode === 'mine' ? 'mine' : undefined, archived: mode === 'archive', status: board ? undefined : status, priority })
      .then(setTasks).catch((e) => setError(e.message));
  }, [mode, status, priority, board]);
  useEffect(load, [load]);
  useLiveReload(load);

  const set = (k: string, v: string) => {
    const p = new URLSearchParams(params);
    if (v) p.set(k, v); else p.delete(k);
    setParams(p, { replace: true });
  };
  // По умолчанию незавершённые — сверху, по сроку.
  const sorted = [...(tasks ?? [])].sort((a, b) => Number(a.status === 'completed') - Number(b.status === 'completed') || (a.deadline_at ?? 0) - (b.deadline_at ?? 0));

  return (
    <>
      <PageHeader
        title={mode === 'mine' ? 'Мои задачи' : mode === 'all' ? 'Все задачи' : 'Архив'}
        subtitle={mode === 'archive' ? 'Завершённые задачи уходят сюда автоматически; их можно вернуть.' : tasks ? `${tasks.filter((t) => t.status !== 'completed').length} открыто` : 'Загрузка…'}
        action={mode !== 'archive' && <a href="/api/export/tasks" className="inline-flex items-center gap-2 rounded-xl px-3 py-2.5 text-sm font-semibold text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"><Download className="size-4" />Excel</a>}
      />
      <Tabs />
      {mode !== 'archive' && (
        <div className="mb-4 flex flex-wrap items-center gap-2">
          <div className="flex rounded-xl border border-slate-200 p-0.5 dark:border-slate-700">
            <button onClick={() => set('view', '')} className={`rounded-lg px-2.5 py-1.5 ${!board ? 'bg-slate-100 dark:bg-slate-800' : ''}`} title="Список"><List className="size-4" /></button>
            <button onClick={() => set('view', 'board')} className={`rounded-lg px-2.5 py-1.5 ${board ? 'bg-slate-100 dark:bg-slate-800' : ''}`} title="Канбан"><LayoutGrid className="size-4" /></button>
          </div>
          {!board && (
            <select className="input w-auto py-2" value={status ?? ''} onChange={(e) => set('status', e.target.value)}>
              <option value="">Любой статус</option>
              {columns.map((s) => <option key={s} value={s}>{statusName[s]}</option>)}
            </select>
          )}
          <select className="input w-auto py-2" value={priority ?? ''} onChange={(e) => set('priority', e.target.value)}>
            <option value="">Любой приоритет</option>
            <option value="1">P1</option><option value="2">P2</option><option value="3">P3</option>
          </select>
        </div>
      )}
      <ErrorText>{error}</ErrorText>
      {tasks && tasks.length === 0 && (
        <div className="card px-8 py-16 text-center text-slate-500">
          {mode === 'mine' ? 'Вам пока не назначено задач.' : mode === 'archive' ? 'Архив пуст.' : 'Задач нет.'}
        </div>
      )}
      {board && tasks ? <Board tasks={tasks} reload={load} /> : (
        tasks && tasks.length > 0 && (
          <div className="card divide-y divide-slate-100 overflow-hidden dark:divide-slate-800">
            {sorted.map((t) => (
              <TaskRow key={t.id} t={t} showClient
                extra={mode === 'archive' && t.access === 'manage' && (
                  <button className="text-xs font-medium text-brand-600" onClick={async (e) => { e.preventDefault(); await work.unarchive(t.id); load(); }}>
                    <ArchiveRestore className="inline size-3.5" /> Вернуть
                  </button>
                )} />
            ))}
          </div>
        )
      )}
    </>
  );
}

// ---------- Страница задачи ----------

function Stages({ t, reload }: { t: TaskCard; reload: () => void }) {
  const manage = t.access === 'manage';
  const [title, setTitle] = useState('');
  const [error, setError] = useState('');
  const run = (f: () => Promise<unknown>) => f().then(reload).catch((e) => setError(e.message));
  return (
    <div>
      <div className="mb-2 flex items-center justify-between">
        <h3 className="font-semibold">Этапы</h3>
        {t.progress && <Progress {...t.progress} />}
      </div>
      {t.stages.length === 0 && <p className="text-sm text-slate-500">Этапов нет.</p>}
      <ul className="flex flex-col gap-1">
        {t.stages.map((s) => (
          <li key={s.id} className="group flex items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-slate-50 dark:hover:bg-slate-800/50">
            <input type="checkbox" className="size-4 accent-brand-600" checked={!!s.completed_at} disabled={t.archived_at != null}
              onChange={(e) => run(() => work.updateStage(s.id, { done: e.target.checked }))} />
            <span className={`flex-1 text-sm ${s.completed_at ? 'text-slate-400 line-through' : ''}`}>{s.title}</span>
            {s.completed_at && <span className="text-xs text-slate-400">{s.completed_by_name}</span>}
            {manage && <button className="hidden text-slate-400 hover:text-red-600 group-hover:block" title="Удалить этап" onClick={() => run(() => work.deleteStage(s.id))}><X className="size-4" /></button>}
          </li>
        ))}
      </ul>
      {manage && (
        <form className="mt-2 flex gap-2" onSubmit={(e) => { e.preventDefault(); if (title.trim()) run(async () => { await work.addStage(t.id, title); setTitle(''); }); }}>
          <input className="input py-2" placeholder="Новый этап" value={title} onChange={(e) => setTitle(e.target.value)} maxLength={300} />
          <Button type="submit" variant="secondary" className="px-3"><Plus className="size-4" /></Button>
        </form>
      )}
      {error && <p className="mt-2 text-xs text-red-600">{error}</p>}
    </div>
  );
}

function EditTask({ t, onClose, onSaved }: { t: TaskCard; onClose: () => void; onSaved: () => void }) {
  const people = usePeople();
  const [assignees, setAssignees] = useState((t.assignees ?? []).map((a) => a.id));
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal open onClose={onClose} title="Изменить задачу" wide>
      <form className="grid gap-4 sm:grid-cols-2" onSubmit={async (e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        setBusy(true);
        setError('');
        try {
          await work.updateTask(t.id, {
            title: String(f.get('title')),
            description: String(f.get('description')),
            priority: Number(f.get('priority')) as Priority,
            deadline_at: fromLocalInput(String(f.get('deadline'))),
            assignees,
          });
          onSaved();
        } catch (x) {
          setError((x as Error).message);
        } finally {
          setBusy(false);
        }
      }}>
        <label className="flex flex-col gap-1.5 text-sm font-medium sm:col-span-2">Название<input name="title" className="input" defaultValue={t.title} required maxLength={300} /></label>
        <label className="flex flex-col gap-1.5 text-sm font-medium">Приоритет
          <select name="priority" className="input" defaultValue={t.priority}><option value="1">P1 — высокий</option><option value="2">P2 — средний</option><option value="3">P3 — низкий</option></select>
        </label>
        <label className="flex flex-col gap-1.5 text-sm font-medium">Дедлайн
          <input name="deadline" type="datetime-local" className="input" defaultValue={toLocalInput(t.deadline_at ?? Date.now())} required />
          {t.parent_deadline_at && <span className="text-xs font-normal text-slate-500">Не позже {dateTime(t.parent_deadline_at)}</span>}
        </label>
        <div className="flex flex-col gap-1.5 text-sm font-medium sm:col-span-2">Исполнители<PeoplePicker people={people} value={assignees} onChange={setAssignees} /></div>
        <label className="flex flex-col gap-1.5 text-sm font-medium sm:col-span-2">Описание<textarea name="description" className="input min-h-28" defaultValue={t.description} maxLength={50000} /></label>
        <div className="flex flex-col gap-3 sm:col-span-2"><ErrorText>{error}</ErrorText><Button type="submit" busy={busy}>Сохранить</Button></div>
      </form>
    </Modal>
  );
}

export function TaskPage() {
  const { id = '' } = useParams();
  const navigate = useNavigate();
  const { me } = useAuth();
  const [t, setT] = useState<TaskCard | null>(null);
  const [error, setError] = useState('');
  const [note, setNote] = useState('');
  const [editing, setEditing] = useState(false);
  const [sub, setSub] = useState(false);
  const [trash, setTrash] = useState(false);

  const load = useCallback(() => work.task(id).then(setT).catch((e) => setError(e.message)), [id]);
  useEffect(() => { setT(null); setError(''); setNote(''); void load(); }, [load]);
  useLiveReload(load, id);

  if (error && !t) return <ErrorText>{error === 'не найдено' ? 'Задача не найдена или у вас нет к ней доступа.' : error}</ErrorText>;
  if (!t) return null;
  const manage = t.access === 'manage';
  const archived = t.archived_at != null;

  async function status(s: Status) {
    if (!t) return;
    setNote('');
    try {
      const r = await work.setStatus(t.id, s);
      if (s === 'completed' && r.open_stages > 0) setNote(`Задача завершена, хотя этапов не отмечено: ${r.open_stages}.`);
      load();
    } catch (e) {
      setNote((e as Error).message);
    }
  }

  return (
    <div className="flex flex-col gap-5 lg:h-[calc(100dvh-5rem)] lg:flex-row">
      <div className="min-w-0 flex-1 overflow-y-auto lg:pr-2">
        <nav className="mb-3 flex flex-wrap items-center gap-1 text-sm text-slate-500">
          <Link to={`/clients/${t.client_id}`} className="hover:text-brand-600">{t.client_name}</Link>
          {t.path.map((p) => (
            <span key={p.id} className="flex items-center gap-1"><ChevronRight className="size-4" />
              {p.readable ? <Link to={`/tasks/${p.id}`} className="hover:text-brand-600">{p.title}</Link> : <span>{p.title}</span>}
            </span>
          ))}
        </nav>
        <div className="mb-4 flex flex-wrap items-start gap-3">
          <h1 className="min-w-0 flex-1 text-2xl font-bold tracking-tight">{t.title}</h1>
          {manage && !archived && <Button variant="secondary" onClick={() => setEditing(true)}><Pencil className="size-4" />Изменить</Button>}
          {t.can_trash && <Button variant="ghost" title="В корзину" onClick={() => setTrash(true)}><Trash2 className="size-4" /></Button>}
        </div>
        <div className="card mb-5 flex flex-wrap items-center gap-x-5 gap-y-3 p-4 text-sm">
          {t.priority && <PriorityBadge p={t.priority} />}
          {t.status && <StatusBadge s={t.status} />}
          <span className="text-slate-500">до <b className="text-slate-800 dark:text-slate-100">{t.deadline_at && dateTime(t.deadline_at)}</b></span>
          <DeadlineText t={t} />
          <span className="text-slate-500">Исполнители: <b className="text-slate-800 dark:text-slate-100">{t.assignees?.length ? t.assignees.map((a) => a.name).join(', ') : 'не назначены'}</b></span>
        </div>
        {archived ? (
          <div className="mb-5 flex items-center gap-3 rounded-xl bg-slate-100 px-4 py-3 text-sm dark:bg-slate-800">
            <Archive className="size-4" /> Задача в архиве.
            {manage && <Button variant="secondary" className="ml-auto py-1.5" onClick={async () => { await work.unarchive(t.id); load(); }}>Вернуть из архива</Button>}
          </div>
        ) : (
          <div className="mb-5 flex flex-wrap gap-2">
            {t.status !== 'in_progress' && <Button variant="secondary" onClick={() => status('in_progress')}>{t.status === 'completed' ? 'Переоткрыть' : 'Взять в работу'}</Button>}
            {t.status !== 'completed' && <Button onClick={() => status('completed')}><Check className="size-4" />Завершить</Button>}
          </div>
        )}
        {note && <div className="mb-5"><ErrorText>{note}</ErrorText></div>}
        {t.description && <div className="card mb-5 p-5"><Md text={t.description} /></div>}
        <div className="card mb-5 p-5"><Stages t={t} reload={load} /></div>
        <div className="card mb-5 p-5">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="font-semibold">Подзадачи</h3>
            {manage && !archived && t.status !== 'completed' && <Button variant="secondary" className="px-3 py-1.5" onClick={() => setSub(true)}><Plus className="size-4" />Подзадача</Button>}
          </div>
          {t.subtasks.length ? (
            <div className="divide-y divide-slate-100 overflow-hidden rounded-xl dark:divide-slate-800">{t.subtasks.map((s) => <TaskRow key={s.id} t={s} />)}</div>
          ) : <p className="text-sm text-slate-500">Подзадач нет. Задачу нельзя завершить, пока открыты подзадачи.</p>}
        </div>
        <p className="mb-6 text-xs text-slate-400">Создал {t.created_by.name}{t.created_at ? ` · ${dateTime(t.created_at)}` : ''}</p>
      </div>
      <section className="card flex h-[70dvh] flex-col overflow-hidden lg:h-full lg:w-[420px]">
        <h3 className="border-b border-slate-200 px-4 py-3 font-semibold dark:border-slate-800">Чат задачи</h3>
        <Chat taskId={t.id} canWrite={!archived && (t.access === 'participant' || t.access === 'manage')} />
      </section>
      {editing && <EditTask t={t} onClose={() => setEditing(false)} onSaved={() => { setEditing(false); load(); }} />}
      <TaskForm open={sub} onClose={() => setSub(false)} parent={{ id: t.id, deadline_at: t.deadline_at, title: t.title }} onCreated={(nid) => { setSub(false); navigate(`/tasks/${nid}`); }} />
      <Modal open={trash} onClose={() => setTrash(false)} title="Убрать задачу в корзину?">
        <p className="text-sm text-slate-600 dark:text-slate-300">Вместе с подзадачами и чатом. Вернуть можно из корзины{me?.role === 'host' ? '' : ' (если задачу создали вы)'}.</p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setTrash(false)}>Отмена</Button>
          <Button variant="danger" onClick={async () => { await work.trashTask(t.id); navigate(`/clients/${t.client_id}`); }}>В корзину</Button>
        </div>
      </Modal>
    </div>
  );
}
