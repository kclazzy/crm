import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { Check, Copy, Download, KeyRound, Plus, Trash2, UsersRound } from 'lucide-react';
import { timeAgo } from '../api';
import { type Group, type OrgSettings, type VpnInfo, work } from '../work';
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
            Сотрудник вставит ключ в программу CRM («Подключиться по ключу») — она сама найдёт сервер в сети или через VPN и проверит, что он ваш.
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

// ---------- Удалённый доступ (WireGuard) ----------

export function AdminVpn() {
  const people = usePeople();
  const [info, setInfo] = useState<VpnInfo | null>(null);
  const [error, setError] = useState('');
  const [issued, setIssued] = useState<Awaited<ReturnType<typeof work.vpnAdd>> | null>(null);
  const [adding, setAdding] = useState(false);
  const load = useCallback(() => work.vpn().then(setInfo).catch((e) => setError(e.message)), []);
  useEffect(() => void load(), [load]);
  if (!info) return <ErrorText>{error}</ErrorText>;

  return (
    <>
      <PageHeader title="Удалённый доступ (VPN)" subtitle="Работа из дома через WireGuard: через туннель идёт только CRM, интернет сотрудника — как обычно." />
      <ErrorText>{error}</ErrorText>
      <div className="grid gap-5">
        <section className="card p-5">
          <h2 className="mb-1 font-semibold">1. Внешний адрес офиса</h2>
          <p className="mb-4 text-sm text-slate-500">Нужен «белый» IP (узнайте у провайдера; проверить — сайт 2ip.ru) или имя DDNS, если IP меняется (KeenDNS, No-IP).</p>
          <form className="flex flex-wrap gap-2" onSubmit={async (e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            setError('');
            try { await work.vpnSetup(String(f.get('endpoint')), Number(f.get('port'))); load(); } catch (x) { setError((x as Error).message); }
          }}>
            <input name="endpoint" className="input min-w-60 flex-1" placeholder="85.12.34.56 или office.keenetic.pro" defaultValue={info.endpoint ?? ''} required />
            <input name="port" type="number" className="input w-28" defaultValue={info.port} min={1024} max={65535} />
            <Button type="submit">{info.configured ? 'Сохранить' : 'Настроить'}</Button>
          </form>
        </section>
        {info.configured && (
          <section className="card p-5">
            <h2 className="mb-1 font-semibold">2. Включить на этом компьютере</h2>
            <p className="text-sm text-slate-600 dark:text-slate-300">
              Значок CRM в трее → <b>«Применить настройки VPN…»</b> → подтвердите запрос Windows. Программа сама поставит WireGuard и откроет порт.
              Повторяйте после выдачи или отзыва доступа.
            </p>
            <p className="mt-2 text-sm">Туннель: {info.tunnel_up ? <Badge tone="green" dot>работает</Badge> : <Badge tone="amber" dot>не включён</Badge>}</p>
            <p className="mt-3 text-sm text-slate-600 dark:text-slate-300">
              На роутере: перенаправьте <b>UDP-порт {info.port}</b> на этот компьютер (раздел «Переадресация портов» / «Port forwarding»).
            </p>
          </section>
        )}
        {info.configured && (
          <section className="card p-5">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="font-semibold">3. Устройства сотрудников</h2>
              <Button onClick={() => setAdding(true)}><Plus className="size-4" />Выдать доступ</Button>
            </div>
            {info.peers.length === 0 && <p className="text-sm text-slate-500">Пока никому не выдан.</p>}
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {info.peers.map((p) => (
                <li key={p.id} className="flex items-center gap-3 py-2.5 text-sm">
                  <span className="min-w-0 flex-1"><b>{p.name}</b>{p.user_name && ` · ${p.user_name}`}<br />
                    <span className="text-xs text-slate-500">{p.address} · {p.last_seen_at ? `заходил ${timeAgo(p.last_seen_at)}` : 'ещё не подключался'}</span></span>
                  <Button variant="ghost" className="px-2 py-1 text-xs text-red-600" onClick={async () => { await work.vpnRemove(p.id); load(); }}><Trash2 className="size-3.5" />Отозвать</Button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
      <Modal open={adding} onClose={() => setAdding(false)} title="Выдать доступ к VPN">
        <form className="flex flex-col gap-4" onSubmit={async (e) => {
          e.preventDefault();
          const f = new FormData(e.currentTarget);
          try {
            setIssued(await work.vpnAdd(String(f.get('name')), String(f.get('user') || '') || null));
            setAdding(false);
            load();
          } catch (x) { setError((x as Error).message); setAdding(false); }
        }}>
          <Field label="Устройство"><input name="name" className="input" required maxLength={60} placeholder="Ноутбук Ивана" /></Field>
          <Field label="Сотрудник"><select name="user" className="input"><option value="">—</option>{people.map((p) => <option key={p.id} value={p.id}>{p.display_name}</option>)}</select></Field>
          <Button type="submit">Создать</Button>
        </form>
      </Modal>
      <Modal open={!!issued} onClose={() => setIssued(null)} title="Доступ создан">
        {issued && (
          <div className="flex flex-col items-center gap-4 text-center">
            <p className="text-sm text-slate-600 dark:text-slate-300">Показывается один раз. Телефон: приложение WireGuard → «+» → «Сканировать QR-код». Компьютер: WireGuard → «Импорт туннеля из файла».</p>
            {/* QR-код строит сервер (SVG без скриптов) — показываем картинкой. */}
            <img alt="QR-код настроек VPN" className="w-60 rounded-xl bg-white p-2" src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(issued.qr_svg)}`} />
            <Button onClick={() => {
              const a = document.createElement('a');
              a.href = URL.createObjectURL(new Blob([issued.conf], { type: 'text/plain' }));
              a.download = issued.file_name;
              a.click();
            }}><Download className="size-4" />Скачать {issued.file_name}</Button>
            <p className="text-xs text-slate-500">Дальше: в программе CRM сотрудника нажмите «Подключиться» — когда офисная сеть недоступна, она сама пойдёт через VPN.</p>
          </div>
        )}
      </Modal>
    </>
  );
}
