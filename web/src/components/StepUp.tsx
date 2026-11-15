import { type FormEvent, useEffect, useState } from 'react';
import { api, NEED_CODE_EVENT } from '../api';
import { Button, ErrorText, Field, Modal } from './ui';

/** Вход открыт в офисе без кода, а работа идёт из интернета: подтвердить кодом из приложения. */
export function StepUp() {
  const [open, setOpen] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const show = () => setOpen(true);
    window.addEventListener(NEED_CODE_EVENT, show);
    return () => window.removeEventListener(NEED_CODE_EVENT, show);
  }, []);
  if (!open) return null;
  return (
    <Modal open onClose={() => setOpen(false)} title="Подтвердите вход кодом">
      <form className="flex flex-col gap-4" onSubmit={async (e: FormEvent<HTMLFormElement>) => {
        e.preventDefault();
        setBusy(true);
        setError('');
        try {
          await api.verifyCode(String(new FormData(e.currentTarget).get('code')));
          location.reload();
        } catch (x) {
          setError((x as Error).message);
        } finally {
          setBusy(false);
        }
      }}>
        <p className="text-sm text-slate-600 dark:text-slate-300">
          Вы работаете не из сети офиса. Введите 6 цифр из приложения-аутентификатора (или запасной код) — один раз для этого входа.
        </p>
        <Field label="Код из приложения">
          <input name="code" className="input text-center text-2xl tracking-[0.3em]" inputMode="numeric" autoComplete="one-time-code" autoFocus required maxLength={20} />
        </Field>
        <ErrorText>{error}</ErrorText>
        <Button type="submit" busy={busy}>Подтвердить</Button>
      </form>
    </Modal>
  );
}
