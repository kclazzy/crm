import { useState } from 'react';
import { HelpCircle } from 'lucide-react';
import { GLOSSARY } from '../pages/Help';

/** «?» рядом с непонятным словом: наведите или нажмите — объяснение в одну-две строки. */
export function Hint({ word, text }: { word?: string; text?: string }) {
  const [open, setOpen] = useState(false);
  const body = text ?? GLOSSARY.find(([w]) => w === word)?.[1] ?? '';
  return (
    <span className="relative inline-flex align-middle" onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
      <button type="button" className="rounded-full text-slate-400 hover:text-brand-600" aria-label={word ? `Что такое «${word}»` : 'Подсказка'}
        onClick={(e) => { e.preventDefault(); e.stopPropagation(); setOpen((o) => !o); }}>
        <HelpCircle className="size-4" />
      </button>
      {open && (
        <span role="tooltip" className="absolute left-1/2 top-6 z-50 w-64 -translate-x-1/2 rounded-xl bg-slate-900 px-3 py-2 text-left text-xs font-normal leading-relaxed text-white shadow-lg dark:bg-slate-100 dark:text-slate-900">
          {word && <b className="block">{word}</b>}{body}
        </span>
      )}
    </span>
  );
}

/** Раздел справки для текущей страницы — кнопка «?» в шапке. */
export function helpFor(pathname: string): string {
  const admin: [string, string][] = [
    ['/admin/staff', 'staff'], ['/admin/groups', 'staff'], ['/admin/sessions', 'staff'],
    ['/admin/remote', 'remote'], ['/admin/protection', 'security'], ['/admin/journal', 'security'],
    ['/admin/storage', 'backups'], ['/admin', 'start'],
  ];
  const hit = admin.find(([p]) => pathname === p);
  if (hit) return `/admin/help#${hit[1]}`;
  if (pathname.startsWith('/admin')) return '/admin/help';
  if (pathname.startsWith('/clients')) return '/help#tasks';
  if (pathname.startsWith('/security')) return '/help#code';
  if (pathname.startsWith('/messenger')) return '/help#tasks';
  return '/help#tasks';
}
