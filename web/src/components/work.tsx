import { type FormEvent, type ReactNode, useEffect, useState } from 'react';
import { Link } from 'react-router';
import { AlertTriangle, CheckCircle2, Circle, Flag, MessageCircle, PlayCircle } from 'lucide-react';
import {
  type ClientInput, type Client, dateTime, deadlineText, fromLocalInput, type Group, type Person, type Priority,
  statusName, type Status, type TaskSummary, toLocalInput, work,
} from '../work';
import { Button, ErrorText, Field, Modal } from './ui';

// ---------- Метки ----------

const priorityStyle: Record<Priority, string> = {
  1: 'text-red-800 bg-red-50 ring-red-700/20 dark:bg-red-500/10 dark:text-red-300',
  2: 'text-orange-700 bg-orange-50 ring-orange-600/20 dark:bg-orange-500/10 dark:text-orange-300',
  3: 'text-slate-600 bg-slate-100 ring-slate-500/20 dark:bg-slate-800 dark:text-slate-300',
};

export function PriorityBadge({ p }: { p: Priority }) {
  return (
    <span className={`inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-bold ring-1 ring-inset ${priorityStyle[p]}`} title={['', 'Высокий', 'Средний', 'Низкий'][p]}>
      <Flag className="size-3" fill={p === 1 ? 'currentColor' : 'none'} />P{p}
    </span>
  );
}

const statusIcon: Record<Status, ReactNode> = {
  new: <Circle className="size-3.5" />,
  in_progress: <PlayCircle className="size-3.5" />,
  completed: <CheckCircle2 className="size-3.5" />,
};

const statusStyle: Record<Status, string> = {
  new: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
  in_progress: 'bg-brand-50 text-brand-700 dark:bg-brand-500/15 dark:text-brand-100',
  completed: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-300',
};

export function StatusBadge({ s }: { s: Status }) {
  return <span className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${statusStyle[s]}`}>{statusIcon[s]}{statusName[s]}</span>;
}

/** Полоса слева у карточки задачи — цвет срока, но всегда рядом есть текст. */
export function deadlineBar(t: TaskSummary) {
  if (t.status === 'new' && t.deadline_state === 'ok') return 'border-l-slate-300 dark:border-l-slate-600';
  return {
    ok: 'border-l-emerald-500',
    due_soon: 'border-l-amber-500',
    overdue: 'border-l-red-600',
    done: 'border-l-transparent',
  }[t.deadline_state ?? 'ok'];
}

export function DeadlineText({ t }: { t: TaskSummary }) {
  if (!t.deadline_at || !t.deadline_state) return null;
  const tone = {
    ok: 'text-slate-500',
    due_soon: 'text-amber-700 dark:text-amber-300 font-medium',
    overdue: 'text-red-700 dark:text-red-300 font-semibold',
    done: 'text-slate-400',
  }[t.deadline_state];
  return (
    <span className={`inline-flex items-center gap-1 text-xs ${tone}`} title={`Дедлайн: ${dateTime(t.deadline_at)}`}>
      {t.deadline_state === 'overdue' && <AlertTriangle className="size-3.5" />}
      {deadlineText(t.deadline_at, t.deadline_state)}
    </span>
  );
}

export function Progress({ done, total }: { done: number; total: number }) {
  if (total === 0) return null;
  const pct = Math.round((done / total) * 100);
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-slate-500" title={`Этапы: ${done} из ${total}`}>
      <span className="h-1.5 w-14 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700">
        <span className="block h-full rounded-full bg-brand-500" style={{ width: `${pct}%` }} />
      </span>
      {done}/{total}
    </span>
  );
}

export function UnreadDot({ n }: { n?: number }) {
  if (!n) return null;
  return (
    <span className="inline-flex min-w-5 items-center justify-center gap-1 rounded-full bg-brand-600 px-1.5 py-0.5 text-[11px] font-bold text-white" title={`Непрочитанных: ${n}`}>
      <MessageCircle className="size-3" />{n}
    </span>
  );
}

/** Строка задачи: в списках, в дереве клиента, в мессенджере. */
export function TaskRow({ t, showClient = false, depth = 0, extra }: { t: TaskSummary; showClient?: boolean; depth?: number; extra?: ReactNode }) {
  if (t.access === 'path_only') {
    return (
      <div className="flex items-center gap-2 py-2 text-sm text-slate-400" style={{ paddingLeft: depth * 22 + 14 }}>
        <span className="truncate">{t.title}</span>
        <span className="text-xs">· путь к вашей подзадаче</span>
      </div>
    );
  }
  const done = t.status === 'completed';
  return (
    <Link
      to={`/tasks/${t.id}`}
      className={`group flex flex-wrap items-center gap-x-3 gap-y-1 border-l-4 bg-white py-2.5 pr-4 transition hover:bg-slate-50 dark:bg-slate-900 dark:hover:bg-slate-800/60 ${deadlineBar(t)} ${done ? 'opacity-60' : ''}`}
      style={{ paddingLeft: depth * 22 + 12 }}
    >
      {t.priority && <PriorityBadge p={t.priority} />}
      <span className={`order-first w-full min-w-0 truncate font-medium sm:order-none sm:w-auto sm:flex-1 ${done ? 'line-through' : ''}`}>
        {t.title}
        {showClient && <span className="ml-2 text-xs font-normal text-slate-500">{t.client_name}</span>}
      </span>
      {extra}
      <UnreadDot n={t.unread} />
      {t.progress && <Progress {...t.progress} />}
      {t.assignees && t.assignees.length > 0 && <span className="hidden max-w-40 truncate text-xs text-slate-500 sm:inline">{t.assignees.map((a) => a.name.split(' ')[0]).join(', ')}</span>}
      {t.status && t.status !== 'in_progress' && <StatusBadge s={t.status} />}
      <DeadlineText t={t} />
    </Link>
  );
}

// ---------- Простой Markdown: жирный, курсив, код, списки, ссылки — без HTML ----------

function inline(text: string, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`|https?:\/\/[^\s)]+)/g;
  let last = 0;
  let i = 0;
  for (const m of text.matchAll(re)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const s = m[0];
    const k = `${key}-${i++}`;
    if (s.startsWith('**')) out.push(<b key={k}>{s.slice(2, -2)}</b>);
    else if (s.startsWith('`')) out.push(<code key={k} className="rounded bg-slate-100 px-1 font-mono text-[0.9em] dark:bg-slate-800">{s.slice(1, -1)}</code>);
    else if (s.startsWith('*')) out.push(<i key={k}>{s.slice(1, -1)}</i>);
    else out.push(<a key={k} href={s} target="_blank" rel="noreferrer noopener" className="text-brand-600 underline break-all">{s}</a>);
    last = at + s.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function Md({ text, className = '' }: { text: string; className?: string }) {
  if (!text.trim()) return null;
  const blocks: ReactNode[] = [];
  let list: string[] = [];
  const flush = (k: number) => {
    if (list.length) {
      blocks.push(<ul key={`l${k}`} className="my-1 list-disc pl-5">{list.map((l, i) => <li key={i}>{inline(l, `li${k}${i}`)}</li>)}</ul>);
      list = [];
    }
  };
  text.split('\n').forEach((line, i) => {
    const m = line.match(/^\s*[-*•]\s+(.*)$/);
    if (m) {
      list.push(m[1]);
      return;
    }
    flush(i);
    if (line.trim() === '') blocks.push(<div key={i} className="h-2" />);
    else blocks.push(<p key={i}>{inline(line, `p${i}`)}</p>);
  });
  flush(-1);
  return <div className={`text-[15px] leading-relaxed break-words ${className}`}>{blocks}</div>;
}

// ---------- Справочники ----------

export function usePeople() {
  const [people, setPeople] = useState<Person[]>([]);
  useEffect(() => void work.people().then(setPeople).catch(() => {}), []);
  return people;
}

export function useGroups() {
  const [groups, setGroups] = useState<Group[]>([]);
  useEffect(() => void work.groups().then(setGroups).catch(() => {}), []);
  return groups;
}

export function PeoplePicker({ people, value, onChange }: { people: Person[]; value: string[]; onChange: (v: string[]) => void }) {
  return (
    <div className="flex max-h-40 flex-wrap gap-1.5 overflow-auto">
      {people.map((p) => {
        const on = value.includes(p.id);
        return (
          <button
            type="button"
            key={p.id}
            onClick={() => onChange(on ? value.filter((x) => x !== p.id) : [...value, p.id])}
            className={`rounded-full border px-3 py-1 text-sm transition ${on ? 'border-brand-500 bg-brand-50 text-brand-700 dark:bg-brand-500/15 dark:text-brand-100' : 'border-slate-200 hover:border-brand-400 dark:border-slate-700'}`}
          >
            {p.display_name}
          </button>
        );
      })}
    </div>
  );
}

// ---------- Формы ----------

export function ClientForm({ open, onClose, initial, onSaved }: { open: boolean; onClose: () => void; initial?: Client; onSaved: (id: string) => void }) {
  const people = usePeople();
  const groups = useGroups();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    const c: ClientInput = {
      kind: f.get('kind') as 'person' | 'company',
      name: String(f.get('name')),
      phone: String(f.get('phone') ?? ''),
      email: String(f.get('email') ?? ''),
      description: String(f.get('description') ?? ''),
      group_id: String(f.get('group_id') || 'default'),
      manager_id: String(f.get('manager_id') ?? ''),
      tags: String(f.get('tags') ?? '').split(',').map((t) => t.trim()).filter(Boolean),
    };
    setBusy(true);
    setError('');
    try {
      if (initial) {
        await work.updateClient(initial.id, c);
        onSaved(initial.id);
      } else {
        onSaved((await work.createClient(c)).id);
      }
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title={initial ? 'Карточка клиента' : 'Новый клиент'} wide>
      <form onSubmit={submit} className="grid gap-4 sm:grid-cols-2">
        <Field label="Название или ФИО">
          <input name="name" className="input" required maxLength={300} defaultValue={initial?.name} autoFocus placeholder="ООО «Ромашка» или Иван Петров" />
        </Field>
        <Field label="Тип">
          <select name="kind" className="input" defaultValue={initial?.kind ?? 'company'}>
            <option value="company">Компания</option>
            <option value="person">Физическое лицо</option>
          </select>
        </Field>
        <div className="sm:col-span-2">
          <Field label="Описание" hint="Кто это, суть работы, реквизиты, адрес. Можно **жирный**, списки через «- ».">
            <textarea name="description" className="input min-h-28" maxLength={20000} defaultValue={initial?.description} />
          </Field>
        </div>
        <Field label="Телефон"><input name="phone" className="input" maxLength={50} defaultValue={initial?.phone ?? ''} placeholder="+7 900 000-00-00" /></Field>
        <Field label="Email"><input name="email" type="email" className="input" maxLength={200} defaultValue={initial?.email ?? ''} /></Field>
        <Field label="Группа">
          <select name="group_id" className="input" defaultValue={initial?.group_id ?? 'default'}>
            {groups.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
          </select>
        </Field>
        <Field label="Ответственный">
          <select name="manager_id" className="input" defaultValue={initial?.manager_id ?? ''}>
            <option value="">— не назначен —</option>
            {people.filter((p) => p.role !== 'employee').map((p) => <option key={p.id} value={p.id}>{p.display_name}</option>)}
          </select>
        </Field>
        <div className="sm:col-span-2">
          <Field label="Теги" hint="Через запятую: VIP, опт, Москва">
            <input name="tags" className="input" defaultValue={initial?.tags.map((t) => t.name).join(', ')} />
          </Field>
        </div>
        <div className="flex flex-col gap-3 sm:col-span-2">
          <ErrorText>{error}</ErrorText>
          <Button type="submit" busy={busy}>{initial ? 'Сохранить' : 'Создать клиента'}</Button>
        </div>
      </form>
    </Modal>
  );
}

/** Новая задача (или подзадача). Предупреждает о дедлайне в нерабочее время. */
export function TaskForm({
  open, onClose, clientId, parent, onCreated,
}: { open: boolean; onClose: () => void; clientId?: string; parent?: { id: string; deadline_at?: number; title: string }; onCreated: (id: string) => void }) {
  const people = usePeople();
  const [assignees, setAssignees] = useState<string[]>([]);
  const [error, setError] = useState('');
  const [warning, setWarning] = useState('');
  const [busy, setBusy] = useState(false);
  const defaultDeadline = parent?.deadline_at ? Math.min(parent.deadline_at, Date.now() + 2 * 86_400_000) : Date.now() + 2 * 86_400_000;
  const [deadline, setDeadline] = useState(toLocalInput(defaultDeadline));
  const [confirmed, setConfirmed] = useState('');

  useEffect(() => {
    if (!open) return;
    const at = fromLocalInput(deadline);
    if (Number.isNaN(at)) return;
    work.checkTime(at).then((r) => setWarning(r.working ? '' : r.reason ?? '')).catch(() => {});
  }, [deadline, open]);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    // «Это суббота — оставить?» — второе нажатие подтверждает.
    if (warning && confirmed !== deadline) {
      setConfirmed(deadline);
      setError('');
      return;
    }
    const f = new FormData(e.currentTarget);
    setBusy(true);
    setError('');
    try {
      const { id } = await work.createTask({
        client_id: clientId,
        parent_id: parent?.id,
        title: String(f.get('title')),
        description: String(f.get('description') ?? ''),
        priority: Number(f.get('priority')) as Priority,
        deadline_at: fromLocalInput(deadline),
        assignees,
        stages: String(f.get('stages') ?? '').split('\n').map((s) => s.trim()).filter(Boolean),
      });
      setAssignees([]);
      onCreated(id);
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title={parent ? `Подзадача к «${parent.title}»` : 'Новая задача'} wide>
      <form onSubmit={submit} className="grid gap-4 sm:grid-cols-2">
        <div className="sm:col-span-2">
          <Field label="Что нужно сделать"><input name="title" className="input" required maxLength={300} autoFocus /></Field>
        </div>
        <Field label="Приоритет">
          <select name="priority" className="input" defaultValue="2">
            <option value="1">P1 — высокий (напоминания за 24, 4 и 1 ч)</option>
            <option value="2">P2 — средний</option>
            <option value="3">P3 — низкий</option>
          </select>
        </Field>
        <Field label="Дедлайн" hint={parent?.deadline_at ? `Не позже дедлайна родителя: ${dateTime(parent.deadline_at)}` : undefined}>
          <input type="datetime-local" className="input" required value={deadline} onChange={(e) => setDeadline(e.target.value)} />
        </Field>
        {warning && (
          <p className="rounded-xl bg-amber-50 px-3.5 py-2.5 text-sm text-amber-800 sm:col-span-2 dark:bg-amber-500/10 dark:text-amber-300">
            Это {warning}. {confirmed === deadline ? 'Нажмите «Создать» ещё раз, чтобы оставить.' : 'Оставить так?'}
          </p>
        )}
        <div className="sm:col-span-2">
          <Field label="Исполнители"><PeoplePicker people={people} value={assignees} onChange={setAssignees} /></Field>
        </div>
        <div className="sm:col-span-2">
          <Field label="Описание"><textarea name="description" className="input min-h-20" maxLength={50000} /></Field>
        </div>
        <div className="sm:col-span-2">
          <Field label="Этапы (чек-лист)" hint="Каждый этап — с новой строки. Прогресс считается по отмеченным.">
            <textarea name="stages" className="input min-h-20" placeholder={'Согласовать ТЗ\nПодписать договор'} />
          </Field>
        </div>
        <div className="flex flex-col gap-3 sm:col-span-2">
          <ErrorText>{error}</ErrorText>
          <Button type="submit" busy={busy}>Создать задачу</Button>
        </div>
      </form>
    </Modal>
  );
}
