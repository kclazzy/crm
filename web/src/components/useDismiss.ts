import { type RefObject, useEffect } from 'react';

/** Всплывающий список закрывается щелчком мимо и клавишей Esc — везде одинаково. */
export function useDismiss(open: boolean, close: () => void, box: RefObject<HTMLElement | null>) {
  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => { if (box.current && !box.current.contains(e.target as Node)) close(); };
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') close(); };
    window.addEventListener('mousedown', outside);
    window.addEventListener('keydown', esc);
    return () => { window.removeEventListener('mousedown', outside); window.removeEventListener('keydown', esc); };
  }, [open, close, box]);
}
