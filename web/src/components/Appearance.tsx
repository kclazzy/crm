import { useState } from 'react';
import { Monitor, Moon, Palette, Sun } from 'lucide-react';
import { Modal } from './ui';

export type Theme = 'system' | 'light' | 'dark';
export const SCALES = [{ v: 0.9, label: 'Мельче' }, { v: 1, label: 'Обычный' }, { v: 1.125, label: 'Крупнее' }, { v: 1.25, label: 'Крупный' }];
const THEME_KEY = 'crm.theme';
const SCALE_KEY = 'crm.ui-scale';

function read<T>(key: string, fallback: T, ok: (v: string) => boolean): T {
  try { const v = localStorage.getItem(key); return v !== null && ok(v) ? (v as unknown as T) : fallback; } catch { return fallback; }
}
export const savedTheme = () => read<Theme>(THEME_KEY, 'system', (v) => ['system', 'light', 'dark'].includes(v));
export const savedScale = () => Number(read<string>(SCALE_KEY, '1', (v) => SCALES.some((s) => String(s.v) === v)));

const system = () => typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: dark)').matches;

/** Применить тему и размер текста к странице. Вызывается до первой отрисовки. */
export function applyAppearance(theme = savedTheme(), scale = savedScale()) {
  const dark = theme === 'dark' || (theme === 'system' && system());
  document.documentElement.classList.toggle('dark', dark);
  document.documentElement.style.setProperty('--ui-scale', String(scale));
}

if (typeof matchMedia !== 'undefined') {
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyAppearance());
}

/** «Вид»: тема и размер текста во всей CRM — для этого компьютера. */
export function AppearanceButton({ className = '' }: { className?: string }) {
  const [open, setOpen] = useState(false);
  const [theme, setTheme] = useState(savedTheme);
  const [scale, setScale] = useState(savedScale);
  const save = (t: Theme, s: number) => {
    setTheme(t);
    setScale(s);
    try { localStorage.setItem(THEME_KEY, t); localStorage.setItem(SCALE_KEY, String(s)); } catch { /* до закрытия окна */ }
    applyAppearance(t, s);
  };
  const seg = (on: boolean) => `flex flex-1 items-center justify-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium ${on ? 'bg-brand-600 text-white' : 'bg-slate-100 text-slate-600 hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300'}`;
  return (
    <>
      <button onClick={() => setOpen(true)} className={className}><Palette className="size-[18px]" /><span className="flex-1 text-left">Вид</span></button>
      <Modal open={open} onClose={() => setOpen(false)} title="Вид">
        <p className="mb-2 text-sm font-medium">Тема</p>
        <div className="flex gap-1.5">
          <button className={seg(theme === 'system')} onClick={() => save('system', scale)}><Monitor className="size-4" />Как в системе</button>
          <button className={seg(theme === 'light')} onClick={() => save('light', scale)}><Sun className="size-4" />Светлая</button>
          <button className={seg(theme === 'dark')} onClick={() => save('dark', scale)}><Moon className="size-4" />Тёмная</button>
        </div>
        <p className="mb-2 mt-5 text-sm font-medium">Размер текста во всей CRM</p>
        <div className="flex gap-1.5">
          {SCALES.map((s) => (
            <button key={s.v} className={seg(scale === s.v)} onClick={() => save(theme, s.v)} style={{ fontSize: `${s.v * 0.875}rem` }}>{s.label}</button>
          ))}
        </div>
        <p className="mt-4 text-xs text-slate-500">Запоминается на этом компьютере. Размер текста в отдельной задаче можно менять ещё и кнопками «А» в её карточке.</p>
      </Modal>
    </>
  );
}
