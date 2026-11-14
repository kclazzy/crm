/** Схема рекомендуемого резервного копирования: правило 3-2-1. */
export function BackupScheme() {
  const box = 'fill-white stroke-slate-200 dark:fill-slate-900 dark:stroke-slate-700';
  const title = 'fill-slate-900 text-[13px] font-semibold dark:fill-slate-100';
  const sub = 'fill-slate-500 text-[11px]';
  return (
    <figure className="card p-6">
      <figcaption className="mb-1 font-semibold">Рекомендуемая схема: правило 3-2-1</figcaption>
      <p className="mb-5 text-sm text-slate-500">3 копии данных, на 2 разных носителях, 1 из них — вне офиса. Так данные переживут и поломку диска, и кражу или пожар.</p>
      <svg viewBox="0 0 720 210" className="w-full" role="img" aria-label="Схема: база на сервере, почасовые копии на втором диске, еженедельная копия вне офиса">
        <defs>
          <marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M0,0 L10,5 L0,10 z" className="fill-brand-500" />
          </marker>
        </defs>
        {[
          { x: 10, n: '1', t: 'Рабочая база', a: 'Сервер, диск C:', b: 'Работает постоянно' },
          { x: 260, n: '2', t: 'Копии на 2-м диске', a: 'Второй диск, USB или NAS', b: 'Все за 48 ч + по одной в сутки за 30 дней' },
          { x: 510, n: '3', t: 'Копия вне офиса', a: 'Зашифрованный USB-диск', b: 'Раз в неделю — домой или в сейф' },
        ].map((s) => (
          <g key={s.n}>
            <rect x={s.x} y="40" width="200" height="120" rx="16" className={box} strokeWidth="1.5" />
            <circle cx={s.x + 28} cy="72" r="14" className="fill-brand-600" />
            <text x={s.x + 28} y="77" textAnchor="middle" className="fill-white text-[13px] font-bold">{s.n}</text>
            <text x={s.x + 50} y="77" className={title}>{s.t}</text>
            <text x={s.x + 20} y="110" className="fill-slate-700 text-[12px] dark:fill-slate-300">{s.a}</text>
            <text x={s.x + 20} y="132" className={sub}>{s.b.length > 30 ? s.b.slice(0, s.b.indexOf(' ', 24)) : s.b}</text>
            {s.b.length > 30 && <text x={s.x + 20} y="148" className={sub}>{s.b.slice(s.b.indexOf(' ', 24) + 1)}</text>}
          </g>
        ))}
        <line x1="214" y1="100" x2="254" y2="100" className="stroke-brand-500" strokeWidth="2" markerEnd="url(#arr)" />
        <text x="234" y="30" textAnchor="middle" className="fill-brand-600 text-[11px] font-semibold dark:fill-brand-100">каждый час</text>
        <text x="234" y="190" textAnchor="middle" className={sub}>автоматически</text>
        <line x1="464" y1="100" x2="504" y2="100" className="stroke-brand-500" strokeWidth="2" strokeDasharray="5 4" markerEnd="url(#arr)" />
        <text x="484" y="30" textAnchor="middle" className="fill-brand-600 text-[11px] font-semibold dark:fill-brand-100">раз в неделю</text>
        <text x="484" y="190" textAnchor="middle" className={sub}>вручную</text>
      </svg>
    </figure>
  );
}
