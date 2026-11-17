import { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { Bell as BellIcon, BellRing, Moon } from 'lucide-react';
import { timeAgo } from '../api';
import { noteTarget, reminderText, useLive } from '../live';
import { work } from '../work';

/** Колокольчик: напоминания о сроках и тревоги владельцу. */
export function Bell() {
  const { bell, refreshBell } = useLive();
  const [open, setOpen] = useState(false);
  const [always, setAlways] = useState<boolean | null>(null);
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
            {bell.unread > 0 && <button className="text-xs font-medium text-brand-600" onClick={async () => { await work.readNotifications(); refreshBell(); }}>Прочитать все</button>}
          </div>
          {bell.quiet && <p className="bg-slate-50 px-4 py-2 text-xs text-slate-500 dark:bg-slate-800/50"><Moon className="inline size-3.5" /> Сейчас нерабочее время — без звука и всплывающих уведомлений.</p>}
          <div className="max-h-80 overflow-y-auto">
            {bell.items.length === 0 && <p className="p-6 text-center text-sm text-slate-500">Уведомлений нет.</p>}
            {bell.items.map((n) => {
              const t = reminderText(n);
              return (
                <button key={n.id} onClick={() => { setOpen(false); if (n.task_id) navigate(`/tasks/${n.task_id}`); else navigate(noteTarget(n)); }}
                  className={`block w-full border-b border-slate-100 px-4 py-2.5 text-left hover:bg-slate-50 dark:border-slate-800 dark:hover:bg-slate-800/50 ${n.read_at ? 'opacity-60' : ''}`}>
                  <p className="text-sm font-medium">{t.title}</p>
                  <p className="truncate text-xs text-slate-500">{t.body}</p>
                  <p className="text-[11px] text-slate-400">{timeAgo(n.created_at)}</p>
                </button>
              );
            })}
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
