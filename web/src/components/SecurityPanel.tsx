import { useEffect, useState } from 'react';
import { ShieldAlert, ShieldCheck } from 'lucide-react';
import { api, timeAgo } from '../api';

type Sec = Awaited<ReturnType<typeof api.security>>;

/** Защита от перегрузки и подбора паролей: что сервер заметил за сутки. */
export function SecurityPanel() {
  const [sec, setSec] = useState<Sec | null>(null);
  useEffect(() => void api.security().then(setSec).catch(() => {}), []);
  const quiet = sec && sec.blocked.length === 0 && sec.failed_logins.length === 0;
  return (
    <section className="card p-6">
      <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-slate-500">
        {quiet ? <ShieldCheck className="size-4" /> : <ShieldAlert className="size-4" />}Защита за последние сутки
      </h3>
      <p className="mt-2 text-sm text-slate-600 dark:text-slate-300">
        Сервер сам ограничивает слишком частые запросы и подбор паролей: 5 ошибок для логина или 20 с одного адреса за 15 минут — пауза.
      </p>
      {quiet && <p className="mt-4 text-sm font-medium text-emerald-700 dark:text-emerald-300">Всё спокойно: подозрительной активности не было.</p>}
      {!!sec?.failed_logins.length && (
        <div className="mt-4">
          <p className="text-sm font-semibold">Неудачные входы</p>
          <ul className="mt-2 divide-y divide-slate-100 text-sm dark:divide-slate-800">
            {sec.failed_logins.map((f) => (
              <li key={f.ip} className="flex justify-between py-1.5"><span className="font-mono">{f.ip}</span><span className="text-slate-500">{f.count} · {timeAgo(f.last_at)}</span></li>
            ))}
          </ul>
        </div>
      )}
      {!!sec?.blocked.length && (
        <div className="mt-4">
          <p className="text-sm font-semibold">Временно ограничены</p>
          <ul className="mt-2 divide-y divide-slate-100 text-sm dark:divide-slate-800">
            {sec.blocked.map((b) => (
              <li key={b.ip} className="flex justify-between py-1.5"><span className="font-mono">{b.ip}</span><span className="text-slate-500">{b.count} отказов · {timeAgo(Date.now() - b.seconds_ago * 1000)}</span></li>
            ))}
          </ul>
          <p className="mt-2 text-xs text-slate-500">Незнакомый адрес в списке — проверьте «Активные входы» и смените пароли, если нужно.</p>
        </div>
      )}
    </section>
  );
}
