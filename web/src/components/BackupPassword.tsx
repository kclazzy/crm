import { useState } from 'react';
import { KeyRound, Lock, LockOpen } from 'lucide-react';
import { api } from '../api';
import { useConfirm } from './Confirm';
import { Button, ErrorText, Field } from './ui';

/** Пароль резервных копий: без него копию с флешки не прочитать. */
export function BackupPassword({ encrypted, onChanged }: { encrypted: boolean; onChanged: () => void }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const ask = useConfirm();

  async function save(password: string | null) {
    setBusy(true);
    setError('');
    try {
      await api.setBackupPassword(password);
      setOpen(false);
      onChanged();
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card p-6">
      <h2 className="flex items-center gap-2 font-semibold">
        {encrypted ? <Lock className="size-5 text-emerald-600" /> : <LockOpen className="size-5 text-amber-600" />}
        Пароль копий {encrypted ? '— включён' : '— не задан'}
      </h2>
      <p className="mt-1 max-w-3xl text-sm text-slate-500">
        {encrypted
          ? 'Новые копии зашифрованы: тот, кто унесёт флешку или диск с копиями, ничего не прочитает. Этот сервер открывает свои копии сам; на другом компьютере при восстановлении спросят пароль.'
          : 'Сейчас копию с флешки или сетевой папки может открыть кто угодно — в ней вся база. Задайте пароль, и новые копии будут зашифрованы.'}
      </p>
      {encrypted && (
        <p className="mt-2 max-w-3xl text-sm font-medium text-amber-700 dark:text-amber-300">
          Запишите пароль и храните отдельно от копий. Без него восстановить копию на новом компьютере нельзя — никто, включая нас, его не подскажет.
        </p>
      )}
      {!open ? (
        <div className="mt-4 flex flex-wrap gap-2">
          <Button variant={encrypted ? 'secondary' : 'primary'} onClick={() => { setError(''); setOpen(true); }}>
            <KeyRound className="size-4" />{encrypted ? 'Сменить пароль' : 'Задать пароль'}
          </Button>
          {encrypted && (
            <Button variant="ghost" busy={busy} onClick={async () => {
              if (await ask({ title: 'Выключить шифрование копий?', text: 'Новые копии будут незашифрованными. Уже сделанные останутся под прежним паролем.', ok: 'Выключить', danger: true })) save(null);
            }}>Не шифровать</Button>
          )}
        </div>
      ) : (
        <form className="mt-4 flex max-w-md flex-col gap-3" onSubmit={(e) => {
          e.preventDefault();
          const f = new FormData(e.currentTarget);
          const p = String(f.get('password'));
          if (p !== String(f.get('password2'))) return setError('Пароли не совпадают');
          save(p);
        }}>
          <Field label="Пароль копий" hint="Не короче 10 символов. Прежние копии останутся под старым паролем.">
            <input name="password" type="password" className="input" minLength={10} required autoFocus autoComplete="new-password" />
          </Field>
          <Field label="Повторите пароль"><input name="password2" type="password" className="input" minLength={10} required autoComplete="new-password" /></Field>
          <ErrorText>{error}</ErrorText>
          <div className="flex gap-2">
            <Button type="submit" busy={busy}>Сохранить</Button>
            <Button variant="secondary" onClick={() => setOpen(false)}>Отмена</Button>
          </div>
        </form>
      )}
      {!open && <ErrorText>{error}</ErrorText>}
    </section>
  );
}
