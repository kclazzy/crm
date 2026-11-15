import { type FormEvent, useState } from 'react';
import { Link } from 'react-router';
import { CheckCircle2, Server } from 'lucide-react';
import { api } from '../api';
import { AuthLayout } from '../components/AuthLayout';
import { Button, ErrorText, Field } from '../components/ui';

type Who = 'employee' | 'owner';

export function Forgot() {
  const [who, setWho] = useState<Who>('employee');
  const [done, setDone] = useState<Who | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    setError('');
    if (who === 'owner' && f.get('password') !== f.get('password2')) return setError('Пароли не совпадают');
    setBusy(true);
    try {
      if (who === 'employee') await api.forgot(String(f.get('username')));
      else await api.recover(String(f.get('username')), String(f.get('code')), String(f.get('password')));
      setDone(who);
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const tab = (w: Who, label: string) => (
    <button type="button" onClick={() => { setWho(w); setError(''); }}
      className={`flex-1 rounded-lg px-3 py-2 text-sm font-semibold transition ${who === w ? 'bg-white text-brand-700 shadow-sm dark:bg-slate-800 dark:text-brand-100' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200'}`}>
      {label}
    </button>
  );

  if (done) {
    return (
      <AuthLayout title={done === 'owner' ? 'Пароль изменён' : 'Запрос отправлен'} subtitle="">
        <div className="flex flex-col gap-5">
          <p className="flex gap-3 text-[15px] text-slate-600 dark:text-slate-300">
            <CheckCircle2 className="size-6 shrink-0 text-emerald-600" />
            {done === 'owner'
              ? 'Войдите с новым паролем. Использованный код больше не действует — когда останется мало кодов, создайте новые в разделе «Сервер».'
              : 'Если такой сотрудник есть, владелец CRM увидит ваш запрос. Он сбросит доступ и пришлёт новую ссылку — по ней вы зададите новый пароль.'}
          </p>
          <Link to="/login"><Button className="w-full py-3">Ко входу</Button></Link>
        </div>
      </AuthLayout>
    );
  }

  return (
    <AuthLayout title="Забыли пароль?" subtitle="Восстановим доступ — выберите, кто вы.">
      <div className="mb-6 flex gap-1 rounded-xl bg-slate-100 p-1 dark:bg-slate-900">{tab('employee', 'Я сотрудник')}{tab('owner', 'Я владелец')}</div>
      <form key={who} onSubmit={submit} className="flex flex-col gap-5">
        <Field label="Логин"><input name="username" className="input" required autoFocus spellCheck={false} autoComplete="username" /></Field>
        {who === 'owner' && (
          <>
            <Field label="Код сброса пароля" hint="Один из 10 кодов, которые вы сохранили или распечатали">
              <input name="code" className="input font-mono uppercase tracking-wider" required placeholder="XXXX-XXXX-XXXX" spellCheck={false} autoComplete="off" />
            </Field>
            <Field label="Новый пароль" hint="Не короче 10 символов и без логина внутри">
              <input name="password" type="password" className="input" minLength={10} required autoComplete="new-password" />
            </Field>
            <Field label="Повторите пароль"><input name="password2" type="password" className="input" minLength={10} required autoComplete="new-password" /></Field>
          </>
        )}
        <ErrorText>{error}</ErrorText>
        <Button type="submit" busy={busy} className="py-3">{who === 'owner' ? 'Сменить пароль' : 'Попросить владельца сбросить пароль'}</Button>
        {who === 'owner' && (
          <p className="flex gap-2.5 rounded-xl bg-slate-100 p-3 text-sm text-slate-600 dark:bg-slate-900 dark:text-slate-300">
            <Server className="mt-0.5 size-4 shrink-0" />
            Нет кода? На самом сервере: значок CRM в трее → «Сбросить пароль владельца…» (понадобятся права администратора Windows).
          </p>
        )}
        <p className="text-center text-sm text-slate-500"><Link to="/login" className="font-semibold text-brand-600 hover:underline">Вернуться ко входу</Link></p>
      </form>
    </AuthLayout>
  );
}
