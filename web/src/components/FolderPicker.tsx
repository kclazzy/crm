import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { ArrowUp, Archive, Folder, FolderPlus, HardDrive } from 'lucide-react';
import { request } from '../api';
import { Button, ErrorText, Modal } from './ui';

interface Entry { name: string; path: string; file?: boolean }
interface Listing { path: string | null; parent: string | null; entries: Entry[] }

/** Окно выбора папки на компьютере-сервере: диски, вложенные папки, «Новая папка».
 *  `backups` — ещё и файлы копий CRM (.crmpack, .db.zst): их можно выбрать сразу. */
export function FolderPicker({ start, backups, onPick, onClose }: { start?: string; backups?: boolean; onPick: (path: string) => void; onClose: () => void }) {
  const [list, setList] = useState<Listing | null>(null);
  const [error, setError] = useState('');
  const [naming, setNaming] = useState(false);
  const open = useCallback(async (path?: string | null) => {
    setError('');
    try {
      const q = new URLSearchParams();
      if (path) q.set('path', path);
      if (backups) q.set('files', 'backups');
      setList(await request<Listing>('GET', `/api/admin/fs?${q}`));
    } catch (x) {
      setError((x as Error).message);
      if (path) setList(await request<Listing>('GET', '/api/admin/fs'));
    }
  }, [backups]);
  useEffect(() => void open(start || null), [open, start]);

  return (
    <Modal open onClose={onClose} title={backups ? 'Выбор копии на сервере' : 'Выбор папки на сервере'}>
      <div className="flex flex-col gap-3">
        <p className="text-xs text-slate-500">{backups
          ? 'Нажмите на копию — или выберите папку с копиями, возьмётся самая свежая.'
          : 'Папки компьютера, на котором работает CRM: туда пишет служба сервера.'}</p>
        <div className="flex items-center gap-2">
          <button className="rounded-lg p-2 hover:bg-slate-100 disabled:opacity-40 dark:hover:bg-slate-800" title="Выше" disabled={!list?.path}
            onClick={() => void open(list?.parent ?? null)}><ArrowUp className="size-4" /></button>
          <p className="min-w-0 flex-1 truncate rounded-lg bg-slate-100 px-3 py-2 font-mono text-sm dark:bg-slate-800">{list?.path ?? 'Диски'}</p>
        </div>
        <div className="max-h-72 min-h-40 overflow-auto rounded-xl border border-slate-200 dark:border-slate-700">
          {list?.entries.length === 0 && <p className="p-4 text-sm text-slate-500">Вложенных папок нет — можно выбрать эту или создать новую.</p>}
          {list?.entries.map((e) => (
            <button key={e.path} onClick={() => (e.file ? onPick(e.path) : void open(e.path))} className="flex w-full items-center gap-2 border-b border-slate-100 px-3 py-2 text-left text-sm last:border-0 hover:bg-brand-50 dark:border-slate-800 dark:hover:bg-brand-500/10">
              {e.file ? <Archive className="size-4 text-brand-600" /> : list.path ? <Folder className="size-4 text-amber-500" /> : <HardDrive className="size-4 text-slate-500" />}{e.name}
            </button>
          ))}
        </div>
        {naming && list?.path && (
          <form className="flex gap-2" onSubmit={async (e: FormEvent<HTMLFormElement>) => {
            e.preventDefault();
            const name = String(new FormData(e.currentTarget).get('name'));
            try {
              const made = await request<Entry>('POST', '/api/admin/fs/mkdir', { parent: list.path, name });
              setNaming(false);
              await open(made.path);
            } catch (x) { setError((x as Error).message); }
          }}>
            <input name="name" className="input flex-1" placeholder="CRM-копии" autoFocus required maxLength={120} />
            <Button type="submit">Создать</Button>
          </form>
        )}
        <ErrorText>{error}</ErrorText>
        <div className="flex flex-wrap justify-end gap-2">
          {list?.path && !naming && <Button variant="ghost" onClick={() => setNaming(true)}><FolderPlus className="size-4" />Новая папка</Button>}
          <Button variant="secondary" onClick={onClose}>Отмена</Button>
          <Button disabled={!list?.path} onClick={() => list?.path && onPick(list.path)}>Выбрать эту папку</Button>
        </div>
      </div>
    </Modal>
  );
}

/** Поле пути + «Обзор…». `name` — для обычных форм; `value`/`onChange` — для управляемых. */
export function PathInput({ name, value, onChange, placeholder, required, backups }: {
  name?: string; value?: string; onChange?: (v: string) => void; placeholder?: string; required?: boolean; backups?: boolean;
}) {
  const [own, setOwn] = useState('');
  const [picking, setPicking] = useState(false);
  const v = value ?? own;
  const set = (x: string) => (onChange ? onChange(x) : setOwn(x));
  return (
    <div className="flex min-w-0 flex-1 gap-2">
      <input name={name} className="input min-w-0 flex-1 font-mono text-sm" value={v} placeholder={placeholder} required={required} spellCheck={false}
        onChange={(e) => set(e.target.value)} />
      <Button type="button" variant="secondary" className="shrink-0" onClick={() => setPicking(true)}><Folder className="size-4" />Обзор…</Button>
      {picking && <FolderPicker start={v} backups={backups} onClose={() => setPicking(false)} onPick={(p) => { set(p); setPicking(false); }} />}
    </div>
  );
}
