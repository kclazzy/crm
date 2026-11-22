import { useEffect, useState } from 'react';
import { Timer } from 'lucide-react';
import { api, type SessionLimits } from '../api';
import { Button, ErrorText } from './ui';

const OFFICE = [[1, '1 день'], [7, '7 дней'], [14, '14 дней'], [30, '30 дней'], [90, '90 дней']] as const;
const IDLE = [[1, '1 час'], [4, '4 часа'], [12, '12 часов'], [24, 'сутки'], [72, '3 суток'], [168, 'неделю'], [720, '30 дней']] as const;
const CODE = [[0, 'только при входе'], [1, 'каждый день'], [7, 'раз в неделю'], [14, 'раз в 2 недели'], [30, 'раз в месяц']] as const;
const RECOMMENDED: SessionLimits = { office_days: 30, remote_idle_hours: 12, remote_code_days: 7 };

/** Сколько живёт вход: в офисе и из интернета (выбирает владелец). */
export function SessionLimitsCard() {
  const [l, setL] = useState<SessionLimits | null>(null);
  const [saved, setSaved] = useState<SessionLimits | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { api.sessionLimits().then((x) => { setL(x); setSaved(x); }).catch((e) => setError(e.message)); }, []);
  if (!l) return <ErrorText>{error}</ErrorText>;
  const dirty = JSON.stringify(l) !== JSON.stringify(saved);
  const select = 'input w-auto py-2';
  const row = (title: string, hint: string, list: readonly (readonly [number, string])[], key: keyof SessionLimits) => (
    <div className="flex flex-wrap items-center justify-between gap-3 py-3">
      <div className="min-w-0 flex-1"><p className="font-medium">{title}</p><p className="text-xs text-slate-500">{hint}</p></div>
      <select className={select} value={l[key]} onChange={(e) => setL({ ...l, [key]: Number(e.target.value) })}>
        {list.map(([v, t]) => <option key={v} value={v}>{t}</option>)}
      </select>
    </div>
  );
  return (
    <section className="card p-6">
      <h2 className="flex items-center gap-2 font-semibold"><Timer className="size-5 text-brand-600" />Сколько действует вход</h2>
      <p className="mt-1 text-sm text-slate-500">
        Из дома риск выше: ноутбук могут украсть или оставить открытым. Поэтому для интернета сроки короче. Срок считается от последнего действия — пока человек работает, его не выкинет.
      </p>
      <div className="mt-3 divide-y divide-slate-100 dark:divide-slate-800">
        {row('В офисе', 'Без действий столько — и нужно войти заново.', OFFICE, 'office_days')}
        {row('Из интернета: без действий', 'Отошёл от ноутбука дома — через столько времени вход закроется.', IDLE, 'remote_idle_hours')}
        {row('Из интернета: код из приложения', 'Как часто заново спрашивать 6 цифр с телефона (работа не прерывается).', CODE, 'remote_code_days')}
      </div>
      <ErrorText>{error}</ErrorText>
      <div className="mt-4 flex flex-wrap gap-2">
        <Button disabled={!dirty} busy={busy} onClick={async () => {
          setBusy(true); setError('');
          try { const x = await api.setSessionLimits(l); setL(x); setSaved(x); } catch (e) { setError((e as Error).message); } finally { setBusy(false); }
        }}>Сохранить</Button>
        <Button variant="ghost" onClick={() => setL(RECOMMENDED)}>Рекомендуемые: 30 дней · 12 часов · раз в неделю</Button>
      </div>
    </section>
  );
}
