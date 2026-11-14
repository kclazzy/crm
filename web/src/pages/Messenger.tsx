import { useCallback, useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import { ArrowLeft, CheckCheck, ExternalLink, MessagesSquare } from 'lucide-react';
import { useLive, useLiveReload } from '../live';
import { type Thread, work } from '../work';
import { Button } from '../components/ui';
import { DeadlineText, PriorityBadge, UnreadDot } from '../components/work';
import { Chat } from '../components/Chat';

function when(ms: number) {
  const d = new Date(ms);
  return new Date().toDateString() === d.toDateString()
    ? d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'short' });
}

/** Все чаты задач в одном месте: «Клиент → Задача», свежие сверху. */
export function Messenger() {
  const [params, setParams] = useSearchParams();
  const open = params.get('task');
  const [onlyUnread, setOnlyUnread] = useState(false);
  const [threads, setThreads] = useState<Thread[] | null>(null);
  const { unread, refreshUnread } = useLive();

  const load = useCallback(() => work.threads(onlyUnread).then(setThreads).catch(() => {}), [onlyUnread]);
  useEffect(() => void load(), [load]);
  useLiveReload(load);
  const current = threads?.find((t) => t.id === open);

  return (
    <div className="flex h-[calc(100dvh-8.5rem)] gap-4 lg:h-[calc(100dvh-5rem)]">
      <aside className={`card flex w-full flex-col overflow-hidden md:w-80 md:shrink-0 ${open ? 'hidden md:flex' : 'flex'}`}>
        <div className="flex items-center gap-2 border-b border-slate-200 px-4 py-3 dark:border-slate-800">
          <h1 className="flex-1 font-semibold">Мессенджер</h1>
          {unread > 0 && (
            <button className="text-xs font-medium text-brand-600" title="Отметить все прочитанными" onClick={async () => { await work.readAll(); refreshUnread(); load(); }}>
              <CheckCheck className="inline size-4" /> Прочитать все
            </button>
          )}
        </div>
        <label className="flex items-center gap-2 border-b border-slate-100 px-4 py-2 text-sm text-slate-500 dark:border-slate-800">
          <input type="checkbox" checked={onlyUnread} onChange={(e) => setOnlyUnread(e.target.checked)} /> Только непрочитанные
        </label>
        <div className="flex-1 overflow-y-auto">
          {threads?.length === 0 && <p className="p-6 text-center text-sm text-slate-500">{onlyUnread ? 'Всё прочитано.' : 'Чатов пока нет.'}</p>}
          {threads?.map((t) => (
            <button key={t.id} onClick={() => setParams({ task: t.id })}
              className={`flex w-full flex-col gap-1 border-b border-slate-100 px-4 py-3 text-left transition hover:bg-slate-50 dark:border-slate-800 dark:hover:bg-slate-800/50 ${t.id === open ? 'bg-brand-50/70 dark:bg-brand-500/10' : ''}`}>
              <div className="flex items-center gap-2">
                <span className="truncate text-xs text-slate-500">{t.client_name}</span>
                <span className="ml-auto shrink-0 text-xs text-slate-400">{t.last_message && when(t.last_message.created_at)}</span>
              </div>
              <div className="flex items-center gap-2">
                {t.priority && <PriorityBadge p={t.priority} />}
                <span className={`min-w-0 flex-1 truncate ${t.unread ? 'font-semibold' : 'font-medium'}`}>{t.title}</span>
                <UnreadDot n={t.unread} />
              </div>
              {t.last_message && (
                <p className="truncate text-sm text-slate-500">
                  {t.last_message.kind === 'system' ? <i>{t.last_message.excerpt}</i> : <><b className="font-medium">{t.last_message.sender_name.split(' ')[0]}:</b> {t.last_message.excerpt}</>}
                </p>
              )}
            </button>
          ))}
        </div>
      </aside>
      <section className={`card min-w-0 flex-1 flex-col overflow-hidden ${open ? 'flex' : 'hidden md:flex'}`}>
        {open && current ? (
          <>
            <div className="flex items-center gap-3 border-b border-slate-200 px-4 py-3 dark:border-slate-800">
              <button className="md:hidden" onClick={() => setParams({})} aria-label="Назад"><ArrowLeft className="size-5" /></button>
              <div className="min-w-0 flex-1">
                <p className="truncate font-semibold">{current.title}</p>
                <p className="truncate text-xs text-slate-500">{current.client_name} · <DeadlineText t={current} /></p>
              </div>
              <Link to={`/tasks/${current.id}`}><Button variant="ghost" className="px-3 py-1.5"><ExternalLink className="size-4" />Задача</Button></Link>
            </div>
            <Chat taskId={current.id} canWrite={current.access === 'participant' || current.access === 'manage'} />
          </>
        ) : (
          <div className="m-auto flex flex-col items-center p-8 text-center text-slate-500">
            <MessagesSquare className="size-10 text-brand-500" />
            <p className="mt-3">Выберите чат слева.</p>
          </div>
        )}
      </section>
    </div>
  );
}
