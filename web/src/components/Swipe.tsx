import { type ReactNode, useRef, useState } from 'react';

/** Свайп влево на телефоне открывает действие под строкой («Завершить», «Удалить»).
 * На компьютере (мышь) ничего не меняется. */
export function Swipe({ children, label, tone = 'brand', onAction }: { children: ReactNode; label: ReactNode; tone?: 'brand' | 'red' | 'green'; onAction: () => void }) {
  const start = useRef<{ x: number; y: number } | null>(null);
  const [dx, setDx] = useState(0);
  const [locked, setLocked] = useState<'h' | 'v' | null>(null);
  const W = 104;
  const bg = { brand: 'bg-brand-600', red: 'bg-red-600', green: 'bg-emerald-600' }[tone];
  return (
    <div className="relative overflow-hidden">
      <button type="button" onClick={() => { setDx(0); onAction(); }} tabIndex={dx ? 0 : -1}
        className={`absolute inset-y-0 right-0 flex items-center justify-center px-3 text-sm font-semibold text-white ${bg}`} style={{ width: W }} aria-hidden={!dx}>
        {label}
      </button>
      <div
        className={`relative ${start.current ? '' : 'transition-transform duration-200'}`}
        style={{ transform: `translateX(${dx}px)` }}
        onTouchStart={(e) => { start.current = { x: e.touches[0].clientX, y: e.touches[0].clientY }; setLocked(null); }}
        onTouchMove={(e) => {
          if (!start.current) return;
          const mx = e.touches[0].clientX - start.current.x;
          const my = e.touches[0].clientY - start.current.y;
          const dir = locked ?? (Math.abs(mx) > 8 || Math.abs(my) > 8 ? (Math.abs(mx) > Math.abs(my) ? 'h' : 'v') : null);
          if (dir !== locked) setLocked(dir);
          if (dir === 'h') setDx(Math.max(-W, Math.min(0, mx + (dx < 0 ? -W : 0))));
        }}
        onTouchEnd={() => { start.current = null; setDx((d) => (d < -W / 2 ? -W : 0)); }}
        onClickCapture={(e) => { if (dx) { e.preventDefault(); e.stopPropagation(); setDx(0); } }}
      >
        {children}
      </div>
    </div>
  );
}
