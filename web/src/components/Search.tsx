import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useNavigate } from 'react-router';
import { Building2, CheckSquare, ListChecks, MessageCircle, Search as SearchIcon } from 'lucide-react';
import { type Hit, work } from '../work';

const kindMeta = {
  client: { label: 'Клиенты', icon: Building2 },
  task: { label: 'Задачи', icon: ListChecks },
  stage: { label: 'Этапы', icon: CheckSquare },
  message: { label: 'Сообщения', icon: MessageCircle },
} as const;

/** Ctrl+K: поиск по клиентам, задачам, этапам и сообщениям — только по доступному. */
export function SearchPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const [q, setQ] = useState('');
  const [archive, setArchive] = useState(false);
  const [hits, setHits] = useState<Hit[]>([]);
  const [active, setActive] = useState(0);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setQ('');
      setHits([]);
      setTimeout(() => input.current?.focus(), 0);
    }
  }, [open]);

  useEffect(() => {
    if (q.trim().length < 3) {
      setHits([]);
      return;
    }
    setBusy(true);
    const t = setTimeout(() => {
      work.search(q, archive).then((h) => { setHits(h); setActive(0); }).catch(() => setHits([])).finally(() => setBusy(false));
    }, 180);
    return () => clearTimeout(t);
  }, [q, archive]);

  function go(h: Hit) {
    onClose();
    if (h.kind === 'client') navigate(`/clients/${h.id}`);
    else if (h.task_id) navigate(`/tasks/${h.task_id}`);
  }

  if (!open) return null;
  const groups = (Object.keys(kindMeta) as Hit['kind'][]).map((k) => ({ k, items: hits.filter((h) => h.kind === k) })).filter((g) => g.items.length);
  const flat = groups.flatMap((g) => g.items);

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-slate-950/40 p-4 pt-[12vh] backdrop-blur-sm" onMouseDown={onClose}>
      <div className="card animate-rise w-full max-w-xl overflow-hidden" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Поиск">
        <div className="flex items-center gap-3 border-b border-slate-200 px-4 dark:border-slate-800">
          <SearchIcon className="size-5 text-slate-400" />
          <input ref={input} className="h-14 flex-1 bg-transparent text-[16px] outline-none" placeholder="Клиент, задача, этап, текст сообщения…" value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') onClose();
              if (e.key === 'ArrowDown') { e.preventDefault(); setActive((a) => Math.min(a + 1, flat.length - 1)); }
              if (e.key === 'ArrowUp') { e.preventDefault(); setActive((a) => Math.max(a - 1, 0)); }
              if (e.key === 'Enter' && flat[active]) go(flat[active]);
            }} />
          <label className="flex items-center gap-1.5 text-xs text-slate-500"><input type="checkbox" checked={archive} onChange={(e) => setArchive(e.target.checked)} />и архив</label>
        </div>
        <div className="max-h-[60vh] overflow-y-auto p-2">
          {q.trim().length < 3 && <p className="p-4 text-center text-sm text-slate-500">Введите хотя бы 3 символа. Стрелки — выбор, Enter — открыть.</p>}
          {q.trim().length >= 3 && !busy && flat.length === 0 && <p className="p-4 text-center text-sm text-slate-500">Ничего не найдено.</p>}
          {groups.map((g) => {
            const Icon = kindMeta[g.k].icon;
            return (
              <div key={g.k} className="mb-2">
                <p className="px-3 py-1 text-[11px] font-semibold uppercase tracking-wider text-slate-400">{kindMeta[g.k].label}</p>
                {g.items.map((h) => {
                  const i = flat.indexOf(h);
                  return (
                    <button key={`${h.kind}${h.id}`} onClick={() => go(h)} onMouseEnter={() => setActive(i)}
                      className={`flex w-full items-start gap-3 rounded-xl px-3 py-2 text-left ${i === active ? 'bg-brand-50 dark:bg-brand-500/15' : ''}`}>
                      <Icon className="mt-0.5 size-4 shrink-0 text-brand-500" />
                      <span className="min-w-0">
                        <span className="block truncate font-medium">{h.title || h.snippet}</span>
                        {h.snippet && h.snippet !== h.title && <span className="block truncate text-sm text-slate-500">{h.snippet.replace(/\*\*|`/g, '')}</span>}
                      </span>
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
    </div>,
    document.body,
  );
}
