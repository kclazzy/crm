// Живые данные: WebSocket с сервера, счётчики непрочитанных, колокольчик, уведомления ОС.
import { createContext, type ReactNode, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { type Notification as Note, work } from './work';

type Listener = (e: { type: string; task_id?: string; what?: string }) => void;

interface Live {
  unread: number;
  unreadByTask: Record<string, number>;
  bell: { unread: number; items: Note[]; quiet: boolean };
  online: boolean;
  refreshUnread: () => void;
  refreshBell: () => void;
  subscribe: (l: Listener) => () => void;
}

const Ctx = createContext<Live>({
  unread: 0,
  unreadByTask: {},
  bell: { unread: 0, items: [], quiet: false },
  online: false,
  refreshUnread: () => {},
  refreshBell: () => {},
  subscribe: () => () => {},
});

function osNotify(title: string, body: string) {
  try {
    if ('Notification' in window && Notification.permission === 'granted') {
      const n = new Notification(title, { body, icon: '/logo.png', tag: title });
      n.onclick = () => window.focus();
    }
  } catch {
    // уведомления недоступны в этом окне — не страшно
  }
}

export function reminderText(n: Note): { title: string; body: string } {
  const p = n.payload as Record<string, string | number | null>;
  switch (n.kind) {
    case 'reminder': {
      const when = { h24: 'через сутки', h4: 'через 4 часа', h1: 'через час', overdue: 'просрочена' }[String(p.reminder)] ?? '';
      return { title: p.reminder === 'overdue' ? 'Задача просрочена' : `Дедлайн ${when}`, body: `${p.title} · ${p.client}` };
    }
    case 'disk_critical':
      return { title: 'Диск почти заполнен', body: `Том ${p.volume}: освободите место или перенесите данные` };
    case 'disk_warning':
      return { title: 'Диск заполняется', body: p.days_left ? `Том ${p.volume}: хватит примерно на ${p.days_left} дн.` : `Том ${p.volume}` };
    case 'backup_problem':
      return { title: 'Резервные копии не делаются', body: String(p.error ?? 'Проверьте папку копий в «Хранилище и копии»') };
    default:
      return { title: 'Уведомление CRM', body: '' };
  }
}

export function LiveProvider({ children }: { children: ReactNode }) {
  const [unread, setUnread] = useState({ total: 0, tasks: {} as Record<string, number> });
  const [bell, setBell] = useState<Live['bell']>({ unread: 0, items: [], quiet: false });
  const [online, setOnline] = useState(false);
  const listeners = useRef(new Set<Listener>());
  const seen = useRef<Set<string> | null>(null);
  const lastTotal = useRef(0);
  const quiet = useRef(false);

  const refreshUnread = useCallback(() => {
    work.unread().then((u) => {
      if (u.total > lastTotal.current && document.hidden && !quiet.current) {
        osNotify('Новые сообщения', `Непрочитанных: ${u.total}`);
      }
      lastTotal.current = u.total;
      setUnread(u);
    }).catch(() => {});
  }, []);

  const refreshBell = useCallback(() => {
    work.notifications().then((b) => {
      quiet.current = b.quiet;
      // Уведомления ОС — только о новых, только вне тихих часов.
      if (seen.current) {
        for (const n of b.items) {
          if (!seen.current.has(n.id) && !n.read_at && !b.quiet) {
            const t = reminderText(n);
            osNotify(t.title, t.body);
          }
        }
      }
      seen.current = new Set(b.items.map((n) => n.id));
      setBell(b);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    refreshUnread();
    refreshBell();
    let ws: WebSocket | null = null;
    let retry = 0;
    let timer: number | undefined;
    let closed = false;
    const connect = () => {
      const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
      ws = new WebSocket(`${proto}//${location.host}/api/ws`);
      ws.onopen = () => {
        retry = 0;
        setOnline(true);
        // После обрыва — всё могло измениться.
        refreshUnread();
        refreshBell();
      };
      ws.onmessage = (m) => {
        let e: { type: string; task_id?: string; what?: string };
        try {
          e = JSON.parse(String(m.data));
        } catch {
          return;
        }
        if (e.type === 'task' || e.type === 'unread' || e.type === 'resync') refreshUnread();
        if (e.type === 'notification' || e.type === 'resync') refreshBell();
        for (const l of listeners.current) l(e);
      };
      ws.onclose = () => {
        setOnline(false);
        if (closed) return;
        retry = Math.min(retry + 1, 6);
        timer = window.setTimeout(connect, 500 * 2 ** retry);
      };
    };
    connect();
    return () => {
      closed = true;
      clearTimeout(timer);
      ws?.close();
    };
  }, [refreshUnread, refreshBell]);

  useEffect(() => {
    document.title = unread.total > 0 ? `(${unread.total}) CRM` : 'CRM';
  }, [unread.total]);

  const subscribe = useCallback((l: Listener) => {
    listeners.current.add(l);
    return () => void listeners.current.delete(l);
  }, []);

  return (
    <Ctx.Provider value={{ unread: unread.total, unreadByTask: unread.tasks, bell, online, refreshUnread, refreshBell, subscribe }}>
      {children}
    </Ctx.Provider>
  );
}

export const useLive = () => useContext(Ctx);

/** Перезагрузить данные, когда сервер сообщил об изменении (`taskId` — только этой задачи). */
export function useLiveReload(reload: () => void, taskId?: string) {
  const { subscribe } = useLive();
  const ref = useRef(reload);
  ref.current = reload;
  useEffect(
    () =>
      subscribe((e) => {
        if (e.type === 'resync' || (e.type === 'task' && (!taskId || e.task_id === taskId))) ref.current();
      }),
    [subscribe, taskId],
  );
}
