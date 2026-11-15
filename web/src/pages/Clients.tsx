import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import { AlertTriangle, Building2, ChevronRight, FileUp, Mail, Pencil, Phone, Plus, Search, Trash2, User } from 'lucide-react';
import { useAuth } from '../auth';
import { useLiveReload } from '../live';
import { childrenOf, type Client, type ClientCardT, type TaskSummary, work } from '../work';
import { BackButton, Badge, Button, ErrorText, Modal, PageHeader } from '../components/ui';
import { ClientForm, Md, TaskForm, TaskRow, UnreadDot } from '../components/work';
import { ExportMenu } from '../components/ExportMenu';

function TagChip({ name, color }: { name: string; color: string }) {
  return (
    <span className="inline-flex items-center gap-1 rounded-full bg-slate-100 px-2 py-0.5 text-xs font-medium dark:bg-slate-800">
      <span className="size-2 rounded-full" style={{ background: color }} />{name}
    </span>
  );
}

export function Clients() {
  const { me } = useAuth();
  const navigate = useNavigate();
  const [list, setList] = useState<Client[] | null>(null);
  const [q, setQ] = useState('');
  const [tag, setTag] = useState('');
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState('');
  const canCreate = me?.role !== 'employee';
  // Вид запоминается в этом браузере; при 50+ клиентах удобнее таблица.
  const [view, setViewState] = useState<'cards' | 'table'>(() => {
    try { return (localStorage.getItem('crm.clients.view') as 'cards' | 'table') || 'cards'; } catch { return 'cards'; }
  });
  const setView = (v: 'cards' | 'table') => { setViewState(v); try { localStorage.setItem('crm.clients.view', v); } catch { /* без памяти */ } };

  const load = useCallback(() => work.clients({ q, tag }).then(setList).catch((e) => setError(e.message)), [q, tag]);
  useEffect(() => {
    const t = setTimeout(load, 200);
    return () => clearTimeout(t);
  }, [load]);
  useLiveReload(load);

  const manyGroups = new Set((list ?? []).map((c) => c.group_id)).size > 1;
  const tags = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of list ?? []) for (const t of c.tags) m.set(t.name, t.color);
    return [...m.entries()];
  }, [list]);

  return (
    <>
      <PageHeader
        title="Клиенты"
        subtitle={list ? `${list.length} в списке` : 'Загрузка…'}
        action={
          <div className="flex flex-wrap gap-2">
            <ExportMenu url="/api/export/clients" />
            {canCreate && (
              <>
                <Link to="/clients/import" className="inline-flex items-center gap-2 rounded-xl border border-slate-200 bg-white px-4 py-2.5 text-sm font-semibold hover:border-brand-500 dark:border-slate-700 dark:bg-slate-900">
                  <FileUp className="size-4" />Импорт
                </Link>
                <Button onClick={() => setAdding(true)}><Plus className="size-4" />Новый клиент</Button>
              </>
            )}
          </div>
        }
      />
      <div className="mb-4 flex flex-wrap items-center gap-2">
        <div className="relative min-w-60 flex-1">
          <Search className="pointer-events-none absolute left-3.5 top-1/2 size-4 -translate-y-1/2 text-slate-400" />
          <input className="input pl-10" placeholder="Название, телефон, email, описание…" value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
        <div className="flex rounded-xl bg-slate-100 p-0.5 dark:bg-slate-800">
          {(['cards', 'table'] as const).map((v) => (
            <button key={v} onClick={() => setView(v)} className={`rounded-lg px-3 py-1.5 text-sm font-medium ${view === v ? 'bg-white text-brand-700 shadow-sm dark:bg-slate-700 dark:text-white' : 'text-slate-600 dark:text-slate-300'}`}>
              {v === 'cards' ? 'Карточки' : 'Таблица'}
            </button>
          ))}
        </div>
        {tags.length > 0 && (
          <select className="input w-auto" value={tag} onChange={(e) => setTag(e.target.value)}>
            <option value="">Все теги</option>
            {tags.map(([t]) => <option key={t} value={t}>{t}</option>)}
          </select>
        )}
      </div>
      <ErrorText>{error}</ErrorText>
      {list && list.length === 0 && (
        <div className="card flex flex-col items-center px-8 py-16 text-center">
          <Building2 className="size-10 text-brand-500" />
          <p className="mt-4 font-semibold">{q || tag ? 'Ничего не нашлось' : 'Клиентов пока нет'}</p>
          <p className="mt-1 max-w-sm text-sm text-slate-500">
            {canCreate ? 'Заведите клиента вручную или загрузите список из Excel.' : 'Здесь появятся клиенты, по которым вам назначены задачи.'}
          </p>
        </div>
      )}
      {view === 'table' ? (
        list && list.length > 0 && (
          <div className="card overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
                <tr className="border-b border-slate-200 dark:border-slate-800">
                  <th className="px-4 py-2.5">Клиент</th><th className="px-4 py-2.5">Телефон</th><th className="px-4 py-2.5">Ответственный</th>
                  <th className="px-4 py-2.5 text-right">Открыто</th><th className="px-4 py-2.5 text-right">Просрочено</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                {list.map((c) => (
                  <tr key={c.id} className="cursor-pointer hover:bg-slate-50 dark:hover:bg-slate-800/50" onClick={() => navigate(`/clients/${c.id}`)}>
                    <td className="px-4 py-2.5 font-medium"><span className="flex items-center gap-2">{c.kind === 'company' ? <Building2 className="size-4 text-slate-400" /> : <User className="size-4 text-slate-400" />}{c.name}<UnreadDot n={c.counts.unread} /></span></td>
                    <td className="whitespace-nowrap px-4 py-2.5 text-slate-600 dark:text-slate-300">{c.phone ?? '—'}</td>
                    <td className="px-4 py-2.5 text-slate-600 dark:text-slate-300">{c.manager_name ?? '—'}</td>
                    <td className="px-4 py-2.5 text-right tabular-nums">{c.counts.open}</td>
                    <td className={`px-4 py-2.5 text-right tabular-nums ${c.counts.overdue ? 'font-semibold text-red-600' : 'text-slate-400'}`}>{c.counts.overdue}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      ) : (
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {list?.map((c) => (
          <Link key={c.id} to={`/clients/${c.id}`} className="card group flex flex-col gap-2 p-4 transition hover:border-brand-400 hover:shadow-md">
            <div className="flex items-start gap-3">
              <div className="grid size-10 shrink-0 place-items-center rounded-xl bg-brand-50 text-brand-600 dark:bg-brand-500/15">
                {c.kind === 'company' ? <Building2 className="size-5" /> : <User className="size-5" />}
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate font-semibold">{c.name}</p>
                {c.manager_name && <p className="truncate text-xs text-slate-500">Ответственный: {c.manager_name}</p>}
                {manyGroups && <p className="truncate text-xs text-slate-400">{c.group_name}</p>}
              </div>
              <UnreadDot n={c.counts.unread} />
            </div>
            {(c.phone || c.email) && (
              <p className="flex flex-wrap gap-x-3 text-sm text-slate-600 dark:text-slate-300">
                {c.phone && <span className="inline-flex items-center gap-1"><Phone className="size-3.5 text-slate-400" />{c.phone}</span>}
                {c.email && <span className="inline-flex min-w-0 items-center gap-1 truncate"><Mail className="size-3.5 text-slate-400" />{c.email}</span>}
              </p>
            )}
            {c.description && <p className="line-clamp-2 text-sm text-slate-600 dark:text-slate-400">{c.description}</p>}
            <div className="mt-auto flex flex-wrap items-center gap-1.5 pt-1">
              {c.tags.map((t) => <TagChip key={t.name} {...t} />)}
              <span className="ml-auto flex gap-2 text-xs text-slate-500">
                <span>{c.counts.open} открыто</span>
                {c.counts.overdue > 0 && <span className="inline-flex items-center gap-0.5 rounded-full bg-red-50 px-1.5 font-semibold text-red-700 dark:bg-red-500/10 dark:text-red-300"><AlertTriangle className="size-3" />{c.counts.overdue} просрочено</span>}
              </span>
            </div>
          </Link>
        ))}
      </div>
      )}
      <ClientForm open={adding} onClose={() => setAdding(false)} onSaved={(id) => { setAdding(false); navigate(`/clients/${id}`); }} />
    </>
  );
}

/** Дерево задач клиента: подзадачи с отступом. */
export function TaskTree({ tasks }: { tasks: TaskSummary[] }) {
  const by = childrenOf(tasks);
  const render = (parent: string | null, depth: number): React.ReactNode[] =>
    (by.get(parent) ?? []).flatMap((t) => [<TaskRow key={t.id} t={t} depth={depth} />, ...render(t.id, depth + 1)]);
  return <div className="divide-y divide-slate-100 overflow-hidden rounded-xl dark:divide-slate-800">{render(null, 0)}</div>;
}

export function ClientCard() {
  const { id = '' } = useParams();
  const navigate = useNavigate();
  const [c, setC] = useState<ClientCardT | null>(null);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState(false);
  const [adding, setAdding] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [showDone, setShowDone] = useState(false);

  const load = useCallback(() => work.client(id).then(setC).catch((e) => setError(e.message)), [id]);
  useEffect(() => void load(), [load]);
  useLiveReload(load);

  if (error && !c) return <ErrorText>{error}</ErrorText>;
  if (!c) return null;
  const manage = c.access === 'manage';
  const visible = c.tasks.filter((t) => !t.archived_at && (showDone || t.status !== 'completed' || t.access === 'path_only'));

  return (
    <>
      <nav className="mb-3 flex items-center gap-3 text-sm text-slate-500"><BackButton fallback="/clients" /><Link to="/clients" className="hover:text-brand-600">Клиенты</Link><ChevronRight className="size-4" /></nav>
      <PageHeader
        title={c.name}
        subtitle={`${c.kind === 'company' ? 'Компания' : 'Физлицо'} · ${c.group_name}${c.manager_name ? ` · отв. ${c.manager_name}` : ''}`}
        action={manage && (
          <div className="flex gap-2">
            <Button variant="secondary" onClick={() => setEditing(true)}><Pencil className="size-4" />Изменить</Button>
            {c.can_trash && <Button variant="ghost" onClick={() => setConfirm(true)} title="В корзину"><Trash2 className="size-4" /></Button>}
          </div>
        )}
      />
      <div className="grid gap-5 lg:grid-cols-[1fr_300px]">
        <section className="card p-5">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
            <h2 className="font-semibold">Задачи</h2>
            <div className="flex items-center gap-3">
              <label className="flex items-center gap-1.5 text-sm text-slate-500"><input type="checkbox" checked={showDone} onChange={(e) => setShowDone(e.target.checked)} />Завершённые</label>
              <ExportMenu compact url={`/api/export/tasks?client_id=${c.id}`} />
              {manage && <Button className="px-3 py-1.5" onClick={() => setAdding(true)}><Plus className="size-4" />Задача</Button>}
            </div>
          </div>
          {visible.length ? <TaskTree tasks={visible} /> : <p className="py-8 text-center text-sm text-slate-500">Открытых задач нет.</p>}
        </section>
        <aside className="flex flex-col gap-4">
          <div className="card grid grid-cols-3 divide-x divide-slate-100 p-0 text-center dark:divide-slate-800">
            <div className="p-3"><p className="text-xl font-bold">{c.counts.open}</p><p className="text-xs text-slate-500">открыто</p></div>
            <div className="p-3"><p className={`text-xl font-bold ${c.counts.overdue ? 'text-red-600' : ''}`}>{c.counts.overdue}</p><p className="text-xs text-slate-500">просрочено</p></div>
            <div className="p-3"><p className="text-xl font-bold">{c.counts.done_30d}</p><p className="text-xs text-slate-500">за 30 дней</p></div>
          </div>
          <div className="card flex flex-col gap-3 p-5 text-sm">
            {c.phone && <a href={`tel:${c.phone}`} className="flex items-center gap-2 hover:text-brand-600"><Phone className="size-4 text-slate-400" />{c.phone}</a>}
            {c.email && <a href={`mailto:${c.email}`} className="flex items-center gap-2 break-all hover:text-brand-600"><Mail className="size-4 text-slate-400" />{c.email}</a>}
            {c.tags.length > 0 && <div className="flex flex-wrap gap-1.5">{c.tags.map((t) => <TagChip key={t.name} {...t} />)}</div>}
            {c.description ? <Md text={c.description} /> : <p className="text-slate-500">Описания пока нет{manage ? ' — добавьте, кто это и в чём суть работы.' : '.'}</p>}
            {c.access === 'view' && <Badge tone="slate">Только просмотр</Badge>}
          </div>
        </aside>
      </div>
      {editing && <ClientForm open onClose={() => setEditing(false)} initial={c} onSaved={() => { setEditing(false); load(); }} />}
      <TaskForm open={adding} onClose={() => setAdding(false)} clientId={c.id} onCreated={(t) => { setAdding(false); navigate(`/tasks/${t}`); }} />
      <Modal open={confirm} onClose={() => setConfirm(false)} title="Убрать клиента в корзину?">
        <p className="text-sm text-slate-600 dark:text-slate-300">Клиент и его задачи пропадут из списков. Вернуть можно из корзины.</p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setConfirm(false)}>Отмена</Button>
          <Button variant="danger" onClick={async () => { await work.trashClient(c.id); navigate('/clients'); }}>В корзину</Button>
        </div>
      </Modal>
    </>
  );
}
