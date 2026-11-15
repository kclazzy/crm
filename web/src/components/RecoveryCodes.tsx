import { useEffect, useState } from 'react';
import { Check, Copy, KeyRound, Printer } from 'lucide-react';
import { api } from '../api';
import { Button, Modal } from './ui';

/** Коды сброса пароля владельца: сколько осталось, выпуск новых, печать. */
export function RecoveryCodes({ compact = false }: { compact?: boolean }) {
  const [unused, setUnused] = useState<number | null>(null);
  const [codes, setCodes] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  useEffect(() => void api.recoveryStatus().then((s) => setUnused(s.unused)).catch(() => {}), [codes]);

  async function issue() {
    setBusy(true);
    try {
      setCodes((await api.newRecoveryCodes()).codes);
    } finally {
      setBusy(false);
    }
  }

  // Баннер в списке сотрудников прячется, когда коды есть, но окно с только что
  // выданными кодами должно остаться — поэтому оно вне баннера.
  const hideBanner = compact && unused !== 0;
  const low = unused !== null && unused < 3;
  return (
    <>
    {!hideBanner && <section className={compact ? 'mb-6 flex flex-wrap items-center gap-4 rounded-2xl border border-amber-300 bg-amber-50 p-4 text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-200' : 'card p-6'}>
      <div className="flex-1">
        <h3 className={`flex items-center gap-2 ${compact ? 'font-semibold' : 'text-xs font-semibold uppercase tracking-wider text-slate-500'}`}><KeyRound className="size-4" />Коды сброса пароля</h3>
        <p className={`mt-1 text-sm ${compact ? '' : 'text-slate-600 dark:text-slate-300'}`}>
          {unused === 0
            ? 'Создайте коды сброса пароля — без них забытый пароль владельца можно сбросить только на самом сервере.'
            : `Если забудете пароль, войдёте с одним из кодов. Осталось неиспользованных: ${unused ?? '…'}.`}
          {low && unused !== 0 && ' Кодов мало — создайте новые.'}
        </p>
      </div>
      <Button variant={compact ? 'primary' : 'secondary'} busy={busy} onClick={issue} className={compact ? '' : 'mt-4'}>{unused ? 'Создать новые коды' : 'Создать коды'}</Button>
    </section>}

      <Modal open={!!codes} onClose={() => setCodes(null)} title="Сохраните коды сброса пароля">
        <p className="text-sm text-slate-600 dark:text-slate-300">Каждый код срабатывает <b>один раз</b>. Распечатайте или запишите и храните отдельно от компьютера. Прежние коды больше не действуют. Это окно покажется только сейчас.</p>
        <ol className="my-4 grid grid-cols-2 gap-2 font-mono text-sm">
          {codes?.map((c, i) => <li key={c} className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 dark:border-slate-700 dark:bg-slate-950"><span className="mr-2 text-slate-400">{i + 1}.</span>{c}</li>)}
        </ol>
        <div className="flex gap-2">
          <Button variant="secondary" className="flex-1" onClick={() => window.print()}><Printer className="size-4" />Печать</Button>
          <Button className="flex-1" onClick={async () => { await navigator.clipboard.writeText(codes?.join('\n') ?? ''); setCopied(true); }}>
            {copied ? <Check className="size-4" /> : <Copy className="size-4" />}{copied ? 'Скопировано' : 'Копировать все'}
          </Button>
        </div>
      </Modal>
    </>
  );
}
