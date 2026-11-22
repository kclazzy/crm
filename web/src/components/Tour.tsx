import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { Sparkles } from 'lucide-react';
import { Button } from './ui';

export interface TourStep { target: string; title: string; text: string }

const KEY = (user: string) => `crm.tour-done.${user}`;

/** Знакомство при первом входе: 4–5 подсказок поверх экрана, один раз на пользователя. */
export function Tour({ user, steps }: { user: string; steps: TourStep[] }) {
  const [i, setI] = useState<number | null>(() => {
    try { return localStorage.getItem(KEY(user)) ? null : 0; } catch { return null; }
  });
  const [box, setBox] = useState<DOMRect | null>(null);
  const step = i === null ? null : steps[i];

  useEffect(() => {
    if (!step) return;
    const place = () => {
      const el = [...document.querySelectorAll<HTMLElement>(`[data-tour="${step.target}"]`)].find((e) => e.offsetParent !== null);
      setBox(el ? el.getBoundingClientRect() : null);
    };
    place();
    // Esc — то же, что «Пропустить».
    const esc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      try { localStorage.setItem(KEY(user), '1'); } catch { /* без памяти — покажем ещё раз */ }
      setI(null);
    };
    window.addEventListener('resize', place);
    window.addEventListener('keydown', esc);
    return () => { window.removeEventListener('resize', place); window.removeEventListener('keydown', esc); };
  }, [step, user]);

  if (!step || i === null) return null;
  const finish = () => {
    try { localStorage.setItem(KEY(user), '1'); } catch { /* без памяти — покажем ещё раз */ }
    setI(null);
  };
  const last = i === steps.length - 1;
  // Карточка — рядом с подсвеченным элементом, иначе по центру внизу.
  const style = box
    ? { top: Math.min(box.bottom + 12, window.innerHeight - 220), left: Math.max(12, Math.min(box.left, window.innerWidth - 352)) }
    : { bottom: 96, left: '50%', transform: 'translateX(-50%)' };
  return createPortal(
    <div className="fixed inset-0 z-[70]" role="dialog" aria-label="Знакомство с CRM">
      <div className="absolute inset-0 bg-slate-950/40" onClick={finish} />
      {box && <div className="pointer-events-none absolute rounded-xl ring-4 ring-brand-400 transition-all" style={{ top: box.top - 4, left: box.left - 4, width: box.width + 8, height: box.height + 8 }} />}
      <div className="card absolute w-[min(340px,calc(100vw-24px))] p-4 shadow-2xl" style={style}>
        <p className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-brand-600"><Sparkles className="size-4" />Знакомство · {i + 1} из {steps.length}</p>
        <p className="mt-2 font-semibold">{step.title}</p>
        <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">{step.text}</p>
        <div className="mt-4 flex items-center gap-2">
          <button className="text-sm text-slate-500 hover:underline" onClick={finish}>Пропустить</button>
          <span className="flex-1" />
          {i > 0 && <Button variant="secondary" className="px-3 py-1.5 text-sm" onClick={() => setI(i - 1)}>Назад</Button>}
          <Button className="px-3 py-1.5 text-sm" onClick={() => (last ? finish() : setI(i + 1))} autoFocus>{last ? 'Понятно' : 'Дальше'}</Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** Показать знакомство ещё раз (из справки). */
export function restartTour(user: string) {
  try { localStorage.removeItem(KEY(user)); } catch { /* ничего */ }
  window.location.reload();
}
