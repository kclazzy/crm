import { useEffect, useState } from 'react';
import { Sparkles } from 'lucide-react';
import { api, type TrashPolicy } from '../api';
import { plural } from '../work';
import { ErrorText } from './ui';

/** Автоочистка корзины — только по решению владельца. */
export function TrashAutoClean() {
  const [p, setP] = useState<TrashPolicy | null>(null);
  const [error, setError] = useState('');
  const load = () => api.trashPolicy().then(setP).catch((e) => setError(e.message));
  useEffect(() => void load(), []);
  if (!p) return null;
  return (
    <section className={`card mb-5 flex flex-wrap items-center gap-3 p-4 ${p.suggest ? 'border-amber-300 bg-amber-50/70 dark:border-amber-500/40 dark:bg-amber-500/10' : ''}`}>
      <Sparkles className="size-5 shrink-0 text-brand-600" />
      <div className="min-w-0 flex-1 text-sm">
        <b>Автоочистка корзины</b>
        <span className="block text-slate-500">
          {p.days
            ? `Удалённое старше ${p.days} дней удаляется навсегда само, раз в сутки.`
            : p.old_items > 0
              ? `Выключена. В корзине ${p.old_items} ${plural(p.old_items, 'запись', 'записи', 'записей')} старше 90 дней — можно включить, чтобы база не разрасталась.`
              : 'Выключена: удалённое лежит здесь, пока его не вернут или не удалят вручную.'}
        </span>
        <ErrorText>{error}</ErrorText>
      </div>
      <select className="input w-auto py-2" value={p.days ?? 0} onChange={async (e) => {
        setError('');
        try { await api.setTrashPolicy(Number(e.target.value) || null); load(); } catch (x) { setError((x as Error).message); }
      }}>
        <option value={0}>Не удалять само</option>
        {p.choices.map((d) => <option key={d} value={d}>Через {d} дней</option>)}
      </select>
    </section>
  );
}
