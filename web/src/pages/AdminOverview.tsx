import { type ReactNode, useCallback, useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import {
  AlertTriangle, Check, CheckCircle2, Circle, DatabaseBackup, Globe, HardDrive, KeyRound, Laptop, Server as ServerIcon, ShieldAlert, UserPlus, Users,
} from 'lucide-react';
import { api, bytes, journal, timeAgo, type BackupStatus, type Health, type JournalItem, type Session, type StaffMember, type StorageInfo } from '../api';
import { work, type NodeDevices, type RemoteInfo } from '../work';
import { Badge, Button, PageHeader, Ago } from '../components/ui';
import { JournalRow } from './AdminSecurity';

/** Откуда сессия — по адресу (как в журнале на сервере). */
export function placeOfIp(ip: string): 'server' | 'device' | 'office' | 'internet' {
  if (!ip || ip === 'local' || ip.startsWith('127.') || ip === '::1') return 'server';
  if (ip.toLowerCase().startsWith('2001:db8:')) return 'device';
  if (/^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|169\.254\.|fe80:|fd|fc)/i.test(ip)) return 'office';
  return 'internet';
}
export const placeLabel = { server: 'на сервере', device: 'по ключу', office: 'в офисе', internet: 'из интернета' } as const;

const REMOTE_DECIDED = 'crm.remote-decided';

interface Data {
  health: Health | null;
  staff: StaffMember[];
  devices: NodeDevices | null;
  storage: StorageInfo | null;
  last: BackupStatus | null;
  recovery: number | null;
  sessions: Session[];
  node: { enabled: boolean; running: boolean } | null;
  remote: RemoteInfo | null;
  ownerCode: boolean;
  blocked: number;
  failed: JournalItem[];
}

const settle = <T,>(p: Promise<T>, fallback: T) => p.catch(() => fallback);

async function loadAll(): Promise<Data> {
  const [health, staff, devices, storage, backups, recovery, sessions, node, remote, me2fa, security, failed] = await Promise.all([
    settle(api.health(), null),
    settle(api.staff(), []),
    settle(work.nodeDevices(), null),
    settle(api.storage(), null),
    settle(api.backups(), { last: null, files: [] }),
    settle(api.recoveryStatus(), null),
    settle(api.sessions(), []),
    settle(work.nodeAccess(), null),
    settle(work.remote(), null),
    settle(api.twoFactor(), null),
    settle(api.security(), { blocked: [], failed_logins: [] }),
    settle(journal({ kind: 'login', limit: 30 }), { items: [] }),
  ]);
  const day = Date.now() - 86_400_000;
  return {
    health, staff, devices, storage, last: backups.last, recovery: recovery?.unused ?? null, sessions, node, remote,
    ownerCode: !!me2fa?.enabled,
    blocked: security.blocked.length,
    failed: failed.items.filter((e) => e.action.endsWith('.failed') && e.at > day),
  };
}

/** Строка «Требует внимания»: что случилось и кнопка, чтобы решить прямо здесь. */
function Attention({ tone = 'amber', icon, children, action }: { tone?: 'amber' | 'red'; icon: ReactNode; children: ReactNode; action?: ReactNode }) {
  const c = tone === 'red' ? 'border-red-300 bg-red-50/70 dark:border-red-500/40 dark:bg-red-500/10' : 'border-amber-300 bg-amber-50/70 dark:border-amber-500/40 dark:bg-amber-500/10';
  return (
    <li className={`flex flex-wrap items-center gap-3 rounded-xl border p-3 ${c}`}>
      <span className="shrink-0">{icon}</span>
      <div className="min-w-0 flex-1 text-sm">{children}</div>
      {action && <div className="flex shrink-0 flex-wrap gap-2">{action}</div>}
    </li>
  );
}

function Tile({ icon, title, value, hint, to, tone }: { icon: ReactNode; title: string; value: ReactNode; hint?: ReactNode; to: string; tone?: 'ok' | 'warn' }) {
  return (
    <Link to={to} className="card flex flex-col gap-1 p-4 transition hover:border-brand-400">
      <span className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-slate-500">{icon}{title}</span>
      <span className={`text-lg font-semibold ${tone === 'warn' ? 'text-amber-700 dark:text-amber-300' : ''}`}>{value}</span>
      {hint && <span className="text-xs text-slate-500">{hint}</span>}
    </Link>
  );
}

export function AdminOverview() {
  const [d, setD] = useState<Data | null>(null);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [remoteDecided, setRemoteDecided] = useState(() => { try { return localStorage.getItem(REMOTE_DECIDED) === '1'; } catch { return false; } });
  const navigate = useNavigate();
  const load = useCallback(() => loadAll().then(setD), []);
  useEffect(() => {
    void load();
    const t = setInterval(load, 30_000);
    return () => clearInterval(t);
  }, [load]);
  const act = async (key: string, f: () => Promise<unknown>) => {
    setBusy(key);
    setError('');
    try { await f(); await load(); } catch (x) { setError((x as Error).message); } finally { setBusy(''); }
  };

  if (!d) return <PageHeader title="Обзор" subtitle="Загрузка…" />;

  // ---------- Требует внимания ----------
  const pending = d.devices?.devices.filter((x) => !x.approved) ?? [];
  const resets = d.staff.filter((m) => m.reset_requested);
  const policy = d.storage?.policy;
  const backupsOn = !!policy?.enabled && (d.storage?.backup_dirs.length ?? 0) > 0;
  const staleAfter = Math.max(26, (policy?.every_hours ?? 24) * 2) * 3_600_000;
  const backupStale = backupsOn && (!d.last || !d.last.ok || Date.now() - d.last.at > staleAfter);
  const lowDisk = d.storage?.volumes.filter((v) => v.level !== 'ok') ?? [];
  const items: ReactNode[] = [];
  for (const dev of pending) {
    items.push(
      <Attention key={`dev-${dev.id}`} icon={<Laptop className="size-5 text-amber-600" />}
        action={<>
          <Button className="px-3 py-1.5 text-sm" busy={busy === dev.id} onClick={() => act(dev.id, () => work.setNodeDevice(dev.id, true, ''))}>Одобрить</Button>
          <Button variant="secondary" className="px-3 py-1.5 text-sm" onClick={() => act(dev.id, () => work.removeNodeDevice(dev.id))}>Отклонить</Button>
        </>}>
        Устройство вне офиса ждёт одобрения: код <b className="font-mono tracking-wider">{dev.code}</b>, запрос <Ago ms={dev.last_seen} />.
        <span className="block text-xs text-slate-500">Одобряйте, только если сотрудник сам назвал вам этот код.</span>
      </Attention>,
    );
  }
  for (const m of resets) {
    items.push(
      <Attention key={`reset-${m.id}`} icon={<KeyRound className="size-5 text-amber-600" />}
        action={<Button className="px-3 py-1.5 text-sm" onClick={() => navigate(`/admin/staff?open=${m.id}`)}>Открыть</Button>}>
        <b>{m.display_name}</b> просит сбросить пароль.
      </Attention>,
    );
  }
  if (!backupsOn) {
    items.push(
      <Attention key="backup-off" tone="red" icon={<DatabaseBackup className="size-5 text-red-600" />} action={<Button className="px-3 py-1.5 text-sm" onClick={() => navigate('/admin/storage')}>Настроить</Button>}>
        Резервные копии не настроены — при поломке диска данные пропадут.
      </Attention>,
    );
  } else if (backupStale) {
    items.push(
      <Attention key="backup-stale" tone="red" icon={<DatabaseBackup className="size-5 text-red-600" />}
        action={<Button className="px-3 py-1.5 text-sm" busy={busy === 'backup'} onClick={() => act('backup', () => api.backupNow())}>Сделать копию сейчас</Button>}>
        {d.last && !d.last.ok ? <>Последняя копия не удалась: {d.last.error}</> : d.last ? <>Давно не было резервной копии: последняя — <Ago ms={d.last.at} />.</> : <>Резервных копий ещё не было.</>}
      </Attention>,
    );
  }
  for (const v of lowDisk) {
    items.push(
      <Attention key={`disk-${v.path}`} tone={v.level === 'critical' ? 'red' : 'amber'} icon={<HardDrive className="size-5 text-amber-600" />} action={<Button variant="secondary" className="px-3 py-1.5 text-sm" onClick={() => navigate('/admin/storage')}>Подробнее</Button>}>
        Заканчивается место на <b>{v.path}</b>: свободно {bytes(v.free_bytes)}{v.days_left != null && ` — хватит примерно на ${v.days_left} дн.`}
      </Attention>,
    );
  }
  if (d.storage?.restart_required) {
    items.push(<Attention key="restart" icon={<ServerIcon className="size-5 text-amber-600" />}>Чтобы изменения хранилища вступили в силу, перезапустите сервер: программа CRM на сервере → «Перезапустить».</Attention>);
  }
  if (d.recovery === 0) {
    items.push(
      <Attention key="recovery" icon={<ShieldAlert className="size-5 text-amber-600" />} action={<Button className="px-3 py-1.5 text-sm" onClick={() => navigate('/admin/protection')}>Создать коды</Button>}>
        Нет кодов восстановления владельца — забыв пароль, вы сможете войти только на самом сервере.
      </Attention>,
    );
  }
  if (d.blocked > 0 || d.failed.length >= 5) {
    items.push(
      <Attention key="attack" tone="red" icon={<ShieldAlert className="size-5 text-red-600" />} action={<Button variant="secondary" className="px-3 py-1.5 text-sm" onClick={() => navigate('/admin/journal')}>Журнал</Button>}>
        {d.blocked > 0 && <>Заблокировано адресов за подбор пароля: <b>{d.blocked}</b>. </>}
        {d.failed.length > 0 && <>Неверных паролей за сутки: <b>{d.failed.length}</b>.</>}
      </Attention>,
    );
  }

  // ---------- Первый запуск ----------
  const remoteOn = !!d.node?.enabled || !!d.remote?.address;
  const steps = [
    { done: (d.recovery ?? 0) > 0, title: 'Коды восстановления владельца', hint: 'Распечатайте и уберите: вход, если забудете пароль.', to: '/admin/protection', button: 'Создать' },
    { done: d.staff.some((m) => m.role !== 'host'), title: 'Первый сотрудник', hint: 'Добавьте сотрудника — он сам задаст себе пароль.', to: '/admin/staff?add=1', button: 'Добавить' },
    { done: backupsOn, title: 'Резервные копии', hint: 'Лучше на второй диск или флешку.', to: '/admin/storage', button: 'Настроить' },
    { done: remoteOn || remoteDecided, title: 'Удалённый доступ', hint: 'Работа из дома и с телефона — или отметьте, что не нужен.', to: '/admin/remote', button: 'Настроить', skip: true },
    { done: d.ownerCode, title: 'Код из приложения для себя', hint: 'Ваш пароль один открывает все данные — защитите его вторым шагом.', to: '/security', button: 'Настроить' },
  ];
  const doneCount = steps.filter((s) => s.done).length;

  // ---------- Сейчас в CRM ----------
  const recent = Date.now() - 15 * 60_000;
  const online = d.sessions.filter((s) => s.last_seen_at > recent);

  return (
    <>
      <PageHeader title="Обзор" subtitle="Что требует внимания и как работает CRM — на одной странице." />
      {error && <p className="mb-4 rounded-xl bg-red-50 px-3.5 py-2.5 text-sm text-red-700 dark:bg-red-500/10 dark:text-red-300">{error}</p>}

      {doneCount < steps.length && (
        <section className="card mb-6 p-5">
          <div className="mb-3 flex flex-wrap items-center gap-3">
            <h2 className="flex-1 font-semibold">Настройка завершена на {doneCount} из {steps.length}</h2>
            <div className="h-2 w-40 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800" role="progressbar" aria-valuenow={doneCount} aria-valuemax={steps.length}>
              <div className="h-full rounded-full bg-brand-600" style={{ width: `${(doneCount / steps.length) * 100}%` }} />
            </div>
          </div>
          <ol className="flex flex-col gap-1">
            {steps.map((s, i) => (
              <li key={s.title} className="flex flex-wrap items-center gap-3 rounded-xl px-2 py-2 hover:bg-slate-50 dark:hover:bg-slate-800/50">
                {s.done ? <CheckCircle2 className="size-5 shrink-0 text-emerald-600" /> : <Circle className="size-5 shrink-0 text-slate-300" />}
                <div className="min-w-0 flex-1">
                  <p className={`text-sm font-medium ${s.done ? 'text-slate-500 line-through' : ''}`}>{i + 1}. {s.title}</p>
                  {!s.done && <p className="text-xs text-slate-500">{s.hint}</p>}
                </div>
                {!s.done && (
                  <span className="flex gap-2">
                    {s.skip && <Button variant="ghost" className="px-3 py-1.5 text-xs" onClick={() => { try { localStorage.setItem(REMOTE_DECIDED, '1'); } catch { /* без памяти */ } setRemoteDecided(true); }}>Не нужен</Button>}
                    <Button variant="secondary" className="px-3 py-1.5 text-sm" onClick={() => navigate(s.to)}>{s.button}</Button>
                  </span>
                )}
              </li>
            ))}
          </ol>
        </section>
      )}

      <section className="mb-6">
        <h2 className="mb-3 flex items-center gap-2 font-semibold">{items.length ? <AlertTriangle className="size-5 text-amber-600" /> : <Check className="size-5 text-emerald-600" />}Требует внимания</h2>
        {items.length ? <ul className="flex flex-col gap-2">{items}</ul> : <p className="card p-4 text-sm text-slate-500">Всё в порядке — ничего делать не нужно.</p>}
      </section>

      <section className="mb-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Tile to="/admin/server" icon={<ServerIcon className="size-4" />} title="Сервер" value={d.health ? 'Работает' : 'Нет ответа'} tone={d.health ? 'ok' : 'warn'} hint={d.health && `версия ${d.health.version}`} />
        <Tile to="/admin/storage" icon={<DatabaseBackup className="size-4" />} title="Копии" tone={backupStale || !backupsOn ? 'warn' : 'ok'}
          value={!backupsOn ? 'Не настроены' : d.last ? (d.last.ok ? timeAgo(d.last.at) : 'Ошибка') : 'Ещё не было'}
          hint={backupsOn && policy ? `каждые ${policy.every_hours} ч · ${d.storage?.backup_dirs[0]}` : undefined} />
        <Tile to="/admin/storage" icon={<HardDrive className="size-4" />} title="Диск" tone={lowDisk.length ? 'warn' : 'ok'}
          value={d.storage?.volumes[0] ? `${bytes(d.storage.volumes[0].free_bytes)} свободно` : '—'}
          hint={d.storage?.volumes[0] && `CRM занимает ${bytes(d.storage.volumes[0].crm_bytes)}`} />
        <Tile to="/admin/remote" icon={<Globe className="size-4" />} title="Удалённый доступ" value={remoteOn ? 'Включён' : 'Только офис'}
          hint={<>{d.node?.enabled ? (d.node.running ? 'по ключу — на связи' : 'по ключу — не запустился') : 'по ключу — выкл.'} · {d.remote?.address ? `адрес ${d.remote.address}` : 'по адресу — выкл.'}</>} />
      </section>

      <div className="grid gap-6 lg:grid-cols-2">
        <section className="card">
          <h2 className="flex items-center gap-2 border-b border-slate-100 px-4 py-3 font-semibold dark:border-slate-800"><Users className="size-4" />Сейчас в CRM <Badge tone="slate">{online.length}</Badge></h2>
          {online.length === 0 ? <p className="p-4 text-sm text-slate-500">За последние 15 минут никого.</p> : (
            <ul className="divide-y divide-slate-100 dark:divide-slate-800">
              {online.map((s) => {
                const place = placeOfIp(s.ip);
                return (
                  <li key={s.id} className="flex items-center gap-3 px-4 py-2.5 text-sm">
                    <span className="min-w-0 flex-1 truncate"><b>{s.display_name}</b> <span className="text-slate-500">· {s.device_name}</span></span>
                    <Badge tone={place === 'internet' ? 'amber' : place === 'office' ? 'green' : 'brand'}>{placeLabel[place]}</Badge>
                    <span className="w-24 text-right text-xs text-slate-500"><Ago ms={s.last_seen_at} /></span>
                  </li>
                );
              })}
            </ul>
          )}
          <Link to="/admin/sessions" className="block border-t border-slate-100 px-4 py-2 text-sm font-medium text-brand-600 hover:underline dark:border-slate-800">Все входы →</Link>
        </section>
        <section className="card">
          <h2 className="flex items-center gap-2 border-b border-slate-100 px-4 py-3 font-semibold dark:border-slate-800"><ShieldAlert className="size-4" />Неверные пароли за сутки <Badge tone={d.failed.length ? 'amber' : 'slate'}>{d.failed.length}</Badge></h2>
          {d.failed.length === 0 ? <p className="p-4 text-sm text-slate-500">Не было.</p> : <ul className="divide-y divide-slate-100 dark:divide-slate-800">{d.failed.slice(0, 6).map((e) => <JournalRow key={e.id} e={e} />)}</ul>}
          <Link to="/admin/journal" className="block border-t border-slate-100 px-4 py-2 text-sm font-medium text-brand-600 hover:underline dark:border-slate-800">Журнал безопасности →</Link>
        </section>
      </div>
      {d.staff.length <= 1 && (
        <div className="mt-6 flex justify-center"><Button onClick={() => navigate('/admin/staff?add=1')}><UserPlus className="size-4" />Добавить первого сотрудника</Button></div>
      )}
    </>
  );
}
