import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import {
  Archive, ArchiveRestore, CalendarDays, ChevronDown, ChevronRight, ListChecks, MoreHorizontal, Pencil, Plus, Trash2, UserRoundCog, X,
} from 'lucide-react';
import { useLiveReload } from '../live';
import { dateTime, fromLocalInput, type Priority, statusActions, statusName, type Status, type TaskCard, toLocalInput, work } from '../work';
import { Avatar, BackButton, Button, ErrorText, Modal } from '../components/ui';
import { DeadlineText, Md, PeoplePicker, PriorityBadge, Progress, TaskForm, TaskRow, statusIcon, statusStyle, usePeople } from '../components/work';
import { Chat } from '../components/Chat';

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[130px_1fr] items-center gap-3 py-2 text-[15px]">
      <span className="text-slate-500">{label}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** Выпадающее меню у кнопки. */
function Menu({ button, children }: { button: (toggle: () => void) => ReactNode; children: (close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => { if (box.current && !box.current.contains(e.target as Node)) setOpen(false); };
    window.addEventListener('mousedown', close);
    return () => window.removeEventListener('mousedown', close);
  }, [open]);
  return (
    <div className="relative" ref={box}>
      {button(() => setOpen((o) => !o))}
      {open && <div className="card absolute z-30 mt-1 min-w-56 overflow-hidden py-1 shadow-xl">{children(() => setOpen(false))}</div>}
    </div>
  );
}

function MenuItem({ onClick, icon, children, danger = false }: { onClick: () => void; icon?: ReactNode; children: ReactNode; danger?: boolean }) {
  return (
    <button onClick={onClick} className={`flex w-full items-center justify-between gap-6 px-4 py-2.5 text-left text-[15px] hover:bg-slate-50 dark:hover:bg-slate-800 ${danger ? 'text-red-600' : ''}`}>
      <span>{children}</span><span className="text-slate-400">{icon}</span>
    </button>
  );
}

function Stages({ t, reload }: { t: TaskCard; reload: () => void }) {
  const manage = t.access === 'manage';
  const [title, setTitle] = useState('');
  const [error, setError] = useState('');
  const run = (f: () => Promise<unknown>) => f().then(reload).catch((e) => setError(e.message));
  return (
    <div>
      <div className="mb-2 flex items-center justify-between">
        <h3 className="flex items-center gap-2 font-semibold"><ListChecks className="size-4 text-brand-500" />Этапы</h3>
        {t.progress && <Progress {...t.progress} />}
      </div>
      {t.stages.length === 0 && <p className="text-sm text-slate-500">Этапов нет.</p>}
      <ol className="flex flex-col gap-0.5">
        {t.stages.map((s, i) => (
          <li key={s.id} className="group flex items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-slate-50 dark:hover:bg-slate-800/50">
            <span className="w-5 text-right text-xs text-slate-400">{i + 1}.</span>
            <input type="checkbox" className="size-4 accent-brand-600" checked={!!s.completed_at} disabled={t.archived_at != null}
              onChange={(e) => run(() => work.updateStage(s.id, { done: e.target.checked }))} />
            <span className={`flex-1 text-sm ${s.completed_at ? 'text-slate-400 line-through' : ''}`}>{s.title}</span>
            {s.completed_at && <span className="text-xs text-slate-400">{s.completed_by_name}</span>}
            {manage && <button className="hidden text-slate-400 hover:text-red-600 group-hover:block" title="Удалить этап" onClick={() => run(() => work.deleteStage(s.id))}><X className="size-4" /></button>}
          </li>
        ))}
      </ol>
      {manage && !t.archived_at && (
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
        <label className="flex flex-col gap-1.5 text-sm font-medium">Крайний срок
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

/** Сменить исполнителя — и у задачи в работе: статус, этапы и чат остаются. */
function Reassign({ t, onClose, onSaved }: { t: TaskCard; onClose: () => void; onSaved: () => void }) {
  const people = usePeople();
  const [ids, setIds] = useState((t.assignees ?? []).map((a) => a.id));
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal open onClose={onClose} title="Сменить исполнителя">
      <form className="flex flex-col gap-4" onSubmit={async (e) => {
        e.preventDefault();
        setBusy(true);
        setError('');
        try { await work.reassign(t.id, ids); onSaved(); } catch (x) { setError((x as Error).message); } finally { setBusy(false); }
      }}>
        <p className="text-sm text-slate-600 dark:text-slate-300">Статус{t.status ? ` «${statusName[t.status]}»` : ""}, этапы и чат сохранятся. Новый исполнитель увидит задачу сразу.</p>
        {(() => {
          const team = people.filter((p) => t.team.includes(p.id));
          const rest = people.filter((p) => !t.team.includes(p.id));
          return (
            <>
              {team.length > 0 && <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">Группа клиента</p>}
              <PeoplePicker people={team} value={ids} onChange={setIds} />
              {rest.length > 0 && (
                <details className="text-sm" open={team.length === 0 || ids.some((i) => rest.some((p) => p.id === i))}>
                  <summary className="cursor-pointer text-xs font-semibold uppercase tracking-wide text-slate-500">Другие сотрудники — увидят этого клиента через задачу</summary>
                  <div className="mt-2"><PeoplePicker people={rest} value={ids} onChange={setIds} /></div>
                </details>
              )}
            </>
          );
        })()}
        <ErrorText>{error}</ErrorText>
        <Button type="submit" busy={busy} disabled={!ids.length}>Сохранить</Button>
      </form>
    </Modal>
  );
}

const longDate = (ms: number) => new Date(ms).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' });

export function TaskPage() {
  const { id = '' } = useParams();
  const navigate = useNavigate();
  const [t, setT] = useState<TaskCard | null>(null);
  const [error, setError] = useState('');
  const [note, setNote] = useState('');
  const [editing, setEditing] = useState(false);
  const [reassigning, setReassigning] = useState(false);
  const [sub, setSub] = useState(false);
  const [trash, setTrash] = useState(false);

  const load = useCallback(() => work.task(id).then(setT).catch((e) => setError(e.message)), [id]);
  useEffect(() => { setT(null); setError(''); setNote(''); void load(); }, [load]);
  useLiveReload(load, id);

  if (error && !t) return <><BackButton fallback="/" /><ErrorText>{error === 'не найдено' ? 'Задача не найдена или у вас нет к ней доступа.' : error}</ErrorText></>;
  if (!t || !t.status) return null;
  const manage = t.access === 'manage';
  const archived = t.archived_at != null;
  const status = t.status;
  const actions = archived || !t.can_change_status ? [] : statusActions.filter((a) => a.from.includes(status));
  const primary = actions.find((a) => a.to === 'in_progress') ?? actions.find((a) => a.to === 'completed');
  const secondary = actions.filter((a) => a !== primary && a.to === 'completed');
  const more = actions.filter((a) => a !== primary && !secondary.includes(a));

  const act = async (f: () => Promise<unknown>, after?: string) => {
    setNote('');
    try { await f(); if (after) setNote(after); load(); } catch (e) { setNote((e as Error).message); }
  };
  async function setStatus(to: Status) {
    if (!t) return;
    await act(async () => {
      const r = await work.setStatus(t.id, to);
      if (to === 'completed' && r.open_stages > 0) setTimeout(() => setNote(`Задача завершена, хотя этапов не отмечено: ${r.open_stages}.`), 0);
    });
  }

  return (
    <div className="-mx-4 -mt-4 flex flex-col sm:-mx-8 lg:-mx-12 lg:h-[calc(100dvh-3.5rem)] lg:flex-row">
      {/* Левая панель: поля задачи */}
      <section className="flex min-w-0 flex-col bg-slate-100/70 lg:w-[44%] lg:max-w-[640px] dark:bg-slate-900/50">
        <div className="flex-1 overflow-y-auto px-4 py-4 sm:px-6">
          <nav className="mb-2 flex flex-wrap items-center gap-1 text-sm text-slate-500">
            <span className="mr-2"><BackButton fallback="/" /></span>
            <Link to={`/clients/${t.client_id}`} className="hover:text-brand-600">{t.client_name}</Link>
            {t.path.map((p) => (
              <span key={p.id} className="flex items-center gap-1"><ChevronRight className="size-4" />
                {p.readable ? <Link to={`/tasks/${p.id}`} className="hover:text-brand-600">{p.title}</Link> : <span>{p.title}</span>}
              </span>
            ))}
          </nav>
          <h1 className="mb-4 text-2xl font-bold tracking-tight">{t.title}</h1>

          <div className="card mb-4 px-5 py-3">
            <Row label="Постановщик"><span className="flex items-center gap-2"><Avatar name={t.created_by.name} size="sm" />{t.created_by.name}</span></Row>
            <Row label="Исполнитель">
              {t.assignees?.length ? (
                <span className="flex flex-col gap-1.5">{t.assignees.map((a) => <span key={a.id} className="flex items-center gap-2"><Avatar name={a.name} size="sm" />{a.name}</span>)}</span>
              ) : <span className="text-slate-400">не назначен</span>}
              {t.can_reassign && !archived && (
                <button className="mt-1.5 flex items-center gap-1 text-sm text-brand-600 hover:underline" onClick={() => setReassigning(true)}>
                  <UserRoundCog className="size-4" />Сменить исполнителя
                </button>
              )}
            </Row>
            <Row label="Крайний срок">
              <span className="flex flex-wrap items-center gap-2"><CalendarDays className="size-4 text-brand-500" />{t.deadline_at && longDate(t.deadline_at)}<DeadlineText t={t} /></span>
            </Row>
            <div className="my-1 border-t border-slate-100 dark:border-slate-800" />
            <Row label="Статус">
              {actions.length ? (
                <Menu button={(toggle) => (
                  <button onClick={toggle} className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-[15px] font-medium ${statusStyle[status]}`}>
                    {statusIcon[status]}{statusName[status]}<ChevronDown className="size-4 opacity-60" />
                  </button>
                )}>
                  {(close) => actions.map((a) => (
                    <MenuItem key={a.label} icon={statusIcon[a.to]} onClick={() => { close(); void setStatus(a.to); }}>{a.label}</MenuItem>
                  ))}
                </Menu>
              ) : (
                <span className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-[15px] font-medium ${statusStyle[status]}`}>{statusIcon[status]}{statusName[status]}</span>
              )}
            </Row>
            <Row label="Приоритет">{t.priority && <PriorityBadge p={t.priority} />}</Row>
            <Row label="Дата создания"><span className="text-slate-600 dark:text-slate-300">{t.created_at && longDate(t.created_at)}</span></Row>
            {archived && <Row label="В архиве"><span className="text-slate-600 dark:text-slate-300">с {t.archived_at && longDate(t.archived_at)}{t.archived_by ? ` · убрал(а) ${t.archived_by}` : ''}</span></Row>}
          </div>

          {note && <div className="mb-4"><ErrorText>{note}</ErrorText></div>}
          {t.description && <div className="card mb-4 p-5"><Md text={t.description} /></div>}
          <div className="card mb-4 p-5"><Stages t={t} reload={load} /></div>
          <div className="card mb-4 p-5">
            <div className="mb-2 flex items-center justify-between">
              <h3 className="font-semibold">Подзадачи</h3>
              {manage && !archived && status !== 'completed' && <Button variant="secondary" className="px-3 py-1.5" onClick={() => setSub(true)}><Plus className="size-4" />Подзадача</Button>}
            </div>
            {t.subtasks.length ? (
              <div className="divide-y divide-slate-100 overflow-hidden rounded-xl dark:divide-slate-800">{t.subtasks.map((s) => <TaskRow key={s.id} t={s} />)}</div>
            ) : <p className="text-sm text-slate-500">Подзадач нет. Задачу нельзя завершить, пока открыты подзадачи.</p>}
          </div>
        </div>

        {/* Нижняя панель действий — как кнопки «Начать / Завершить / ⋯» */}
        <div className="sticky bottom-0 flex items-center gap-2 border-t border-slate-200 bg-white px-4 py-3 sm:px-6 dark:border-slate-800 dark:bg-slate-900">
          {primary && <Button onClick={() => setStatus(primary.to)}>{primary.to === 'in_progress' ? 'Начать' : 'Завершить'}</Button>}
          {secondary.map((a) => <Button key={a.to} variant="secondary" onClick={() => setStatus(a.to)}>Завершить</Button>)}
          {archived && t.can_archive && <Button variant="secondary" onClick={() => act(() => work.unarchive(t.id), 'Задача восстановлена из архива.')}><ArchiveRestore className="size-4" />Восстановить</Button>}
          {archived && t.can_archive && t.can_change_status && (
            <Button onClick={() => act(() => work.unarchive(t.id, true), 'Задача восстановлена и возвращена в работу.')}>Восстановить в работу</Button>
          )}
          {!archived && status === 'completed' && t.can_archive && <Button variant="secondary" onClick={() => act(() => work.archive(t.id), 'Задача убрана в архив.')}><Archive className="size-4" />В архив</Button>}
          <Menu button={(toggle) => <Button variant="ghost" onClick={toggle} title="Ещё"><MoreHorizontal className="size-5" /></Button>}>
            {(close) => (
              <>
                {more.map((a) => <MenuItem key={a.label} icon={statusIcon[a.to]} onClick={() => { close(); void setStatus(a.to); }}>{a.label}</MenuItem>)}
                {manage && !archived && <MenuItem icon={<Pencil className="size-4" />} onClick={() => { close(); setEditing(true); }}>Изменить</MenuItem>}
                {t.can_trash && !archived && <MenuItem danger icon={<Trash2 className="size-4" />} onClick={() => { close(); setTrash(true); }}>В корзину</MenuItem>}
                {!more.length && !(manage && !archived) && !(t.can_trash && !archived) && <p className="px-4 py-2.5 text-sm text-slate-500">Других действий нет</p>}
              </>
            )}
          </Menu>
        </div>
      </section>

      {/* Правая панель: чат задачи */}
      <section className="flex h-[75dvh] min-w-0 flex-1 flex-col lg:h-auto">
        <Chat taskId={t.id} canWrite={!archived && (t.access === 'participant' || t.access === 'manage')}
          participants={[t.created_by.name, ...(t.assignees ?? []).map((a) => a.name)]} />
      </section>

      {reassigning && <Reassign t={t} onClose={() => setReassigning(false)} onSaved={() => { setReassigning(false); load(); }} />}
      {editing && <EditTask t={t} onClose={() => setEditing(false)} onSaved={() => { setEditing(false); load(); }} />}
      <TaskForm open={sub} onClose={() => setSub(false)} parent={{ id: t.id, deadline_at: t.deadline_at, title: t.title }} onCreated={(nid) => { setSub(false); navigate(`/tasks/${nid}`); }} />
      <Modal open={trash} onClose={() => setTrash(false)} title="Убрать задачу в корзину?">
        <p className="text-sm text-slate-600 dark:text-slate-300">Вместе с подзадачами и чатом. Вернуть можно из корзины.</p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setTrash(false)}>Отмена</Button>
          <Button variant="danger" onClick={async () => { await work.trashTask(t.id); navigate(`/clients/${t.client_id}`); }}>В корзину</Button>
        </div>
      </Modal>
    </div>
  );
}
