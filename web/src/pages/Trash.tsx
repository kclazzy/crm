import { TrashAutoClean } from '../components/TrashAutoClean';
import { useCallback, useEffect, useState } from 'react';
import { RotateCcw, Trash2 } from 'lucide-react';
import { useAuth } from '../auth';
import { type Client, work } from '../work';
import { Button, ErrorText, Modal, PageHeader, Ago } from '../components/ui';

export function Trash() {
  const { me } = useAuth();
  const [clients, setClients] = useState<Client[]>([]);
  const [tasks, setTasks] = useState<Awaited<ReturnType<typeof work.trashedTasks>>>([]);
  const [error, setError] = useState('');
  const [purging, setPurging] = useState<Client | null>(null);

  const load = useCallback(() => {
    work.clients({ trash: true }).then(setClients).catch((e) => setError(e.message));
    work.trashedTasks().then(setTasks).catch(() => {});
  }, []);
  useEffect(load, [load]);
  const act = (f: () => Promise<unknown>) => f().then(load).catch((e) => setError(e.message));

  return (
    <>
      <PageHeader title="Корзина" subtitle="Удалённое можно вернуть. Окончательно удаляет только владелец." />
      <ErrorText>{error}</ErrorText>
      {me?.role === 'host' && <TrashAutoClean />}
      <div className="grid gap-5 md:grid-cols-2">
        <section className="card p-5">
          <h2 className="mb-3 font-semibold">Клиенты</h2>
          {clients.length === 0 && <p className="text-sm text-slate-500">Пусто. Удалённые клиенты лежат здесь, пока их не вернут или владелец не удалит навсегда.</p>}
          <ul className="divide-y divide-slate-100 dark:divide-slate-800">
            {clients.map((c) => (
              <li key={c.id} className="flex items-center gap-2 py-2.5 text-sm">
                <span className="min-w-0 flex-1"><b>{c.name}</b><br /><span className="text-xs text-slate-500">удалён {c.deleted_at && <Ago ms={c.deleted_at} />}</span></span>
                <Button variant="secondary" className="px-3 py-1.5 text-xs" onClick={() => act(() => work.restoreClient(c.id))}><RotateCcw className="size-3.5" />Вернуть</Button>
                {me?.role === 'host' && <Button variant="ghost" className="px-2 py-1.5 text-red-600" title="Удалить навсегда" onClick={() => setPurging(c)}><Trash2 className="size-4" /></Button>}
              </li>
            ))}
          </ul>
        </section>
        <section className="card p-5">
          <h2 className="mb-3 font-semibold">Задачи</h2>
          {tasks.length === 0 && <p className="text-sm text-slate-500">Пусто. Задачи, убранные «В корзину», можно вернуть отсюда вместе с подзадачами и чатом.</p>}
          <ul className="divide-y divide-slate-100 dark:divide-slate-800">
            {tasks.map((t) => (
              <li key={t.id} className="flex items-center gap-2 py-2.5 text-sm">
                <span className="min-w-0 flex-1"><b>{t.title}</b><br /><span className="text-xs text-slate-500">{t.client_name} · <Ago ms={t.deleted_at} /></span></span>
                <Button variant="secondary" className="px-3 py-1.5 text-xs" onClick={() => act(() => work.restoreTask(t.id))}><RotateCcw className="size-3.5" />Вернуть</Button>
              </li>
            ))}
          </ul>
        </section>
      </div>
      <Modal open={!!purging} onClose={() => setPurging(null)} title="Удалить навсегда?">
        <p className="text-sm text-slate-600 dark:text-slate-300">Клиент «{purging?.name}», все его задачи, чаты и файлы будут удалены без возможности вернуть (кроме как из резервной копии).</p>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setPurging(null)}>Отмена</Button>
          <Button variant="danger" onClick={() => { const c = purging; setPurging(null); if (c) act(() => work.purgeClient(c.id)); }}>Удалить навсегда</Button>
        </div>
      </Modal>
    </>
  );
}
