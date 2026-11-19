import { useCallback, useEffect, useState } from 'react';
import { Globe, Home, Laptop, Monitor, Server as ServerIcon, Trash2 } from 'lucide-react';
import { api, clearJournal, deleteJournalItem, journal, type JournalItem, type StaffMember } from '../api';
import { useConfirm } from '../components/Confirm';
import { Badge, Button, ErrorText, PageHeader } from '../components/ui';
import { RecoveryCodes } from '../components/RecoveryCodes';
import { SecurityPanel } from '../components/SecurityPanel';
import { TwoFactorPolicyCard } from './AdminWork';

/** Защита входа: код из приложения, коды восстановления владельца, подбор паролей. */
export function AdminProtection() {
  return (
    <>
      <PageHeader title="Защита входа" subtitle="Как подтверждается вход и что делать, если владелец потерял пароль." />
      <div className="flex max-w-3xl flex-col gap-5">
        <TwoFactorPolicyCard />
        <div className="grid gap-5 md:grid-cols-2"><RecoveryCodes /><SecurityPanel /></div>
      </div>
    </>
  );
}

export const placeView: Record<JournalItem['place'], { label: string; icon: typeof Globe; tone: 'slate' | 'green' | 'brand' | 'amber' }> = {
  server: { label: 'на сервере', icon: ServerIcon, tone: 'slate' },
  office: { label: 'офис', icon: Monitor, tone: 'green' },
  device: { label: 'по ключу', icon: Laptop, tone: 'brand' },
  internet: { label: 'интернет', icon: Home, tone: 'amber' },
};

const kinds = [
  { value: '', label: 'Все события' },
  { value: 'login', label: 'Входы и пароли' },
  { value: 'access', label: 'Доступ сотрудников' },
  { value: 'data', label: 'Данные и сервер' },
];

function when(ms: number) {
  return new Date(ms).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

/** Одна строка журнала: что и о ком, ниже — когда, откуда, кто сделал. */
export function JournalRow({ e, onDelete }: { e: JournalItem; onDelete?: () => void }) {
  const place = placeView[e.place];
  const Icon = place.icon;
  const failed = e.action.endsWith('.failed');
  return (
    <li className="group flex items-start gap-2 px-4 py-2.5 text-sm">
      <div className="min-w-0 flex-1">
      <p className="min-w-0">
        <span className={`font-medium ${failed ? 'text-red-700 dark:text-red-300' : ''}`}>{e.title}</span>
        {e.about && <span className="text-slate-500"> · {e.about_name ? `${e.about_name} (@${e.about})` : <>@{e.about} <i>— такого сотрудника нет</i></>}</span>}
      </p>
      <p className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-slate-500">
        <span>{when(e.at)}</span>
        <Badge tone={place.tone}><Icon className="size-3" />{place.label}</Badge>
        {e.ip && e.place !== 'server' && <span className="max-w-48 truncate font-mono" title={e.ip}>{e.ip}</span>}
        {e.actor_name && e.actor_name !== e.about_name && <span>· сделал {e.actor_name}</span>}
      </p>
      </div>
      {onDelete && (
        <button onClick={onDelete} className="rounded-lg p-1.5 text-slate-400 opacity-60 hover:bg-slate-100 hover:text-red-600 group-hover:opacity-100 dark:hover:bg-slate-800" title="Удалить запись" aria-label="Удалить запись">
          <Trash2 className="size-4" />
        </button>
      )}
    </li>
  );
}

/** Журнал безопасности: входы, неудачные попытки, отключения и сбросы — одним списком. */
export function AdminJournal() {
  const [staff, setStaff] = useState<StaffMember[]>([]);
  const [user, setUser] = useState('');
  const [kind, setKind] = useState('');
  const [items, setItems] = useState<JournalItem[] | null>(null);
  const [more, setMore] = useState(false);
  const [error, setError] = useState('');
  const PAGE = 50;
  const ask = useConfirm();

  useEffect(() => void api.staff().then(setStaff).catch(() => {}), []);
  const load = useCallback(async (before?: number) => {
    setError('');
    try {
      const r = await journal({ user, kind, before, limit: PAGE });
      setItems((old) => (before && old ? [...old, ...r.items] : r.items));
      setMore(r.items.length === PAGE);
    } catch (x) { setError((x as Error).message); }
  }, [user, kind]);
  useEffect(() => { setItems(null); void load(); }, [load]);

  return (
    <>
      <PageHeader title="Журнал безопасности" subtitle="Входы, неверные пароли, отключения, сбросы и восстановление. Работа с задачами сюда не попадает. Удалённые записи пропадают из списка, но блокировка за подбор пароля их учитывает." />
      <div className="mb-4 flex flex-wrap gap-2">
        <select className="input w-auto" value={kind} onChange={(e) => setKind(e.target.value)} aria-label="Вид событий">
          {kinds.map((k) => <option key={k.value} value={k.value}>{k.label}</option>)}
        </select>
        <select className="input w-auto" value={user} onChange={(e) => setUser(e.target.value)} aria-label="Сотрудник">
          <option value="">Все сотрудники</option>
          {staff.map((m) => <option key={m.id} value={m.id}>{m.display_name} (@{m.username})</option>)}
        </select>
        {!!items?.length && (
          <Button variant="secondary" className="ml-auto" onClick={async () => {
            const what = [kinds.find((k) => k.value === kind)?.label.toLowerCase(), user && `сотрудника «${staff.find((m) => m.id === user)?.display_name}»`].filter(Boolean).join(', ');
            if (await ask({ title: 'Очистить журнал?', text: <>Удалятся все записи{kind || user ? <> по фильтру: <b>{what}</b></> : ''}. Блокировка за подбор пароля продолжит действовать.</>, ok: 'Удалить', danger: true })) {
              try { await clearJournal({ user, kind }); await load(); } catch (x) { setError((x as Error).message); }
            }
          }}><Trash2 className="size-4" />{kind || user ? 'Удалить показанные' : 'Очистить журнал'}</Button>
        )}
      </div>
      <ErrorText>{error}</ErrorText>
      <div className="card">
        {items === null ? <p className="p-6 text-sm text-slate-500">Загрузка…</p>
          : items.length === 0 ? <p className="p-6 text-sm text-slate-500">Событий нет{user || kind ? ' — попробуйте убрать фильтр' : ''}.</p>
          : <ul className="divide-y divide-slate-100 dark:divide-slate-800">{items.map((e) => (
              <JournalRow key={e.id} e={e} onDelete={async () => {
                try { await deleteJournalItem(e.id); setItems((old) => old?.filter((x) => x.id !== e.id) ?? null); } catch (x) { setError((x as Error).message); }
              }} />
            ))}</ul>}
      </div>
      {more && items && (
        <div className="mt-4 flex justify-center">
          <Button variant="secondary" onClick={() => load(items[items.length - 1].at)}>Показать ещё</Button>
        </div>
      )}
    </>
  );
}
