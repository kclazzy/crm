import { useEffect, useRef, useState } from 'react';
import { Ago } from './ui';
import { useNavigate } from 'react-router';
import { Bell as BellIcon, BellRing, Moon, X } from 'lucide-react';
import { noteTarget, reminderText, useLive } from '../live';
import { work } from '../work';
import { Swipe } from './Swipe';

/** Колокольчик: напоминания о сроках и тревоги владельцу. */
export function Bell() {
  const { bell, refreshBell } = useLive();
  const [open, setOpen] = useState(false);
  const [always, setAlways] = useState<boolean | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  // Старые уведомления, подгруженные кнопкой «Показать ещё».
  const [older, setOlder] = useState<typeof bell.items>([]);
  const [olderMore, setOlderMore] = useState<boolean | null>(null);
  const shown = [...bell.items, ...older.filter((o) => !bell.items.some((n) => n.id === o.id))];
  const hasMore = olderMore ?? !!bell.more;
  const loadOlder = async () => {
    const last = shown[shown.length - 1];
    if (!last) return;
    const r = await work.notifications(last.created_at).catch(() => null);
    if (r) { setOlder((o) => [...o, ...r.items]); setOlderMore(r.more); }
  };
  const [perm, setPerm] = useState(typeof Notification === 'undefined' ? 'denied' : Notification.permission);
  const box = useRef<HTMLDivElement>(null);
  const navigate = useNavigate();

  useEffect(() => {
    if (!open) return;
    work.prefs().then((p) => setAlways(p.notify_always)).catch(() => {});
    const close = (e: MouseEvent) => { if (box.current && !box.current.contains(e.target as Node)) setOpen(false); };
    window.addEventListener('mousedown', close);
    return () => window.removeEventListener('mousedown', close);
  }, [open]);

  return (
    <div className="relative" ref={box}>
      <button onClick={() => setOpen((o) => !o)} className="relative rounded-xl p-2 text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800" title={bell.quiet ? 'Уведомления · нерабочее время, без звука' : 'Уведомления'}>
        <BellIcon className="size-5" />
        {bell.quiet && <Moon className="absolute bottom-1 right-1 size-3 rounded-full bg-slate-50 text-slate-500 dark:bg-slate-950" />}
        {bell.unread > 0 && <span className="absolute -right-0.5 -top-0.5 grid min-w-4 place-items-center rounded-full bg-red-600 px-1 text-[10px] font-bold text-white">{bell.unread}</span>}
      </button>
      {open && (
        <div className="card absolute right-0 z-40 mt-2 w-[min(92vw,360px)] overflow-hidden">
          <div className="flex items-center justify-between border-b border-slate-200 px-4 py-2.5 dark:border-slate-800">
            <p className="font-semibold">Уведомления</p>
            <span className="flex gap-3">
              {bell.unread > 0 && <button className="text-xs font-medium text-brand-600" onClick={async () => { await work.readNotifications(); refreshBell(); }}>Прочитать все</button>}
              {bell.items.length > 0 && (
                confirmClear
                  ? <span className="flex items-center gap-2 text-xs">Удалить все?
                      <button className="font-semibold text-red-600" onClick={async () => { setConfirmClear(false); await work.deleteNotifications(); setOlder([]); setOlderMore(false); refreshBell(); }}>Да</button>
                      <button className="text-slate-500" onClick={() => setConfirmClear(false)}>Нет</button>
                    </span>
                  : <button className="text-xs font-medium text-slate-500 hover:text-red-600" onClick={() => setConfirmClear(true)}>Удалить все</button>
              )}
            </span>
          </div>
          {bell.quiet && <p className="bg-slate-50 px-4 py-2 text-xs text-slate-500 dark:bg-slate-800/50"><Moon className="inline size-3.5" /> Сейчас нерабочее время — без звука и всплывающих уведомлений.</p>}
          <div className="max-h-80 overflow-y-auto">
            {shown.length === 0 && <p className="p-6 text-center text-sm text-slate-500">Уведомлений нет.</p>}
            {shown.map((n) => {
              const t = reminderText(n);
              return (
                <Swipe key={n.id} label="Удалить" tone="red" onAction={async () => { await work.deleteNotification(n.id).catch(() => {}); setOlder((o) => o.filter((x) => x.id !== n.id)); refreshBell(); }}>
                <div className={`group flex items-start border-b border-slate-100 hover:bg-slate-50 dark:border-slate-800 dark:hover:bg-slate-800/50 ${n.read_at ? 'opacity-60' : ''}`}>
                  <button onClick={() => { setOpen(false); if (n.task_id) navigate(`/tasks/${n.task_id}`); else navigate(noteTarget(n)); }}
                    className="min-w-0 flex-1 px-4 py-2.5 text-left">
                    <p className="text-sm font-medium">{t.title}</p>
                    <p className="truncate text-xs text-slate-500">{t.body}</p>
                    <p className="text-[11px] text-slate-400"><Ago ms={n.created_at} /></p>
                  </button>
                  <button onClick={async () => { await work.deleteNotification(n.id).catch(() => {}); setOlder((o) => o.filter((x) => x.id !== n.id)); refreshBell(); }}
                    className="m-1.5 rounded-lg p-1.5 text-slate-400 opacity-60 hover:bg-slate-200 hover:text-red-600 group-hover:opacity-100 dark:hover:bg-slate-700"
                    title="Удалить уведомление" aria-label="Удалить уведомление">
                    <X className="size-4" />
                  </button>
                </div>
                </Swipe>
              );
            })}
            {hasMore && <button className="w-full py-2.5 text-center text-xs font-semibold text-brand-600 hover:bg-slate-50 dark:hover:bg-slate-800/50" onClick={loadOlder}>Показать ещё</button>}
          </div>
          <div className="flex flex-col gap-2 border-t border-slate-200 p-3 text-xs dark:border-slate-800">
            {perm !== 'granted' && perm !== 'denied' && (
              <button className="inline-flex items-center gap-1.5 font-medium text-brand-600" onClick={async () => setPerm(await Notification.requestPermission())}>
                <BellRing className="size-3.5" /> Включить уведомления Windows
              </button>
            )}
            {always !== null && (
              <label className="flex items-center gap-2 text-slate-600 dark:text-slate-300">
                <input type="checkbox" checked={always} onChange={async (e) => { const v = e.target.checked; setAlways(v); await work.setPrefs(v); refreshBell(); }} />
                Уведомлять и в нерабочее время
              </label>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
