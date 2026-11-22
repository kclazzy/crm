import { type ReactNode, useCallback, useRef, useState } from 'react';
import { useDismiss } from './useDismiss';

/** Выпадающее меню «⋯»: закрывается щелчком мимо и Esc. */
export function Menu({ button, children, right = false }: { button: (toggle: () => void) => ReactNode; children: (close: () => void) => ReactNode; right?: boolean }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useDismiss(open, useCallback(() => setOpen(false), []), box);
  return (
    <div className="relative" ref={box}>
      {button(() => setOpen((o) => !o))}
      {open && <div className={`card absolute z-30 mt-1 min-w-56 overflow-hidden py-1 shadow-xl ${right ? 'right-0' : ''}`}>{children(() => setOpen(false))}</div>}
    </div>
  );
}

export function MenuItem({ onClick, icon, children, danger = false }: { onClick: () => void; icon?: ReactNode; children: ReactNode; danger?: boolean }) {
  return (
    <button onClick={onClick} className={`flex w-full items-center justify-between gap-6 px-4 py-2.5 text-left text-[15px] hover:bg-slate-50 dark:hover:bg-slate-800 ${danger ? 'text-red-600' : ''}`}>
      <span>{children}</span><span className="text-slate-400">{icon}</span>
    </button>
  );
}

