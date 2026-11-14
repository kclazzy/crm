import { useEffect, useState } from 'react';
import { Download, ShieldCheck } from 'lucide-react';
import { api, type Health } from '../api';
import { Badge, PageHeader } from '../components/ui';

export function AdminServer() {
  const [h, setH] = useState<Health | null>(null);
  useEffect(() => void api.health().then(setH).catch(() => setH(null)), []);
  return (
    <>
      <PageHeader title="Сервер" subtitle="Состояние и сертификат безопасности." />
      <div className="grid gap-5 md:grid-cols-2">
        <section className="card p-6">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-slate-500">Состояние</h3>
          <div className="mt-4 flex items-center gap-3">
            {h ? <Badge tone="green" dot>Работает</Badge> : <Badge tone="slate" dot>Нет ответа</Badge>}
            <span className="text-sm text-slate-500">версия {h?.version ?? '—'}</span>
          </div>
          <p className="mt-6 text-sm text-slate-500">Бэкапы и заполненность диска появятся здесь в следующих обновлениях. Пока делайте копию вручную — см. инструкцию администратора.</p>
        </section>
        <section className="card p-6">
          <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-slate-500"><ShieldCheck className="size-4" />Сертификат</h3>
          <p className="mt-4 text-sm text-slate-600 dark:text-slate-300">Сотрудник сверяет этот отпечаток при установке сертификата на свой компьютер.</p>
          <code className="mt-3 block rounded-xl border border-slate-200 bg-slate-50 p-3 font-mono text-xs leading-relaxed break-all select-all dark:border-slate-700 dark:bg-slate-950">
            {h?.ca_fingerprint ?? '—'}
          </code>
          <a href="/api/tls/ca" className="mt-4 inline-flex items-center gap-2 text-sm font-semibold text-brand-600 hover:underline">
            <Download className="size-4" />Скачать сертификат (crm-ca.crt)
          </a>
        </section>
      </div>
    </>
  );
}
