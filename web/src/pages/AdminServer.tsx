import { useEffect, useState } from 'react';
import { Check, Copy, Download, Globe, ShieldCheck } from 'lucide-react';
import { api, type Health, type ServerInfo } from '../api';
import { Badge, PageHeader } from '../components/ui';

export function AdminServer() {
  const [h, setH] = useState<Health | null>(null);
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [copied, setCopied] = useState('');
  useEffect(() => {
    api.health().then(setH).catch(() => setH(null));
    api.server().then(setInfo).catch(() => setInfo(null));
  }, []);
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
          <h3 className="mt-6 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-slate-500"><Globe className="size-4" />Адрес для сотрудников</h3>
          <p className="mt-2 text-sm text-slate-500">Первый — по имени компьютера: он не меняется, даже если сменится IP.</p>
          <ul className="mt-3 flex flex-col gap-2">
            {info?.addresses.map((a) => (
              <li key={a} className="flex items-center justify-between gap-2 rounded-xl border border-slate-200 bg-slate-50 py-1.5 pl-3 pr-1.5 font-mono text-sm dark:border-slate-700 dark:bg-slate-950">
                <span className="truncate">{a}</span>
                <button className="rounded-lg p-1.5 text-slate-400 hover:bg-white hover:text-brand-600 dark:hover:bg-slate-900" title="Копировать"
                  onClick={async () => { await navigator.clipboard.writeText(a); setCopied(a); }}>
                  {copied === a ? <Check className="size-4" /> : <Copy className="size-4" />}
                </button>
              </li>
            ))}
          </ul>
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
