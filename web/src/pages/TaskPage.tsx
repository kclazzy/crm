import { useCallback, useEffect, useState, type CSSProperties, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import {
  AlertTriangle,
  Archive, ArchiveRestore, CalendarDays, ChevronDown, ChevronRight, ListChecks, MoreHorizontal, Pencil, Plus, Trash2, UserRoundCog, X,
} from 'lucide-react';
import { useLiveReload } from '../live';
import { dateTime, fromLocalInput, type Priority, statusActions, statusName, type Status, type TaskCard, toLocalInput, work } from '../work';
import { Menu, MenuItem } from '../components/Menu';
import { Avatar, BackButton, Button, ErrorText, Modal } from '../components/ui';
import { DeadlineText, LeadPicker, Md, PeoplePicker, PriorityBadge, Progress, TaskForm, TaskRow, statusIcon, statusStyle, usePeople } from '../components/work';
import { Chat } from '../components/Chat';
import { useToast } from '../components/Toast';

// ---------- Размер текста в задаче: описание, этапы и чат; запоминается на этом устройстве ----------

const SCALES = [0.9, 1, 1.15, 1.3, 1.5];
const SCALE_KEY = 'crm-task-text';

function useTextScale(): [number, (s: number) => void] {
  const [scale, set] = useState(() => {
    try {
      const v = Number(localStorage.getItem(SCALE_KEY));
      return SCALES.includes(v) ? v : 1;
    } catch { return 1; }
  });
  return [scale, (s) => {
    set(s);
    try { localStorage.setItem(SCALE_KEY, String(s)); } catch { /* запомнить не вышло — размер действует до закрытия */ }
  }];
}

function TextSize({ scale, onChange }: { scale: number; onChange: (s: number) => void }) {
  const i = SCALES.indexOf(scale);
  const btn = 'grid h-8 w-9 place-items-center font-bold text-slate-600 hover:bg-slate-100 disabled:opacity-30 dark:text-slate-300 dark:hover:bg-slate-800';
  return (
    <div role="group" aria-label="Размер текста" className="flex shrink-0 overflow-hidden rounded-lg border border-slate-200 bg-white dark:border-slate-700 dark:bg-slate-900" style={{ fontSize: 14 }}>
      <button className={btn} disabled={i <= 0} onClick={() => onChange(SCALES[i - 1])} title="Мельче" aria-label="Текст мельче"><span style={{ fontSize: 12 }}>А</span></button>
      <span className="grid w-12 place-items-center border-x border-slate-200 text-xs tabular-nums text-slate-500 dark:border-slate-700" style={{ fontSize: 12 }} title="Размер текста в задаче">{Math.round(scale * 100)}%</span>
      <button className={btn} disabled={i >= SCALES.length - 1} onClick={() => onChange(SCALES[i + 1])} title="Крупнее" aria-label="Текст крупнее"><span style={{ fontSize: 18 }}>А</span></button>
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[130px_1fr] items-center gap-3 py-2 text-[15px]">
      <span className="text-slate-500">{label}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** Выпадающее меню у кнопки. */
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
  const manage = t.access === 'manage';
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
            // Исполнителей здесь назначает руководитель; остальные — «Сменить исполнителя».
            ...(manage ? { assignees } : {}),
          });
          onSaved();
        } catch (x) {
          setError((x as Error).message);
        } finally {
          setBusy(false);
        }
      }}>
        <label className="flex flex-col gap-1.5 text-sm font-medium sm:col-span-2">Наименование задачи<input name="title" className="input" defaultValue={t.title} required maxLength={300} /></label>
        <label className="flex flex-col gap-1.5 text-sm font-medium">Приоритет
          <select name="priority" className="input" defaultValue={t.priority}><option value="1">P1 — высокий</option><option value="2">P2 — средний</option><option value="3">P3 — низкий</option></select>
        </label>
        <label className="flex flex-col gap-1.5 text-sm font-medium">Крайний срок
          <input name="deadline" type="datetime-local" className="input" defaultValue={toLocalInput(t.deadline_at ?? Date.now())} required />
          {t.parent_deadline_at && <span className="text-xs font-normal text-slate-500">Не позже {dateTime(t.parent_deadline_at)}</span>}
        </label>
        {manage && <div className="flex flex-col gap-1.5 text-sm font-medium sm:col-span-2">Исполнители<PeoplePicker lead people={people} value={assignees} onChange={setAssignees} /><LeadPicker people={people} value={assignees} onChange={setAssignees} /></div>}
        {t.status && t.status !== 'waiting' && <p className="text-xs text-slate-500 sm:col-span-2">Статус «{statusName[t.status]}» не изменится; в чате появится запись о каждом изменении.</p>}
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
              <PeoplePicker lead people={team} value={ids} onChange={setIds} />
              {rest.length > 0 && (
                <details className="text-sm" open={team.length === 0 || ids.some((i) => rest.some((p) => p.id === i))}>
                  <summary className="cursor-pointer text-xs font-semibold uppercase tracking-wide text-slate-500">Другие сотрудники — увидят этого клиента через задачу</summary>
                  <div className="mt-2"><PeoplePicker lead people={rest} value={ids} onChange={setIds} /></div>
                </details>
              )}
              <LeadPicker people={people} value={ids} onChange={setIds} />
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
  const toast = useToast();
  const [t, setT] = useState<TaskCard | null>(null);
  const [error, setError] = useState('');
  const [note, setNote] = useState('');
  const [editing, setEditing] = useState(false);
  const [scale, setScale] = useTextScale();
  const [reassigning, setReassigning] = useState(false);
  const [tab, setTab] = useState<'task' | 'chat'>('task');
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
  const openStages = t.stages.filter((x) => !x.completed_at).length;
  const nextStep = archived || !t.can_change_status ? null
    : status === 'waiting' ? 'Возьмите задачу в работу — кнопка «Начать».'
    : status === 'postponed' ? 'Задача отложена. Когда вернётесь к ней — «Начать».'
    : status === 'in_progress' && openStages > 0 ? `Отмечайте этапы по мере выполнения (осталось ${openStages}), затем «Завершить».`
    : status === 'in_progress' ? 'Когда всё сделано — «Завершить».'
    : status === 'completed' && t.can_archive ? 'Задача выполнена. Уберите её «В архив», когда она больше не нужна в списке.'
    : null;

  const act = async (f: () => Promise<unknown>, after?: string) => {
    setNote('');
    try { await f(); if (after) setNote(after); load(); } catch (e) { setNote((e as Error).message); }
  };
  async function setStatus(to: Status) {
    if (!t) return;
    const was = t.status;
    await act(async () => {
      const r = await work.setStatus(t.id, to);
      if (to === 'completed' && r.open_stages > 0) setTimeout(() => setNote(`Задача завершена, хотя этапов не отмечено: ${r.open_stages}.`), 0);
      // «Отменить» — только если вернуться назад разрешено правилами (в «Ждёт выполнения» нельзя).
      const back = was && statusActions.some((a) => a.to === was && a.from.includes(to));
      toast(`Статус: ${statusName[to]}`, back ? { undo: async () => { await work.setStatus(t.id, was); load(); } } : {});
    });
  }

  return (
    <div className="task-scale -mx-4 -mt-4 flex flex-col sm:-mx-8 lg:-mx-12 lg:h-[calc(100dvh-3.5rem)] lg:flex-row" style={{ '--task-scale': scale } as CSSProperties}>
      {/* Телефон: задача и чат — вкладками, а не одной длинной лентой. */}
      <div className="sticky top-14 z-20 flex border-b border-slate-200 bg-white lg:hidden dark:border-slate-800 dark:bg-slate-900">
        {(['task', 'chat'] as const).map((k) => (
          <button key={k} onClick={() => setTab(k)}
            className={`flex-1 py-2.5 text-sm font-semibold ${tab === k ? 'border-b-2 border-brand-600 text-brand-700 dark:text-brand-100' : 'text-slate-500'}`}>
            {k === 'task' ? 'Задача' : <>Чат{(t.unread ?? 0) > 0 && <span className="ml-1.5 rounded-full bg-brand-600 px-1.5 text-[11px] text-white">{t.unread}</span>}</>}
          </button>
        ))}
      </div>
      {/* Левая панель: поля задачи */}
      <section className={`${tab === 'chat' ? 'hidden' : 'flex'} min-w-0 flex-col bg-slate-100/70 lg:flex lg:w-[44%] lg:max-w-[640px] dark:bg-slate-900/50`}>
        <div className="flex-1 overflow-y-auto px-4 py-4 sm:px-6">
          <nav className="mb-2 flex flex-wrap items-center gap-1 text-sm text-slate-500">
            <span className="mr-2"><BackButton fallback="/" /></span>
            <Link to={`/clients/${t.client_id}`} className="hover:text-brand-600">{t.client_name}</Link>
            {t.path.map((p) => (
              <span key={p.id} className="flex items-center gap-1"><ChevronRight className="size-4" />
                {p.readable ? <Link to={`/tasks/${p.id}`} className="hover:text-brand-600">{p.title}</Link> : <span>{p.title}</span>}
              </span>
            ))}
            <span className="ml-auto"><TextSize scale={scale} onChange={setScale} /></span>
          </nav>
          {t.deadline_state === 'overdue' && (
            <div role="alert" className="mb-3 flex items-center gap-2 rounded-xl bg-red-600 px-4 py-2.5 font-semibold text-white shadow-sm shadow-red-600/30">
              <AlertTriangle className="size-5 shrink-0" />Задача просрочена{t.deadline_at ? ` — срок был ${longDate(t.deadline_at)}` : ''}
            </div>
          )}
          <div className="mb-4 flex items-start gap-3">
            <h1 className="min-w-0 flex-1 text-2xl font-bold tracking-tight [overflow-wrap:anywhere]">{t.title}</h1>
            {t.can_edit && !archived && (
              <Button variant="secondary" className="shrink-0 px-3 py-1.5 text-sm" onClick={() => setEditing(true)} title="Наименование, описание, срок, приоритет">
                <Pencil className="size-4" />Изменить
              </Button>
            )}
          </div>

          <div className="card mb-4 px-5 py-3">
            <Row label="Постановщик"><span className="flex items-center gap-2"><Avatar name={t.created_by.name} size="sm" />{t.created_by.name}</span></Row>
            <Row label={(t.assignees?.length ?? 0) > 1 ? 'Ответственный' : 'Исполнитель'}>
              {t.assignees?.length ? (
                <span className="flex items-center gap-2"><Avatar name={t.assignees[0].name} size="sm" />{t.assignees[0].name}</span>
              ) : <span className="text-slate-400">не назначен</span>}
              {t.can_reassign && !archived && (
                <button className="mt-1.5 flex items-center gap-1 text-sm text-brand-600 hover:underline" onClick={() => setReassigning(true)}>
                  <UserRoundCog className="size-4" />Сменить исполнителя
                </button>
              )}
            </Row>
            {(t.assignees?.length ?? 0) > 1 && (
              <Row label="Соисполнители">
                <span className="flex flex-col gap-1.5">{t.assignees!.slice(1).map((a) => <span key={a.id} className="flex items-center gap-2"><Avatar name={a.name} size="sm" />{a.name}</span>)}</span>
              </Row>
            )}
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

        {/* Что делать дальше — одной строкой над кнопками. */}
        {nextStep && <p className="border-t border-slate-200 bg-brand-50/60 px-4 py-2 text-sm text-brand-800 sm:px-6 dark:border-slate-800 dark:bg-brand-500/10 dark:text-brand-100">→ {nextStep}</p>}
        {/* Нижняя панель действий — как кнопки «Начать / Завершить / ⋯» */}
        <div className="task-actions sticky z-10 flex items-center gap-2 border-t border-slate-200 bg-white px-4 py-3 sm:px-6 lg:bottom-0 dark:border-slate-800 dark:bg-slate-900">
          {primary && <Button onClick={() => setStatus(primary.to)}>{primary.to === 'in_progress' ? 'Начать' : 'Завершить'}</Button>}
          {secondary.map((a) => <Button key={a.to} variant="secondary" onClick={() => setStatus(a.to)}>Завершить</Button>)}
          {archived && t.can_archive && <Button variant="secondary" onClick={() => act(() => work.unarchive(t.id), 'Задача восстановлена из архива.')}><ArchiveRestore className="size-4" />Восстановить</Button>}
          {archived && t.can_archive && t.can_change_status && (
            <Button onClick={() => act(() => work.unarchive(t.id, true), 'Задача восстановлена и возвращена в работу.')}>Восстановить в работу</Button>
          )}
          {!archived && status === 'completed' && t.can_archive && <Button variant="secondary" onClick={() => act(async () => { await work.archive(t.id); toast('Задача убрана в архив', { undo: async () => { await work.unarchive(t.id); load(); } }); })}><Archive className="size-4" />В архив</Button>}
          <Menu button={(toggle) => <Button variant="ghost" onClick={toggle} title="Ещё"><MoreHorizontal className="size-5" /></Button>}>
            {(close) => (
              <>
                {more.map((a) => <MenuItem key={a.label} icon={statusIcon[a.to]} onClick={() => { close(); void setStatus(a.to); }}>{a.label}</MenuItem>)}
                {t.can_edit && !archived && <MenuItem icon={<Pencil className="size-4" />} onClick={() => { close(); setEditing(true); }}>Изменить</MenuItem>}
                {t.can_trash && !archived && <MenuItem danger icon={<Trash2 className="size-4" />} onClick={() => { close(); setTrash(true); }}>В корзину</MenuItem>}
                {!more.length && !(manage && !archived) && !(t.can_trash && !archived) && <p className="px-4 py-2.5 text-sm text-slate-500">Других действий нет</p>}
              </>
            )}
          </Menu>
        </div>
      </section>

      {/* Правая панель: чат задачи */}
      <section className={`${tab === 'task' ? 'hidden' : 'flex'} task-chat min-w-0 flex-col lg:flex lg:flex-1`}>
        <Chat taskId={t.id} canWrite={!archived && (t.access === 'participant' || t.access === 'manage')}
          participants={[t.created_by.name, ...(t.assignees ?? []).map((a) => a.name)]} />
      </section>

      {reassigning && <Reassign t={t} onClose={() => setReassigning(false)} onSaved={() => { setReassigning(false); load(); }} />}
      {editing && <EditTask t={t} onClose={() => setEditing(false)} onSaved={() => { setEditing(false); load(); }} />}
      <TaskForm open={sub} onClose={() => setSub(false)} parent={{ id: t.id, deadline_at: t.deadline_at, title: t.title }} onCreated={(nid) => { setSub(false); navigate(`/tasks/${nid}`); toast('Подзадача создана'); }} />
      <Modal open={trash} onClose={() => setTrash(false)} title="Убрать задачу в корзину?">
        <p className="text-sm text-slate-600 dark:text-slate-300">Вместе с подзадачами и чатом. Вернуть можно из корзины.</p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setTrash(false)}>Отмена</Button>
          <Button variant="danger" onClick={async () => {
            await work.trashTask(t.id);
            navigate(`/clients/${t.client_id}`);
            toast('Задача в корзине', { undo: async () => { await work.restoreTask(t.id); navigate(`/tasks/${t.id}`); } });
          }}>В корзину</Button>
        </div>
      </Modal>
    </div>
  );
}
