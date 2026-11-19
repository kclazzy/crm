import { createContext, type ReactNode, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { Link } from 'react-router';
import { CheckCircle2, AlertCircle, X } from 'lucide-react';

export interface ToastOptions {
  /** «Отменить» в течение 5 секунд. */
  undo?: () => Promise<unknown> | unknown;
  /** Ссылка рядом с текстом: «Открыть». */
  link?: { to: string; text: string };
  error?: boolean;
}

interface Item extends ToastOptions { id: number; text: string }

const Ctx = createContext<(text: string, o?: ToastOptions) => void>(() => {});
const LIFE_MS = 5000;

/** Короткое сообщение внизу экрана после действия: «Сохранено», «В корзине · Отменить». */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Item[]>([]);
  const next = useRef(1);
  const drop = useCallback((id: number) => setItems((all) => all.filter((t) => t.id !== id)), []);
  const show = useCallback((text: string, o: ToastOptions = {}) => {
    const id = next.current++;
    setItems((all) => [...all.slice(-2), { id, text, ...o }]);
  }, []);
  return (
    <Ctx.Provider value={show}>
      {children}
      <div className="pointer-events-none fixed inset-x-0 bottom-20 z-[60] flex flex-col items-center gap-2 px-4 lg:bottom-6" aria-live="polite">
        {items.map((t) => <ToastView key={t.id} t={t} onDone={() => drop(t.id)} onError={(m) => show(m, { error: true })} />)}
      </div>
    </Ctx.Provider>
  );
}

function ToastView({ t, onDone, onError }: { t: Item; onDone: () => void; onError: (m: string) => void }) {
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const timer = setTimeout(onDone, t.undo ? LIFE_MS : 3500);
    return () => clearTimeout(timer);
  }, [t.undo, onDone]);
  return (
    <div role="status" className={`animate-rise pointer-events-auto flex max-w-md items-center gap-3 rounded-xl px-4 py-2.5 text-sm shadow-lg ${t.error ? 'bg-red-600 text-white' : 'bg-slate-900 text-white dark:bg-slate-100 dark:text-slate-900'}`}>
      {t.error ? <AlertCircle className="size-4 shrink-0" /> : <CheckCircle2 className="size-4 shrink-0 text-emerald-400 dark:text-emerald-600" />}
      <span className="min-w-0 flex-1">{t.text}</span>
      {t.link && <Link to={t.link.to} onClick={onDone} className="font-semibold underline-offset-2 hover:underline">{t.link.text}</Link>}
      {t.undo && (
        <button disabled={busy} className="font-semibold text-brand-200 hover:underline disabled:opacity-60 dark:text-brand-700" onClick={async () => {
          setBusy(true);
          try { await t.undo?.(); onDone(); } catch (e) { onDone(); onError(`Не удалось отменить: ${(e as Error).message}`); }
        }}>Отменить</button>
      )}
      <button onClick={onDone} className="rounded p-0.5 opacity-60 hover:opacity-100" aria-label="Закрыть"><X className="size-4" /></button>
    </div>
  );
}

export const useToast = () => useContext(Ctx);
