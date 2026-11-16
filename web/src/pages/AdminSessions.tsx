import { useCallback, useEffect, useState } from 'react';
import { Laptop, LogOut } from 'lucide-react';
import { api, type Session, timeAgo } from '../api';
import { Avatar, Button, ErrorText, Modal, PageHeader } from '../components/ui';

export function AdminSessions() {
  const [list, setList] = useState<Session[] | null>(null);
  const [error, setError] = useState('');
  const [target, setTarget] = useState<Session | null>(null);
  const [busy, setBusy] = useState(false);
  const load = useCallback(() => api.sessions().then(setList).catch((e) => setError(e.message)), []);
  useEffect(() => void load(), [load]);

  async function revoke(s: Session) {
    setBusy(true);
    try {
      await api.revokeSession(s.id);
      setTarget(null);
      load();
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <PageHeader title="Активные входы" subtitle="Где сейчас открыта CRM. Незнакомое устройство — отключите его." />
      {error && <div className="mb-4"><ErrorText>{error}</ErrorText></div>}
      <div className="grid gap-3">
        {list?.map((s) => (
          <div key={s.id} className="card flex items-center gap-4 p-4">
            <Avatar name={s.display_name} />
            <div className="min-w-0 flex-1">
              <p className="font-semibold">{s.display_name} <span className="font-normal text-slate-500">@{s.username}</span></p>
              <p className="mt-0.5 flex flex-wrap items-center gap-x-3 text-sm text-slate-500">
                <span className="inline-flex items-center gap-1.5"><Laptop className="size-3.5" />{s.device_name}</span>
                {s.ip.startsWith('2001:db8:') ? <span className="text-xs" title={`Условный адрес устройства: ${s.ip}`}>по ключу сервера, вне офиса</span> : <span className="font-mono text-xs">{s.ip}</span>}
                <span>активность {timeAgo(s.last_seen_at)}</span>
              </p>
            </div>
            <Button variant="secondary" className="px-3 py-2 text-xs" onClick={() => setTarget(s)}>
              <LogOut className="size-3.5" />Отключить
            </Button>
          </div>
        ))}
        {list?.length === 0 && <p className="text-slate-500">Активных входов нет.</p>}
      </div>
      <Modal open={!!target} onClose={() => setTarget(null)} title="Отключить устройство?">
        <p className="text-sm text-slate-600 dark:text-slate-300">
          {target?.display_name} будет отключён на устройстве «{target?.device_name}» сразу. Чтобы вернуться, нужно снова ввести пароль.
        </p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setTarget(null)}>Отмена</Button>
          <Button variant="danger" busy={busy} onClick={() => target && revoke(target)}>Отключить</Button>
        </div>
      </Modal>
    </>
  );
}
