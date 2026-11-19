import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router';
import { Check, Copy, Globe, Laptop, LogOut, Monitor, RotateCcw, Search, Smartphone, Trash2, UserCheck, UserPlus, UsersRound, UserX, X } from 'lucide-react';
import { ConnectionKey } from './AdminWork';
import { api, journal, type Invite, type JournalItem, type Role, roleName, type Session, type StaffMember, timeAgo } from '../api';
import { Avatar, Badge, Button, ErrorText, Field, Modal, PageHeader } from '../components/ui';
import { useConfirm } from '../components/Confirm';
import { Hint } from '../components/Hint';
import { JournalRow } from './AdminSecurity';
import { placeLabel, placeOfIp } from './AdminOverview';

const filters: { id: string; label: string; test: (m: StaffMember) => boolean }[] = [
  { id: 'all', label: 'Все', test: () => true },
  { id: 'active', label: 'Активные', test: (m) => m.state === 'active' },
  { id: 'invited', label: 'Ждут входа', test: (m) => m.state === 'invited' },
  { id: 'disabled', label: 'Отключены', test: (m) => m.state === 'disabled' },
  { id: 'nocode', label: 'Без кода из приложения', test: (m) => m.state === 'active' && !m.two_factor },
];

/** «Код · локальная сеть» и «Ключ · интернет» рядом. */
function ConnectButtons({ m, onCode, onKey }: { m: StaffMember; onCode: () => void; onKey: () => void }) {
  return (
    <>
      <Button variant="secondary" className="flex-col gap-0 px-3 py-1 text-xs leading-tight" onClick={onCode} title="Адрес сервера и 8 символов — для программы CRM в офисе, без интернета">
        <span className="flex items-center gap-1"><Monitor className="size-3.5" />Код</span>
        <span className="text-[10px] font-normal text-slate-500">локальная сеть</span>
      </Button>
      <Button variant="secondary" className="flex-col gap-0 px-3 py-1 text-xs leading-tight" onClick={onKey} title={m.state === 'invited' ? 'Ключ с приглашением — новый сотрудник сам задаст пароль' : 'Ключ подключения — вход своим паролем'}>
        <span className="flex items-center gap-1"><Globe className="size-3.5" />Ключ</span>
        <span className="text-[10px] font-normal text-slate-500">интернет</span>
      </Button>
    </>
  );
}

/** Карточка сотрудника сбоку: подключение, код из приложения, входы, события, действия. */
function StaffCard({ m, onClose, onChanged, onCode, onKey, onReset, onDelete }: {
  m: StaffMember; onClose: () => void; onChanged: () => void; onCode: () => void; onKey: () => void; onReset: () => void; onDelete: () => void;
}) {
  const ask = useConfirm();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [events, setEvents] = useState<JournalItem[] | null>(null);
  const [error, setError] = useState('');
  const load = useCallback(() => {
    api.sessions().then((all) => setSessions(all.filter((s) => s.username === m.username))).catch(() => {});
    journal({ user: m.id, limit: 8 }).then((r) => setEvents(r.items)).catch(() => setEvents([]));
  }, [m.id, m.username]);
  useEffect(() => {
    load();
    const onKeyDown = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [load, onClose]);
  const act = async (f: () => Promise<unknown>) => {
    setError('');
    try { await f(); load(); onChanged(); } catch (x) { setError((x as Error).message); }
  };
  const host = m.role === 'host';
  return createPortal(
    <div className="fixed inset-0 z-40 bg-slate-950/30" onMouseDown={onClose}>
      <aside className="animate-rise absolute inset-y-0 right-0 flex w-full max-w-md flex-col overflow-y-auto bg-white shadow-2xl dark:bg-slate-900" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label={`Сотрудник ${m.display_name}`}>
        <div className="flex items-start gap-3 border-b border-slate-100 p-5 dark:border-slate-800">
          <Avatar name={m.display_name} />
          <div className="min-w-0 flex-1">
            <p className="text-lg font-semibold">{m.display_name}</p>
            <p className="text-sm text-slate-500">@{m.username} · {roleName[m.role]}</p>
            <div className="mt-2 flex flex-wrap gap-1.5">{stateBadge[m.state]}{m.reset_requested && <Badge tone="amber">Просит сбросить пароль</Badge>}</div>
          </div>
          <button onClick={onClose} className="rounded-lg p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-700 dark:hover:bg-slate-800" aria-label="Закрыть"><X className="size-5" /></button>
        </div>
        <div className="flex flex-col gap-6 p-5">
          <ErrorText>{error}</ErrorText>
          {!host && m.state !== 'disabled' && (
            <section>
              <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">Подключить программу</h3>
              <div className="flex gap-2"><ConnectButtons m={m} onCode={onCode} onKey={onKey} /></div>
              {m.state === 'invited' && <p className="mt-2 text-xs text-slate-500">Ещё не задал пароль: ключ для интернета будет с приглашением.</p>}
            </section>
          )}
          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">Код из приложения</h3>
            <div className="flex flex-wrap items-center gap-2 text-sm">
              {m.two_factor ? <Badge tone="green" dot>настроен</Badge> : <Badge tone="slate" dot>не настроен</Badge>}
              {m.two_factor && !host && (
                <Button variant="secondary" className="px-3 py-1 text-xs" onClick={async () => {
                  if (await ask({ title: 'Сбросить код из приложения?', text: <>У «{m.display_name}» перестанет работать нынешний код. Новый он настроит в офисе или в программе на одобренном компьютере.</>, ok: 'Сбросить код', danger: true })) {
                    await act(() => api.twoFactorReset(m.id));
                  }
                }}><Smartphone className="size-3.5" />Сбросить код</Button>
              )}
            </div>
          </section>
          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">Где сейчас вошёл · {sessions.length}</h3>
            {sessions.length === 0 ? <p className="text-sm text-slate-500">Нигде.</p> : (
              <ul className="flex flex-col gap-2">
                {sessions.map((s) => {
                  const place = placeOfIp(s.ip);
                  return (
                    <li key={s.id} className="flex items-center gap-2 rounded-xl border border-slate-200 p-2.5 text-sm dark:border-slate-700">
                      <Laptop className="size-4 shrink-0 text-slate-400" />
                      <span className="min-w-0 flex-1"><span className="block truncate">{s.device_name}</span><span className="text-xs text-slate-500">{placeLabel[place]} · {timeAgo(s.last_seen_at)}</span></span>
                      <Button variant="ghost" className="px-2 py-1 text-xs" onClick={async () => {
                        if (await ask({ title: 'Отключить устройство?', text: <>«{s.device_name}»: вход закроется сразу, вернуться можно только с паролем.</>, ok: 'Отключить', danger: true })) {
                          await act(() => api.revokeSession(s.id));
                        }
                      }}><LogOut className="size-3.5" />Отключить</Button>
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">Последние события</h3>
            {events === null ? <p className="text-sm text-slate-500">Загрузка…</p> : events.length === 0 ? <p className="text-sm text-slate-500">Пока ничего.</p> : (
              <ul className="-mx-4 divide-y divide-slate-100 dark:divide-slate-800">{events.map((e) => <JournalRow key={e.id} e={e} />)}</ul>
            )}
            <Link to="/admin/journal" className="mt-1 inline-block text-sm font-medium text-brand-600 hover:underline">Весь журнал →</Link>
          </section>
          {!host && (
            <section className="flex flex-wrap gap-2 border-t border-slate-100 pt-5 dark:border-slate-800">
              <Button variant="secondary" onClick={onReset}><RotateCcw className="size-4" />Сбросить доступ</Button>
              {m.state === 'disabled'
                ? <Button variant="secondary" onClick={() => act(() => api.enableUser(m.id))}><UserCheck className="size-4" />Включить</Button>
                : <Button variant="secondary" onClick={async () => {
                    if (await ask({ title: `Отключить «${m.display_name}»?`, text: 'Он сразу выйдет на всех устройствах и не сможет войти. Задачи и данные останутся, включить можно в любой момент.', ok: 'Отключить', danger: true })) {
                      await act(() => api.disableUser(m.id));
                    }
                  }}><UserX className="size-4" />Отключить</Button>}
              <Button variant="danger" onClick={onDelete}><Trash2 className="size-4" />Удалить…</Button>
            </section>
          )}
        </div>
      </aside>
    </div>,
    document.body,
  );
}

const stateBadge = {
  active: <Badge tone="green" dot>Активен</Badge>,
  invited: <Badge tone="amber" dot>Ждёт входа</Badge>,
  disabled: <Badge tone="slate" dot>Отключён</Badge>,
};

/** Удалить сотрудника: открытые задачи и клиенты — выбранному коллеге; история остаётся. */
function DeleteUser({ m, staff, onClose, onDone }: { m: StaffMember; staff: StaffMember[]; onClose: () => void; onDone: () => void }) {
  const heirs = staff.filter((s) => s.id !== m.id && s.state === 'active');
  const [heir, setHeir] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal open onClose={onClose} title={`Удалить «${m.display_name}»?`}>
      <div className="flex flex-col gap-4 text-sm">
        <p className="text-slate-600 dark:text-slate-300">
          Сотрудник исчезнет из списков и не сможет войти; его логин освободится. Сообщения и завершённые задачи
          сохранятся с пометкой «(удалён)». Если нужно лишь временно закрыть доступ — выберите «Отключить».
        </p>
        <Field label="Кому передать открытые задачи и клиентов">
          <select className="input" value={heir} onChange={(e) => setHeir(e.target.value)}>
            <option value="">— не передавать (если их нет) —</option>
            {heirs.map((h) => <option key={h.id} value={h.id}>{h.display_name}</option>)}
          </select>
        </Field>
        <ErrorText>{error}</ErrorText>
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={onClose}>Отмена</Button>
          <Button variant="danger" busy={busy} onClick={async () => {
            setBusy(true);
            setError('');
            try { await api.deleteUser(m.id, heir || undefined); onDone(); } catch (x) { setError((x as Error).message); } finally { setBusy(false); }
          }}><Trash2 className="size-4" />Удалить</Button>
        </div>
      </div>
    </Modal>
  );
}

/** Код для программы в офисе: сотрудник вводит адрес сервера и 8 символов — сам, без интернета. */
function OfficeCode({ userId, name }: { userId: string; name: string }) {
  const [c, setC] = useState<{ code: string; expires_at: number; name: string; addrs: string[]; port: number } | null>(null);
  const [error, setError] = useState('');
  const load = useCallback(() => { setError(''); api.enrollCode(userId).then(setC).catch((e) => setError(e.message)); }, [userId]);
  useEffect(() => { load(); }, [load]);
  if (!c) return <ErrorText>{error}</ErrorText>;
  // Имя компьютера показано крупно; ниже — только IP-адреса на случай, если имя не находится.
  const ips = c.addrs.filter((a) => /^\d+\.\d+\.\d+\.\d+$/.test(a));
  const until = new Date(c.expires_at).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' });
  return (
    <div className="flex flex-col gap-3 rounded-xl border border-brand-200 bg-brand-50/60 p-4 dark:border-brand-500/30 dark:bg-brand-500/10">
      <p className="flex items-center gap-2 font-semibold"><Monitor className="size-4" />В офисе — программа CRM, без интернета</p>
      <div className="grid gap-3 sm:grid-cols-2">
        <div>
          <p className="text-xs text-slate-500">Адрес сервера</p>
          <p className="font-mono text-lg font-semibold select-all">{c.name}</p>
          {ips.length > 0 && <p className="text-xs text-slate-500">или {ips.join(', ')}</p>}
        </div>
        <div>
          <p className="text-xs text-slate-500">Код для офиса · {name} <Hint word="Код для офиса" /></p>
          <p className="font-mono text-2xl font-bold tracking-widest select-all">{c.code}</p>
          <p className="text-xs text-slate-500">действует до {until}</p>
        </div>
      </div>
      <ol className="list-decimal pl-5 text-sm text-slate-600 dark:text-slate-300">
        <li>Установите программу CRM: на компьютере сотрудника откройте в браузере <span className="font-mono select-all">https://{c.name}:{c.port}</span> и внизу страницы входа нажмите «Скачать программу CRM». Браузер предупредит о сертификате — «Дополнительно» → «Перейти».</li>
        <li>В программе: «Подключиться» → «🏢 В офисе» → введите адрес и код.</li>
        <li>Сотрудник сразу задаёт себе пароль — вы его не узнаете.</li>
      </ol>
      <p className="text-xs text-slate-500">Код можно продиктовать. Подсмотреть его по сети нельзя, после 10 неверных попыток он сгорает. <button className="font-medium text-brand-600 hover:underline" onClick={load}>Новый код</button></p>
    </div>
  );
}

function InviteResult({ name, invite, userId, onKey }: { name: string; invite: Invite; userId?: string; onKey?: () => void }) {
  const link = `${location.origin}/invite?code=${encodeURIComponent(invite.invite_token)}`;
  const [copied, setCopied] = useState(false);
  const until = new Date(invite.invite_expires_at).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' });
  return (
    <div className="flex flex-col gap-4">
      {userId && <OfficeCode userId={userId} name={name} />}
      {onKey && (
        <Button variant="secondary" onClick={onKey}><Globe className="size-4" />Сотрудник не в офисе? Ключ для интернета</Button>
      )}
      <p className="text-sm text-slate-600 dark:text-slate-300">
        {userId ? 'Или в браузере: отправьте ссылку' : 'Отправьте ссылку'} <b>{name}</b> лично. По ней сотрудник задаст свой пароль — вы его не узнаете. Ссылка одноразовая и действует до <b>{until}</b>.
      </p>
      <div className="rounded-xl border border-slate-200 bg-slate-50 p-3 font-mono text-xs break-all select-all dark:border-slate-700 dark:bg-slate-950">{link}</div>
      <Button
        onClick={async () => {
          await navigator.clipboard.writeText(link);
          setCopied(true);
        }}
      >
        {copied ? <Check className="size-4" /> : <Copy className="size-4" />} {copied ? 'Скопировано' : 'Скопировать ссылку'}
      </Button>
    </div>
  );
}

export function AdminStaff() {
  const [staff, setStaff] = useState<StaffMember[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [result, setResult] = useState<{ name: string; invite: Invite; userId?: string } | null>(null);
  const [codeFor, setCodeFor] = useState<StaffMember | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [resetting, setResetting] = useState<StaffMember | null>(null);
  const [deleting, setDeleting] = useState<StaffMember | null>(null);
  // Ключ для интернета: с приглашением (у сотрудника ещё нет пароля) или общий.
  const [keyFor, setKeyFor] = useState<{ userId: string | null; name?: string } | null>(null);
  const keyOf = (m: StaffMember) => setKeyFor({ userId: m.state === 'invited' ? m.id : null, name: m.display_name });

  const load = useCallback(() => api.staff().then(setStaff).catch((e) => setError(e.message)), []);
  useEffect(() => void load(), [load]);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');
  // Карточка сотрудника и «Добавить» открываются и по ссылке: /admin/staff?open=id, ?add=1 (из «Обзора»).
  const [params, setParams] = useSearchParams();
  const openId = params.get('open');
  const setOpenId = (id: string | null) => setParams(id ? { open: id } : {}, { replace: true });
  useEffect(() => {
    if (params.get('add')) { setAdding(true); setParams({}, { replace: true }); }
  }, [params, setParams]);
  const open = staff?.find((m) => m.id === openId) ?? null;
  const f = filters.find((x) => x.id === filter) ?? filters[0];
  const q = query.trim().toLowerCase();
  const shown = staff?.filter(f.test).filter((m) => !q || m.display_name.toLowerCase().includes(q) || m.username.toLowerCase().includes(q));

  async function add(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    setBusy(true);
    setError('');
    try {
      const name = String(f.get('display_name'));
      const r = await api.addStaff(String(f.get('username')), name, f.get('role') as Role);
      setAdding(false);
      setResult({ name, invite: r, userId: r.user.id });
      load();
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function reset(m: StaffMember) {
    setBusy(true);
    try {
      const invite = await api.resetAccess(m.id);
      setResetting(null);
      setResult({ name: m.display_name, invite, userId: m.id });
      load();
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const counts = staff ? { active: staff.filter((s) => s.state === 'active').length, invited: staff.filter((s) => s.state === 'invited').length } : null;

  return (
    <>
      <PageHeader
        title="Сотрудники"
        subtitle={counts ? `${counts.active} активных · ${counts.invited} ждут первого входа` : 'Загрузка…'}
        action={
          <div className="flex gap-2">
            <Button variant="secondary" onClick={() => setKeyFor({ userId: null })} title="Общий ключ для тех, у кого уже есть пароль"><Globe className="size-4" />Ключ для интернета</Button>
            <Button onClick={() => { setError(''); setAdding(true); }}><UserPlus className="size-4" />Добавить сотрудника</Button>
          </div>
        }
      />
      {error && !adding && <div className="mb-4"><ErrorText>{error}</ErrorText></div>}
      {staff && staff.filter((m) => m.role !== 'host').length === 0 ? (
        <div className="card flex flex-col items-center gap-3 p-10 text-center">
          <UsersRound className="size-10 text-brand-500" />
          <p className="text-lg font-semibold">Сотрудников пока нет</p>
          <p className="max-w-md text-sm text-slate-500">Добавьте первого: вы получите код для программы в офисе или ключ для подключения из дома — пароль сотрудник задаст себе сам.</p>
          <Button onClick={() => { setError(''); setAdding(true); }}><UserPlus className="size-4" />Добавить первого сотрудника</Button>
        </div>
      ) : (
        <>
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <div className="relative min-w-52 flex-1 sm:max-w-xs">
              <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-slate-400" />
              <input className="input pl-9" placeholder="Имя или логин" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Поиск сотрудника" />
            </div>
            <div className="flex flex-wrap gap-1">
              {filters.map((f) => (
                <button key={f.id} onClick={() => setFilter(f.id)}
                  className={`rounded-full px-3 py-1.5 text-xs font-medium ${filter === f.id ? 'bg-brand-600 text-white' : 'bg-slate-100 text-slate-600 hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300'}`}>
                  {f.label}{staff && ` · ${staff.filter(f.test).length}`}
                </button>
              ))}
            </div>
          </div>
          <ul className="card divide-y divide-slate-100 dark:divide-slate-800">
            {shown?.map((m) => (
              <li key={m.id} className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 transition hover:bg-slate-50/70 dark:hover:bg-slate-800/40">
                <button className="flex min-w-48 flex-1 items-center gap-3 text-left" onClick={() => setOpenId(m.id)} title="Карточка сотрудника">
                  <Avatar name={m.display_name} />
                  <span className="min-w-0">
                    <span className="block truncate font-semibold hover:text-brand-600">{m.display_name}</span>
                    <span className="block text-sm text-slate-500">@{m.username} · {roleName[m.role]}</span>
                  </span>
                </button>
                <div className="ml-auto flex flex-wrap gap-1.5">{stateBadge[m.state]}{m.two_factor && <Badge tone="green">код ✓</Badge>}</div>
                {m.role !== 'host' && m.state !== 'disabled' && (
                  <span className="inline-flex shrink-0 gap-1">
                    {m.reset_requested && (
                      <Button className="flex-col gap-0 px-3 py-1 text-xs leading-tight" onClick={() => setResetting(m)} title="Сотрудник забыл пароль и просит сбросить">
                        <span className="flex items-center gap-1"><RotateCcw className="size-3.5" />Сбросить</span>
                        <span className="text-[10px] font-normal opacity-80">просит пароль</span>
                      </Button>
                    )}
                    <ConnectButtons m={m} onCode={() => setCodeFor(m)} onKey={() => keyOf(m)} />
                  </span>
                )}
              </li>
            ))}
            {shown?.length === 0 && <li className="p-6 text-center text-sm text-slate-500">Никого не нашлось — измените поиск или фильтр.</li>}
          </ul>
        </>
      )}

      {open && (
        <StaffCard
          m={open}
          onClose={() => setOpenId(null)}
          onChanged={load}
          onCode={() => setCodeFor(open)}
          onKey={() => keyOf(open)}
          onReset={() => setResetting(open)}
          onDelete={() => setDeleting(open)}
        />
      )}

      <Modal open={adding} onClose={() => setAdding(false)} title="Новый сотрудник">
        <form onSubmit={add} className="flex flex-col gap-4">
          <Field label="Имя и фамилия"><input name="display_name" className="input" required maxLength={100} placeholder="Иван Петров" autoFocus /></Field>
          <Field label="Логин" hint="Латинские буквы, цифры, точка, дефис; 3–32 символа">
            <input name="username" className="input" required pattern="[A-Za-z0-9._\-]{3,32}" placeholder="ivan" spellCheck={false} />
          </Field>
          <Field label="Роль">
            <select name="role" className="input" defaultValue="employee">
              <option value="employee">Сотрудник — видит свои задачи</option>
              <option value="manager">Руководитель — клиенты и задачи своей группы</option>
            </select>
          </Field>
          <ErrorText>{error}</ErrorText>
          <Button type="submit" busy={busy}>Создать приглашение</Button>
        </form>
      </Modal>

      <Modal open={!!resetting} onClose={() => setResetting(null)} title="Сбросить доступ?">
        <p className="text-sm text-slate-600 dark:text-slate-300">
          {resetting?.display_name} будет отключён на всех устройствах, старый пароль перестанет работать. Вы получите новую ссылку-приглашение.
        </p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setResetting(null)}>Отмена</Button>
          <Button variant="danger" busy={busy} onClick={() => resetting && reset(resetting)}>Сбросить</Button>
        </div>
      </Modal>

      {deleting && <DeleteUser m={deleting} staff={staff ?? []} onClose={() => setDeleting(null)} onDone={() => { setDeleting(null); void load(); }} />}

      {keyFor && <ConnectionKey userId={keyFor.userId} name={keyFor.name} onClose={() => setKeyFor(null)} />}

      <Modal open={!!codeFor} onClose={() => setCodeFor(null)} title={`Подключить программу — ${codeFor?.display_name ?? ''}`}>
        {codeFor && <OfficeCode userId={codeFor.id} name={codeFor.display_name} />}
      </Modal>

      <Modal open={!!result} onClose={() => setResult(null)} title="Приглашение готово">
        {result && <InviteResult {...result} onKey={result.userId ? () => { const r = result; setResult(null); setKeyFor({ userId: r.userId ?? null, name: r.name }); } : undefined} />}
      </Modal>
    </>
  );
}
