import { type FormEvent, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { api, ApiError, SECURITY_PATH } from '../api';
import { AuthLayout } from '../components/AuthLayout';
import { Button, ErrorText, Field } from '../components/ui';
import { useAuth } from '../auth';

export function Login() {
  const { setMe } = useAuth();
  const navigate = useNavigate();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  // Пароль верный, сервер просит код из приложения (вход из интернета).
  const [creds, setCreds] = useState<{ username: string; password: string } | null>(null);

  async function enter(username: string, password: string, code?: string) {
    setBusy(true);
    setError('');
    try {
      const { user, must_setup_2fa } = await api.login(username, password, code);
      setMe(user);
      navigate(must_setup_2fa ? SECURITY_PATH : '/', { replace: true });
    } catch (x) {
      if (x instanceof ApiError && x.data.need_code) {
        setCreds({ username, password });
        if (code) setError(x.message);
      } else {
        setError((x as Error).message);
      }
    } finally {
      setBusy(false);
    }
  }

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    if (creds) void enter(creds.username, creds.password, String(f.get('code')));
    else void enter(String(f.get('username')), String(f.get('password')));
  }

  if (creds) {
    return (
      <AuthLayout title="Код из приложения" subtitle="Пароль верный. Откройте приложение-аутентификатор на телефоне и введите 6 цифр для CRM.">
        <form key="code" onSubmit={submit} className="flex flex-col gap-5">
          <Field label="Код из приложения" hint="Нет телефона под рукой — введите один из запасных кодов.">
            <input key="code" name="code" defaultValue="" className="input text-center text-2xl tracking-[0.3em]" inputMode="numeric" autoComplete="one-time-code" autoFocus required maxLength={20} />
          </Field>
          <ErrorText>{error}</ErrorText>
          <Button type="submit" busy={busy} className="py-3">Войти</Button>
          <button type="button" className="text-sm font-semibold text-slate-500 hover:text-brand-600" onClick={() => { setCreds(null); setError(''); }}>← Другой логин</button>
        </form>
      </AuthLayout>
    );
  }

  return (
    <AuthLayout title="Вход" subtitle="Войдите под своей учётной записью.">
      <form onSubmit={submit} className="flex flex-col gap-5">
        <Field label="Логин">
          <input name="username" className="input" autoComplete="username" autoFocus required spellCheck={false} />
        </Field>
        <Field label="Пароль">
          <input name="password" type="password" className="input" autoComplete="current-password" required />
        </Field>
        <ErrorText>{error}</ErrorText>
        <Button type="submit" busy={busy} className="mt-1 py-3">Войти</Button>
        <Link to="/forgot" className="-mt-1 text-center text-sm font-semibold text-slate-500 hover:text-brand-600 hover:underline">Забыли пароль?</Link>
        <p className="text-center text-sm text-slate-500">
          Есть приглашение?{' '}
          <Link to="/invite" className="font-semibold text-brand-600 hover:underline">Принять приглашение</Link>
        </p>
      </form>
    </AuthLayout>
  );
}
