import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { Check, Copy, KeyRound, Plus, Trash2, UsersRound, Wand2 } from 'lucide-react';
import { api, type TwoFactorPolicy } from '../api';
import { type Group, type OrgSettings, type RemoteInfo, work } from '../work';
import { Badge, Button, ErrorText, Field, Modal, PageHeader } from '../components/ui';
import { PeoplePicker, usePeople } from '../components/work';

// ---------- Группы ----------

export function AdminGroups() {
  const people = usePeople();
  const [groups, setGroups] = useState<Group[]>([]);
  const [edit, setEdit] = useState<Group | 'new' | null>(null);
  const [members, setMembers] = useState<string[]>([]);
  const [error, setError] = useState('');
  const load = useCallback(() => work.groups().then(setGroups).catch((e) => setError(e.message)), []);
  useEffect(() => void load(), [load]);

  function open(g: Group | 'new') {
    setError('');
    setMembers(g === 'new' ? [] : g.members);
    setEdit(g);
  }

  async function save(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const name = String(new FormData(e.currentTarget).get('name'));
    try {
      if (edit === 'new') await work.createGroup(name, members);
      else if (edit) await work.updateGroup(edit.id, name, members);
      setEdit(null);
      load();
    } catch (x) {
      setError((x as Error).message);
    }
  }

  const name = (id: string) => people.find((p) => p.id === id)?.display_name ?? '—';
  return (
    <>
      <PageHeader title="Группы" subtitle="Руководитель видит клиентов и задачи своих групп. Для 5 человек обычно хватает одной «Основной»."
        action={<Button onClick={() => open('new')}><Plus className="size-4" />Новая группа</Button>} />
      <ErrorText>{!edit && error}</ErrorText>
      <div className="grid gap-3 sm:grid-cols-2">
        {groups.map((g) => (
          <button key={g.id} onClick={() => open(g)} className="card flex flex-col gap-2 p-4 text-left hover:border-brand-400">
            <div className="flex items-center gap-2"><UsersRound className="size-5 text-brand-500" /><b className="flex-1">{g.name}</b><span className="text-xs text-slate-500">клиентов: {g.clients}</span></div>
            <p className="text-sm text-slate-500">{g.members.length ? g.members.map(name).join(', ') : 'Нет участников'}</p>
          </button>
        ))}
      </div>
      <Modal open={!!edit} onClose={() => setEdit(null)} title={edit === 'new' ? 'Новая группа' : 'Группа'}>
        <form onSubmit={save} className="flex flex-col gap-4">
          <Field label="Название"><input name="name" className="input" required maxLength={100} defaultValue={edit && edit !== 'new' ? edit.name : ''} /></Field>
          <Field label="Участники" hint="Новые сотрудники попадают в «Основную» автоматически."><PeoplePicker people={people} value={members} onChange={setMembers} /></Field>
          <ErrorText>{error}</ErrorText>
          <div className="flex justify-between gap-2">
            {edit && edit !== 'new' && edit.id !== 'default' && (
              <Button type="button" variant="ghost" className="text-red-600" onClick={async () => {
                try { await work.deleteGroup(edit.id); setEdit(null); load(); } catch (x) { setError((x as Error).message); }
              }}><Trash2 className="size-4" />Удалить</Button>
            )}
            <Button type="submit" className="ml-auto">Сохранить</Button>
          </div>
        </form>
      </Modal>
    </>
  );
}

// ---------- Рабочее время ----------

const dayNames = ['Пн', 'Вт', 'Ср', 'Чт', 'Пт', 'Сб', 'Вс'];

export function AdminOrg() {
  const [o, setO] = useState<OrgSettings | null>(null);
  const [holiday, setHoliday] = useState('');
  const [msg, setMsg] = useState('');
  const [error, setError] = useState('');
  useEffect(() => void work.org().then(setO).catch((e) => setError(e.message)), []);
  if (!o) return <ErrorText>{error}</ErrorText>;
  const set = (p: Partial<OrgSettings>) => { setO({ ...o, ...p }); setMsg(''); };

  return (
    <>
      <PageHeader title="Рабочее время" subtitle="В нерабочее время напоминания ждут утра, уведомления приходят без звука." />
      <form className="card grid gap-5 p-6 sm:grid-cols-2" onSubmit={async (e) => {
        e.preventDefault();
        setError('');
        try { setO(await work.saveOrg(o)); setMsg('Сохранено'); } catch (x) { setError((x as Error).message); }
      }}>
        <div className="sm:col-span-2">
          <p className="mb-2 text-sm font-medium">Рабочие дни</p>
          <div className="flex flex-wrap gap-1.5">
            {dayNames.map((d, i) => {
              const on = o.work_days.includes(i + 1);
              return <button type="button" key={d} onClick={() => set({ work_days: on ? o.work_days.filter((x) => x !== i + 1) : [...o.work_days, i + 1].sort() })}
                className={`w-12 rounded-xl border py-2 text-sm font-medium ${on ? 'border-brand-500 bg-brand-50 text-brand-700 dark:bg-brand-500/15 dark:text-brand-100' : 'border-slate-200 text-slate-500 dark:border-slate-700'}`}>{d}</button>;
            })}
          </div>
        </div>
        <Field label="Начало дня"><input type="time" className="input" value={o.work_start} onChange={(e) => set({ work_start: e.target.value })} /></Field>
        <Field label="Конец дня"><input type="time" className="input" value={o.work_end} onChange={(e) => set({ work_end: e.target.value })} /></Field>
        <Field label="Часовой пояс" hint="Например Europe/Moscow, Asia/Yekaterinburg">
          <input className="input" value={o.timezone} onChange={(e) => set({ timezone: e.target.value })} />
        </Field>
        <Field label="«Скоро дедлайн» за, часов" hint="Задача желтеет за столько часов до срока">
          <input type="number" min={1} max={168} className="input" value={o.warn_hours} onChange={(e) => set({ warn_hours: Number(e.target.value) })} />
        </Field>
        <Field label="Архив через, дней" hint="Завершённые задачи уходят в архив и не мешают в списках">
          <input type="number" min={1} max={365} className="input" value={o.archive_after_days} onChange={(e) => set({ archive_after_days: Number(e.target.value) })} />
        </Field>
        <div className="sm:col-span-2">
          <p className="mb-2 text-sm font-medium">Праздники и нерабочие дни</p>
          <div className="mb-2 flex flex-wrap gap-1.5">
            {o.holidays.map((h) => <span key={h} className="inline-flex items-center gap-1 rounded-full bg-slate-100 px-2.5 py-1 text-xs dark:bg-slate-800">{new Date(h).toLocaleDateString('ru-RU')}
              <button type="button" onClick={() => set({ holidays: o.holidays.filter((x) => x !== h) })}>×</button></span>)}
          </div>
          <div className="flex gap-2">
            <input type="date" className="input w-auto" value={holiday} onChange={(e) => setHoliday(e.target.value)} />
            <Button type="button" variant="secondary" onClick={() => { if (holiday && !o.holidays.includes(holiday)) set({ holidays: [...o.holidays, holiday].sort() }); setHoliday(''); }}>Добавить</Button>
          </div>
        </div>
        <div className="flex items-center gap-3 sm:col-span-2">
          <Button type="submit">Сохранить</Button>
          {msg && <span className="text-sm text-emerald-600"><Check className="inline size-4" /> {msg}</span>}
        </div>
        <div className="sm:col-span-2"><ErrorText>{error}</ErrorText></div>
      </form>
    </>
  );
}

// ---------- Ключ подключения ----------

export function ConnectionKey({ userId, name, onClose }: { userId: string | null; name?: string; onClose: () => void }) {
  const [pin, setPin] = useState(() => String(Math.floor(1000 + Math.random() * 9000)));
  const [usePin, setUsePin] = useState(true);
  const [key, setKey] = useState<Awaited<ReturnType<typeof work.connectionKey>> | null>(null);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);
  return (
    <Modal open onClose={onClose} title={name ? `Ключ подключения для ${name}` : 'Ключ подключения'}>
      {!key ? (
        <div className="flex flex-col gap-4">
          <p className="text-sm text-slate-600 dark:text-slate-300">
            Сотрудник вставит ключ в программу CRM («Подключиться по ключу») — она сама найдёт сервер в сети офиса или через интернет и проверит, что он ваш.
            {userId ? ' В ключ встроено приглашение: после подключения сотрудник сразу задаст пароль.' : ''}
          </p>
          <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={usePin} onChange={(e) => setUsePin(e.target.checked)} />Защитить PIN (рекомендуется)</label>
          {usePin && <Field label="PIN" hint="Сообщите отдельно от ключа — голосом или в другом мессенджере."><input className="input font-mono text-lg tracking-widest" value={pin} onChange={(e) => setPin(e.target.value)} maxLength={32} /></Field>}
          <ErrorText>{error}</ErrorText>
          <Button onClick={async () => {
            setError('');
            try { setKey(await work.connectionKey(userId, usePin ? pin : null)); } catch (x) { setError((x as Error).message); }
          }}><KeyRound className="size-4" />Создать ключ</Button>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          <div className="max-h-40 overflow-auto rounded-xl border border-slate-200 bg-slate-50 p-3 font-mono text-xs break-all select-all dark:border-slate-700 dark:bg-slate-950">{key.key}</div>
          {key.with_pin && <p className="text-sm">PIN: <b className="font-mono text-lg tracking-widest">{pin}</b> — передайте отдельно.</p>}
          {key.invite_expires_at && <p className="text-xs text-slate-500">Приглашение действует до {new Date(key.invite_expires_at).toLocaleString('ru-RU')}.</p>}
          <p className="text-xs text-slate-500">Адреса в ключе: {key.addrs.join(', ')}</p>
          <Button onClick={async () => { await navigator.clipboard.writeText(key.key); setCopied(true); }}>
            {copied ? <Check className="size-4" /> : <Copy className="size-4" />}{copied ? 'Скопировано' : 'Скопировать ключ'}
          </Button>
        </div>
      )}
    </Modal>
  );
}

// ---------- Удалённый доступ (через интернет) ----------

const policies: { value: TwoFactorPolicy; title: string; hint: string }[] = [
  { value: 'remote', title: 'Код при входе из интернета (рекомендуется)', hint: 'Из дома и с мобильного — пароль и код из приложения. В офисе — только пароль.' },
  { value: 'all', title: 'Код при каждом входе', hint: 'И в офисе, и из интернета. Строже, но код вводят чаще.' },
  { value: 'off', title: 'Не требовать', hint: 'Код — только у тех, кто включил его сам. Если сервер открыт в интернет, так небезопасно.' },
];

/** Когда спрашивать код из приложения. Сотрудник без кода из интернета не войдёт — настроит в офисе. */
function TwoFactorPolicyCard() {
  const [policy, setPolicy] = useState<TwoFactorPolicy | null>(null);
  const [error, setError] = useState('');
  useEffect(() => void api.twoFactorPolicy().then((r) => setPolicy(r.policy)).catch((e) => setError(e.message)), []);
  if (!policy) return <ErrorText>{error}</ErrorText>;
  return (
    <section className="card p-5">
      <h2 className="mb-1 font-semibold">5. Защита входа: код из приложения</h2>
      <p className="mb-3 text-sm text-slate-600 dark:text-slate-300">
        Второй шаг входа — 6 цифр из приложения на телефоне (Яндекс Ключ, Google Authenticator). Каждый настраивает его сам:
        меню <b>«Код для входа»</b>. Кто не настроил — из интернета не войдёт, только в офисе.
      </p>
      <div className="flex flex-col gap-2">
        {policies.map((p) => (
          <label key={p.value} className={`flex cursor-pointer gap-3 rounded-xl border p-3 ${policy === p.value ? 'border-brand-500 bg-brand-50/60 dark:bg-brand-500/10' : 'border-slate-200 dark:border-slate-700'}`}>
            <input type="radio" name="policy" className="mt-1" checked={policy === p.value} onChange={async () => {
              setError('');
              try { setPolicy((await api.setTwoFactorPolicy(p.value)).policy); } catch (x) { setError((x as Error).message); }
            }} />
            <span><b className="text-sm">{p.title}</b><br /><span className="text-sm text-slate-500">{p.hint}</span></span>
          </label>
        ))}
      </div>
      <ErrorText>{error}</ErrorText>
      <p className="mt-3 text-sm text-slate-500">Потерял телефон и резервные коды — «Сотрудники» → «Сбросить код». Владелец без кода: на сервере <code>crm-server reset-owner-password</code>.</p>
    </section>
  );
}

/** Открыть порт на роутере по UPnP — без входа в его настройки. */
function RouterOneClick({ onDone }: { onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [result, setResult] = useState<Awaited<ReturnType<typeof work.routerOpen>> | null>(null);
  return (
    <div className="mt-2 rounded-xl bg-slate-50 p-4 dark:bg-slate-800/50">
      <div className="flex flex-wrap items-center gap-2">
        <Button busy={busy} onClick={async () => {
          setBusy(true);
          setError('');
          setResult(null);
          try { setResult(await work.routerOpen()); onDone(); } catch (x) { setError((x as Error).message); } finally { setBusy(false); }
        }}><Wand2 className="size-4" />Настроить роутер автоматически</Button>
        {result && <Button variant="ghost" onClick={async () => { await work.routerClose(); setResult(null); }}>Убрать правило</Button>}
      </div>
      <p className="mt-2 text-xs text-slate-500">Нужен включённый UPnP на роутере. Правило продлевается само, пока сервер работает.</p>
      <ErrorText>{error}</ErrorText>
      {result && (
        <div className="mt-3 text-sm">
          <p className="text-emerald-700 dark:text-emerald-300">Готово: роутер ведёт порт {result.port} на этот компьютер ({result.local_ip}).</p>
          {result.external_ip && !result.public && (
            <p className="mt-1 text-amber-700 dark:text-amber-300">
              Но роутер получил от провайдера «серый» адрес {result.external_ip} — из интернета до офиса не достучаться.
              Попросите у провайдера «белый» (статический или динамический) IP.
            </p>
          )}
          {result.public && <p className="mt-1">Внешний адрес офиса: <b>{result.external_ip}</b>{result.address === result.external_ip && ' — записан в шаг 1'}.</p>}
        </div>
      )}
    </div>
  );
}

/** Доступ через интернет: внешний адрес офиса, переадресация порта, сотрудники и телефоны. */
export function AdminRemote() {
  const [info, setInfo] = useState<RemoteInfo | null>(null);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState('');
  const load = useCallback(() => work.remote().then(setInfo).catch((e) => setError(e.message)), []);
  useEffect(() => void load(), [load]);
  if (!info) return <ErrorText>{error}</ErrorText>;
  const url = info.address ? `https://${info.address}:${info.port}/` : '';
  return (
    <>
      <PageHeader title="Удалённый доступ" subtitle="Работа из дома и с телефона через интернет — без VPN. Канал зашифрован, программа проверяет, что это именно ваш сервер." />
      <div className="flex max-w-3xl flex-col gap-4">
        <ErrorText>{error}</ErrorText>
        {!info.https && <p className="card border-amber-300 p-4 text-sm text-amber-800 dark:text-amber-200">HTTPS выключен — открывать сервер в интернет нельзя. Включите HTTPS в настройках сервера.</p>}
        <section className="card p-5">
          <h2 className="mb-1 font-semibold">1. Внешний адрес офиса</h2>
          <p className="mb-3 text-sm text-slate-600 dark:text-slate-300">
            «Белый» IP (узнайте у провайдера; проверить — сайт 2ip.ru) или имя DDNS, если IP меняется (KeenDNS, No-IP).
            Оставьте пустым и сохраните, чтобы выключить доступ из интернета.
          </p>
          <form className="flex flex-wrap gap-2" onSubmit={async (e: FormEvent<HTMLFormElement>) => {
            e.preventDefault();
            setError('');
            setSaved('');
            try {
              setInfo(await work.setRemote(String(new FormData(e.currentTarget).get('address'))));
              setSaved('Сохранено. Сертификат сервера обновлён — перезапуск не нужен.');
            } catch (x) { setError((x as Error).message); }
          }}>
            <input key={info.address ?? ""} name="address" className="input min-w-60 flex-1" placeholder="85.12.34.56 или office.keenetic.pro" defaultValue={info.address ?? ""} />
            <Button type="submit">Сохранить</Button>
          </form>
          {saved && <p className="mt-2 text-sm text-emerald-700 dark:text-emerald-300">{saved}</p>}
          <p className="mt-3 text-sm">Сейчас: {info.address ? <Badge tone="green" dot>включён — {info.address}</Badge> : <Badge tone="slate" dot>только сеть офиса</Badge>}</p>
        </section>
        <section className="card p-5">
          <h2 className="mb-1 font-semibold">2. Роутер: переадресация порта</h2>
          <RouterOneClick onDone={load} />
          <p className="mt-4 text-sm text-slate-600 dark:text-slate-300">
            Не получилось автоматически — вручную: в настройках роутера (обычно 192.168.1.1 или 192.168.0.1) создайте правило «Переадресация портов» / «Port forwarding»:
          </p>
          <ul className="mt-2 list-disc pl-5 text-sm">
            <li>протокол <b>TCP</b>, внешний и внутренний порт <b>{info.port}</b>;</li>
            <li>адрес назначения — этот компьютер: <b>{info.office_ip ?? 'IP сервера в сети офиса'}</b>.</li>
          </ul>
          <p className="mt-2 text-sm text-slate-600 dark:text-slate-300">
            Закрепите этот IP за компьютером в роутере («Постоянный IP-адрес» / «Резервирование DHCP»), чтобы правило не сломалось.
            Где искать: Keenetic — «Сетевые правила → Переадресация»; TP-Link — «NAT переадресация → Виртуальные серверы»; ASUS — «Интернет → Переадресация портов».
          </p>
        </section>
        <section className="card p-5">
          <h2 className="mb-1 font-semibold">3. Проверка</h2>
          {info.address ? (
            <p className="text-sm text-slate-600 dark:text-slate-300">
              На телефоне выключите Wi-Fi и откройте <b className="select-all">{url}</b>. Открылась страница входа — всё готово.
              Не открывается: проверьте правило на роутере; если IP на 2ip.ru не совпадает с IP в настройках роутера, у провайдера «серый» адрес — закажите «белый».
            </p>
          ) : <p className="text-sm text-slate-500">Сначала укажите внешний адрес.</p>}
        </section>
        <section className="card p-5">
          <h2 className="mb-1 font-semibold">4. Сотрудники и телефоны</h2>
          <ul className="list-disc pl-5 text-sm text-slate-600 dark:text-slate-300">
            <li>Программа CRM сотрудника узнаёт внешний адрес сама при подключении в офисе — дальше из дома работает без настроек.</li>
            <li>Кто ни разу не подключался в офисе — выдайте новый ключ (<b>Сотрудники → Ключ подключения</b>): адрес уже в нём.</li>
            <li>Телефон: <b>Сервер → Вход с телефона</b> — QR-коды для сертификата и для входа из интернета.</li>
          </ul>
        </section>
        <TwoFactorPolicyCard />
      </div>
    </>
  );
}
