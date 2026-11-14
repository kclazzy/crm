import { type FormEvent, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { api } from '../api';
import { AuthLayout } from '../components/AuthLayout';
import { Button, ErrorText, Field } from '../components/ui';
import { useAuth } from '../auth';

export function Login() {
  const { setMe } = useAuth();
  const navigate = useNavigate();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    setBusy(true);
    setError('');
    try {
      const { user } = await api.login(String(f.get('username')), String(f.get('password')));
      setMe(user);
      navigate(user.role === 'host' ? '/admin/staff' : '/', { replace: true });
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
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
        <p className="text-center text-sm text-slate-500">
          Есть код приглашения?{' '}
          <Link to="/invite" className="font-semibold text-brand-600 hover:underline">Принять приглашение</Link>
        </p>
      </form>
    </AuthLayout>
  );
}
