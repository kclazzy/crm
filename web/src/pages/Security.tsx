import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { Link } from 'react-router';
import { Check, Copy, Download, ShieldCheck, Smartphone } from 'lucide-react';
import { api, type TwoFactorPolicy } from '../api';
import { Badge, Button, ErrorText, Field, PageHeader } from '../components/ui';

type Status = Awaited<ReturnType<typeof api.twoFactor>>;
type Started = Awaited<ReturnType<typeof api.twoFactorStart>>;

const policyText: Record<TwoFactorPolicy, string> = {
  off: 'Организация не требует код. Включите его сами — тогда при входе из интернета без кода не войти даже с паролем.',
  remote: 'При входе из интернета (из дома, с мобильного) нужен код. В офисе — только пароль.',
  all: 'Код нужен при каждом входе — и в офисе, и из интернета.',
};

/** Сохранить текст файлом. Ссылка должна быть в документе, иначе часть браузеров молчит. */
function saveText(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

/** Код из приложения (второй фактор): включить, перенастроить, запасные коды. */
export function Security() {
  const [st, setSt] = useState<Status | null>(null);
  const [started, setStarted] = useState<Started | null>(null);
  const [codes, setCodes] = useState<string[] | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const load = useCallback(() => api.twoFactor().then(setSt).catch((e) => setError(e.message)), []);
  useEffect(() => void load(), [load]);

  async function start() {
    setError('');
    try { setStarted(await api.twoFactorStart()); } catch (x) { setError((x as Error).message); }
  }

  async function confirm(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError('');
    try {
      const r = await api.twoFactorConfirm(String(new FormData(e.currentTarget).get('code')));
      setCodes(r.backup_codes);
      setStarted(null);
      void load();
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (!st) return <ErrorText>{error}</ErrorText>;
  const required = st.policy === 'all' || st.policy === 'remote';

  return (
    <>
      <PageHeader title="Код из приложения" subtitle="Второй шаг входа: 6 цифр из приложения на телефоне. Даже узнав пароль, без вашего телефона войти нельзя." />
      <div className="flex max-w-2xl flex-col gap-4">
        <ErrorText>{error}</ErrorText>
        {!st.enabled && st.policy === 'all' && !started && !codes && (
          <p className="card border-amber-300 p-4 text-sm text-amber-800 dark:text-amber-200">
            Организация требует код при каждом входе. Настройте его — после этого CRM откроется.
          </p>
        )}
        <section className="card p-5">
          <div className="mb-2 flex flex-wrap items-center gap-2">
            <h2 className="font-semibold">Состояние</h2>
            {st.enabled ? <Badge tone="green" dot>включён</Badge> : <Badge tone="slate" dot>не настроен</Badge>}
          </div>
          <p className="text-sm text-slate-600 dark:text-slate-300">{policyText[st.policy]}</p>
          {st.enabled && (
            <p className="mt-2 text-sm text-slate-600 dark:text-slate-300">
              Запасных кодов осталось: <b>{st.backup_left}</b>{st.backup_left < 3 && ' — перенастройте код, чтобы получить новые.'}
            </p>
          )}
          {!started && !codes && (
            <div className="mt-4 flex flex-wrap gap-2">
              <Button onClick={start}><Smartphone className="size-4" />{st.enabled ? 'Перенастроить (новый телефон)' : 'Настроить код'}</Button>
              {st.enabled && !required && (
                <Button variant="ghost" onClick={async () => { await api.twoFactorDisable(); void load(); }}>Выключить</Button>
              )}
            </div>
          )}
        </section>

        {started && (
          <section className="card p-5">
            <h2 className="mb-3 font-semibold">Настройка — 2 минуты</h2>
            <ol className="mb-4 list-decimal space-y-1 pl-5 text-sm text-slate-600 dark:text-slate-300">
              <li>Поставьте на телефон приложение-аутентификатор: <b>Яндекс Ключ</b>, <b>Google Authenticator</b> или <b>Microsoft Authenticator</b>.</li>
              <li>В приложении: «+» → «Сканировать QR-код» → наведите камеру на код ниже.</li>
              <li>Введите 6 цифр, которые появятся в приложении.</li>
            </ol>
            <div className="flex flex-col items-center gap-3 sm:flex-row sm:items-start">
              <img alt="QR-код для приложения-аутентификатора" className="w-52 rounded-xl bg-white p-2" src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(started.qr_svg)}`} />
              <div className="min-w-0 flex-1 text-sm">
                <p className="text-slate-600 dark:text-slate-300">Не получается сканировать (настраиваете на этом же телефоне)? Введите ключ вручную, тип — «по времени»:</p>
                <p className="mt-2 break-all rounded-lg bg-slate-100 px-3 py-2 font-mono text-[13px] select-all dark:bg-slate-800">{started.secret}</p>
                <a href={started.url} className="mt-2 inline-block text-brand-600 hover:underline">Открыть в приложении на этом телефоне</a>
              </div>
            </div>
            <form className="mt-4 flex flex-wrap items-end gap-2" onSubmit={confirm}>
              <Field label="Код из приложения">
                <input name="code" className="input w-40 text-center text-xl tracking-[0.25em]" inputMode="numeric" autoComplete="one-time-code" required maxLength={7} autoFocus />
              </Field>
              <Button type="submit" busy={busy}><ShieldCheck className="size-4" />Включить</Button>
              <Button type="button" variant="ghost" onClick={() => setStarted(null)}>Отмена</Button>
            </form>
          </section>
        )}

        {codes && (
          <section className="card p-5">
            <h2 className="mb-1 font-semibold">Код включён. Сохраните запасные коды</h2>
            <p className="mb-3 text-sm text-slate-600 dark:text-slate-300">
              Если телефон потеряется — войдите одним из этих кодов (каждый срабатывает один раз). Показываются только сейчас:
              скачайте или распечатайте и храните отдельно от телефона.
            </p>
            <div className="grid grid-cols-2 gap-2 rounded-xl bg-slate-100 p-3 font-mono text-[15px] sm:grid-cols-5 dark:bg-slate-800">
              {codes.map((c) => <span key={c} className="select-all">{c}</span>)}
            </div>
            <div className="mt-3 flex flex-wrap gap-2">
              <Button variant="secondary" onClick={() => saveText('crm-запасные-коды.txt', `Запасные коды CRM — каждый один раз:\n\n${codes.join('\n')}\n`)}>
                <Download className="size-4" />Скачать
              </Button>
              <Button variant="secondary" onClick={async () => { await navigator.clipboard.writeText(codes.join('\n')); setCopied(true); }}>
                {copied ? <Check className="size-4" /> : <Copy className="size-4" />}{copied ? 'Скопировано' : 'Скопировать'}
              </Button>
              <Link to="/" className="ml-auto"><Button>Готово</Button></Link>
            </div>
          </section>
        )}
      </div>
    </>
  );
}
