import { useCallback, useEffect, useState } from 'react';
import { Globe, Home, Laptop, Monitor, Server as ServerIcon } from 'lucide-react';
import { api, journal, type JournalItem, type StaffMember } from '../api';
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
export function JournalRow({ e }: { e: JournalItem }) {
  const place = placeView[e.place];
  const Icon = place.icon;
  const failed = e.action.endsWith('.failed');
  return (
    <li className="px-4 py-2.5 text-sm">
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
      <PageHeader title="Журнал безопасности" subtitle="Входы, неверные пароли, отключения, сбросы и восстановление. Работа с задачами сюда не попадает." />
      <div className="mb-4 flex flex-wrap gap-2">
        <select className="input w-auto" value={kind} onChange={(e) => setKind(e.target.value)} aria-label="Вид событий">
          {kinds.map((k) => <option key={k.value} value={k.value}>{k.label}</option>)}
        </select>
        <select className="input w-auto" value={user} onChange={(e) => setUser(e.target.value)} aria-label="Сотрудник">
          <option value="">Все сотрудники</option>
          {staff.map((m) => <option key={m.id} value={m.id}>{m.display_name} (@{m.username})</option>)}
        </select>
      </div>
      <ErrorText>{error}</ErrorText>
      <div className="card">
        {items === null ? <p className="p-6 text-sm text-slate-500">Загрузка…</p>
          : items.length === 0 ? <p className="p-6 text-sm text-slate-500">Событий нет{user || kind ? ' — попробуйте убрать фильтр' : ''}.</p>
          : <ul className="divide-y divide-slate-100 dark:divide-slate-800">{items.map((e) => <JournalRow key={e.id} e={e} />)}</ul>}
      </div>
      {more && items && (
        <div className="mt-4 flex justify-center">
          <Button variant="secondary" onClick={() => load(items[items.length - 1].at)}>Показать ещё</Button>
        </div>
      )}
    </>
  );
}
