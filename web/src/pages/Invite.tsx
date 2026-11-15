import { type FormEvent, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { api, SECURITY_PATH } from '../api';
import { AuthLayout } from '../components/AuthLayout';
import { Button, ErrorText, Field } from '../components/ui';
import { useAuth } from '../auth';

export function Invite() {
  const [params] = useSearchParams();
  const { setMe } = useAuth();
  const navigate = useNavigate();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    if (f.get('password') !== f.get('password2')) return setError('Пароли не совпадают');
    setBusy(true);
    setError('');
    try {
      const { user, must_setup_2fa } = await api.acceptInvite(String(f.get('code')).trim(), String(f.get('password')));
      setMe(user);
      navigate(must_setup_2fa ? SECURITY_PATH : '/', { replace: true });
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthLayout title="Добро пожаловать" subtitle="Придумайте пароль — им вы будете входить в CRM.">
      <form onSubmit={submit} className="flex flex-col gap-5">
        <Field label="Приглашение" hint="Его выдаёт владелец CRM. Действует 72 часа, один раз.">
          <input name="code" className="input font-mono text-sm" defaultValue={params.get('code') ?? ''} required spellCheck={false} />
        </Field>
        <Field label="Пароль" hint="Не короче 10 символов и без логина внутри">
          <input name="password" type="password" className="input" autoComplete="new-password" minLength={10} required />
        </Field>
        <Field label="Повторите пароль">
          <input name="password2" type="password" className="input" autoComplete="new-password" minLength={10} required />
        </Field>
        <ErrorText>{error}</ErrorText>
        <Button type="submit" busy={busy} className="mt-1 py-3">Начать работу</Button>
        <p className="text-center text-sm text-slate-500">
          Уже есть пароль? <Link to="/login" className="font-semibold text-brand-600 hover:underline">Войти</Link>
        </p>
      </form>
    </AuthLayout>
  );
}
