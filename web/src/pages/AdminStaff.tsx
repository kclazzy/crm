import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { Check, Copy, KeyRound, MoreHorizontal, RotateCcw, Smartphone, Trash2, UserCheck, UserPlus, UserX } from 'lucide-react';
import { Menu, MenuItem } from '../components/Menu';
import { ConnectionKey } from './AdminWork';
import { api, type Invite, type Role, roleName, type StaffMember } from '../api';
import { Avatar, Badge, Button, ErrorText, Field, Modal, PageHeader } from '../components/ui';
import { RecoveryCodes } from '../components/RecoveryCodes';

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

function InviteResult({ name, invite }: { name: string; invite: Invite }) {
  const link = `${location.origin}/invite?code=${encodeURIComponent(invite.invite_token)}`;
  const [copied, setCopied] = useState(false);
  const until = new Date(invite.invite_expires_at).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' });
  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-slate-600 dark:text-slate-300">
        Отправьте ссылку <b>{name}</b> лично. По ней сотрудник задаст свой пароль — вы его не узнаете. Ссылка одноразовая и действует до <b>{until}</b>.
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
  const [result, setResult] = useState<{ name: string; invite: Invite } | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [resetting, setResetting] = useState<StaffMember | null>(null);
  const [deleting, setDeleting] = useState<StaffMember | null>(null);
  const [keyFor, setKeyFor] = useState<StaffMember | 'all' | null>(null);

  const load = useCallback(() => api.staff().then(setStaff).catch((e) => setError(e.message)), []);
  useEffect(() => void load(), [load]);

  async function add(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    setBusy(true);
    setError('');
    try {
      const name = String(f.get('display_name'));
      const r = await api.addStaff(String(f.get('username')), name, f.get('role') as Role);
      setAdding(false);
      setResult({ name, invite: r });
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
      setResult({ name: m.display_name, invite });
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
            <Button variant="secondary" onClick={() => setKeyFor('all')}><KeyRound className="size-4" />Ключ подключения</Button>
            <Button onClick={() => { setError(''); setAdding(true); }}><UserPlus className="size-4" />Добавить сотрудника</Button>
          </div>
        }
      />
      <RecoveryCodes compact />
      {error && !adding && <div className="mb-4"><ErrorText>{error}</ErrorText></div>}
      <div className="card">
        <table className="w-full text-left text-sm">
          <thead className="border-b border-slate-200/80 bg-slate-50/60 text-xs uppercase tracking-wide text-slate-500 dark:border-slate-800 dark:bg-slate-900">
            <tr><th className="px-5 py-3 font-semibold">Сотрудник</th><th className="px-5 py-3 font-semibold">Роль</th><th className="px-5 py-3 font-semibold">Состояние</th><th className="px-5 py-3" /></tr>
          </thead>
          <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
            {staff?.map((m) => (
              <tr key={m.id} className="transition hover:bg-slate-50/70 dark:hover:bg-slate-800/40">
                <td className="px-5 py-3.5">
                  <div className="flex items-center gap-3">
                    <Avatar name={m.display_name} />
                    <div><p className="font-semibold">{m.display_name}</p><p className="text-slate-500">@{m.username}</p></div>
                  </div>
                </td>
                <td className="px-5 py-3.5">{m.role === 'host' ? <Badge tone="brand">{roleName[m.role]}</Badge> : roleName[m.role]}</td>
                <td className="px-5 py-3.5">
                  <div className="flex flex-wrap gap-1.5">{stateBadge[m.state]}{m.reset_requested && <Badge tone="amber">Просит сбросить пароль</Badge>}{m.two_factor && <Badge tone="green">код ✓</Badge>}</div>
                </td>
                <td className="px-5 py-3.5 text-right">
                  {m.state === 'invited' && (
                    <Button variant="ghost" className="px-3 py-1.5 text-xs" onClick={() => setKeyFor(m)} title="Ключ для программы CRM со встроенным приглашением">
                      <KeyRound className="size-3.5" />Ключ
                    </Button>
                  )}
                  {m.role !== 'host' && (
                    <span className="inline-flex align-middle">
                      <Menu right button={(toggle) => (
                        <Button variant={m.reset_requested ? 'primary' : 'ghost'} className="px-2.5 py-1.5 text-xs" onClick={toggle} title="Действия">
                          {m.reset_requested ? <><RotateCcw className="size-3.5" />Сбросить доступ</> : <MoreHorizontal className="size-4" />}
                        </Button>
                      )}>
                        {(close) => (
                          <div className="text-left">
                            <MenuItem icon={<RotateCcw className="size-4" />} onClick={() => { close(); setResetting(m); }}>Сбросить доступ</MenuItem>
                            {m.two_factor && (
                              <MenuItem icon={<Smartphone className="size-4" />} onClick={async () => {
                                close();
                                if (!confirm(`Сбросить код из приложения у «${m.display_name}»? Новый код он настроит в офисе.`)) return;
                                await api.twoFactorReset(m.id);
                                void load();
                              }}>Сбросить код из приложения</MenuItem>
                            )}
                            {m.state === 'disabled'
                              ? <MenuItem icon={<UserCheck className="size-4" />} onClick={async () => { close(); await api.enableUser(m.id); void load(); }}>Включить</MenuItem>
                              : <MenuItem icon={<UserX className="size-4" />} onClick={async () => {
                                  close();
                                  if (!confirm(`Отключить «${m.display_name}»? Он сразу выйдет на всех устройствах и не сможет войти. Задачи и данные останутся, включить можно в любой момент.`)) return;
                                  await api.disableUser(m.id);
                                  void load();
                                }}>Отключить</MenuItem>}
                            <MenuItem danger icon={<Trash2 className="size-4" />} onClick={() => { close(); setDeleting(m); }}>Удалить…</MenuItem>
                          </div>
                        )}
                      </Menu>
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

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

      {keyFor && <ConnectionKey userId={keyFor === 'all' ? null : keyFor.id} name={keyFor === 'all' ? undefined : keyFor.display_name} onClose={() => setKeyFor(null)} />}

      <Modal open={!!result} onClose={() => setResult(null)} title="Приглашение готово">
        {result && <InviteResult {...result} />}
      </Modal>
    </>
  );
}
