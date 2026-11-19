import { useEffect, useState } from 'react';
import { Check, Copy, Download, Globe, ShieldCheck, Smartphone } from 'lucide-react';
import { work } from '../work';
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
      <PhoneAccess />
    </>
  );
}

const phoneTitle = {
  cert: { n: 1, title: 'Установить сертификат', hint: 'Один раз на каждый телефон' },
  office: { n: 2, title: 'CRM в офисе', hint: 'Телефон в Wi-Fi офиса' },
  internet: { n: 3, title: 'CRM из интернета', hint: 'Из дома и с мобильного интернета' },
} as const;

/** Вход с телефона: QR-коды и как поставить сертификат на Android и iPhone. */
function PhoneAccess() {
  const [links, setLinks] = useState<Awaited<ReturnType<typeof work.phone>>>([]);
  useEffect(() => void work.phone().then(setLinks).catch(() => {}), []);
  if (!links.length) return null;
  return (
    <section className="card mt-5 p-6">
      <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-slate-500"><Smartphone className="size-4" />Вход с телефона</h3>
      <p className="mt-2 text-sm text-slate-600 dark:text-slate-300">Сотрудник наводит камеру телефона на QR-код. Покажите коды с экрана или распечатайте эту страницу (Ctrl+P).</p>
      <div className="mt-4 grid gap-4 sm:grid-cols-3">
        {links.map((l) => (
          <div key={l.kind} className="flex flex-col items-center rounded-2xl border border-slate-200 p-4 text-center dark:border-slate-700">
            <p className="font-semibold">{phoneTitle[l.kind].n}. {phoneTitle[l.kind].title}</p>
            <p className="text-xs text-slate-500">{phoneTitle[l.kind].hint}</p>
            <img alt={`QR: ${l.url}`} className="mt-3 w-40 rounded-xl bg-white p-2" src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(l.qr_svg)}`} />
            <code className="mt-2 break-all font-mono text-[11px] text-slate-500">{l.url}</code>
          </div>
        ))}
      </div>
      <div className="mt-5 grid gap-4 text-sm text-slate-600 sm:grid-cols-2 dark:text-slate-300">
        <div>
          <p className="font-semibold text-slate-800 dark:text-slate-100">Android</p>
          <ol className="mt-1 list-decimal pl-5">
            <li>Код 1 → браузер предупредит один раз: «Дополнительно» → «Перейти». Файл <b>crm-ca.crt</b> скачается.</li>
            <li>Настройки → Безопасность → <b>Шифрование и учётные данные</b> → Установить сертификат → <b>Сертификат ЦС</b> → выбрать crm-ca.crt.</li>
            <li>Код 2 (или 3 из дома) → вход в CRM. Меню браузера → «Добавить на главный экран».</li>
          </ol>
        </div>
        <div>
          <p className="font-semibold text-slate-800 dark:text-slate-100">iPhone</p>
          <ol className="mt-1 list-decimal pl-5">
            <li>Код 1 в <b>Safari</b> → «Показать подробности» → «посетить веб-сайт» → «Разрешить» загрузку профиля.</li>
            <li>Настройки → <b>Профиль загружен</b> → Установить.</li>
            <li>Настройки → Основные → Об этом устройстве → <b>Доверие сертификатам</b> → включить «CRM».</li>
            <li>Код 2 (или 3 из дома) → вход. «Поделиться» → «На экран „Домой“».</li>
          </ol>
        </div>
      </div>
      <p className="mt-4 text-xs text-slate-500">Сверьте отпечаток при установке с тем, что указан выше в «Сертификат». Код «CRM из интернета» появится, когда в «Удалённый доступ» указан внешний адрес офиса.</p>
    </section>
  );
}
