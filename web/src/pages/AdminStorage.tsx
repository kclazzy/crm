import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { AlertTriangle, CheckCircle2, DatabaseBackup, FolderInput, HardDrive, Plus, RefreshCw, Trash2, XCircle } from 'lucide-react';
import { api, type BackupFile, type BackupStatus, bytes, type StorageInfo, timeAgo } from '../api';
import { BackupScheme } from '../components/BackupScheme';
import { Badge, Button, ErrorText, Field, Modal, PageHeader } from '../components/ui';

const levelColor = { ok: 'bg-emerald-500', warning: 'bg-amber-500', critical: 'bg-red-500' };

function Volumes({ info }: { info: StorageInfo }) {
  return (
    <div className="grid gap-4 md:grid-cols-2">
      {info.volumes.map((v) => {
        const used = v.total_bytes - v.free_bytes;
        const pct = v.total_bytes ? Math.round((used / v.total_bytes) * 100) : 0;
        const crmPct = v.total_bytes ? Math.max(1, (v.crm_bytes / v.total_bytes) * 100) : 0;
        return (
          <section key={v.path + v.roles.join()} className="card p-5">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-2.5 font-semibold"><HardDrive className="size-5 text-slate-400" />Диск {v.path}</div>
              <div className="flex gap-1.5">{v.roles.map((r) => <Badge key={r} tone="brand">{r}</Badge>)}</div>
            </div>
            <div className="mt-4 flex h-2.5 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800" title={`Занято ${pct}%`}>
              <div className="bg-brand-600" style={{ width: `${crmPct}%` }} />
              <div className={`${levelColor[v.level]} opacity-60`} style={{ width: `${Math.max(0, pct - crmPct)}%` }} />
            </div>
            <div className="mt-3 flex flex-wrap justify-between gap-2 text-sm">
              <span className="text-slate-500"><b className="text-slate-900 dark:text-slate-100">{bytes(v.free_bytes)}</b> свободно из {bytes(v.total_bytes)}</span>
              <span className="text-slate-500">CRM занимает {bytes(v.crm_bytes)}</span>
            </div>
            {v.level !== 'ok' && (
              <p className="mt-3 flex items-center gap-2 text-sm text-amber-700 dark:text-amber-300"><AlertTriangle className="size-4" />Диск заполнен на {pct}% — освободите место.</p>
            )}
          </section>
        );
      })}
    </div>
  );
}

function LastBackup({ last }: { last: BackupStatus | null }) {
  if (!last) return <p className="flex items-center gap-2 text-sm text-amber-700 dark:text-amber-300"><AlertTriangle className="size-4" />Копий ещё не было.</p>;
  if (!last.ok) return <p className="flex items-start gap-2 text-sm text-red-600"><XCircle className="mt-0.5 size-4 shrink-0" />Последняя попытка {timeAgo(last.at)} не удалась: {last.error}</p>;
  return <p className="flex items-center gap-2 text-sm text-emerald-700 dark:text-emerald-300"><CheckCircle2 className="size-4" />Последняя копия {timeAgo(last.at)} · {bytes(last.size ?? 0)} · проверена</p>;
}

export function AdminStorage() {
  const [info, setInfo] = useState<StorageInfo | null>(null);
  const [last, setLast] = useState<BackupStatus | null>(null);
  const [files, setFiles] = useState<BackupFile[]>([]);
  const [dirs, setDirs] = useState<string[]>(['']);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<'' | 'save' | 'run' | 'move'>('');
  const [moving, setMoving] = useState(false);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    try {
      const [s, b] = await Promise.all([api.storage(), api.backups()]);
      setInfo(s);
      setLast(b.last);
      setFiles(b.files);
      setDirs(s.backup_dirs.length ? s.backup_dirs : ['']);
    } catch (x) {
      setError((x as Error).message);
    }
  }, []);
  useEffect(() => void load(), [load]);

  async function act(kind: 'save' | 'run', fn: () => Promise<unknown>) {
    setBusy(kind);
    setError('');
    try {
      await fn();
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy('');
      load();
    }
  }

  const saveDirs = () =>
    act('save', async () => {
      const r = await api.setBackupDirs(dirs.map((d) => d.trim()).filter(Boolean));
      setWarnings(r.warnings);
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    });

  async function move(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy('move');
    setError('');
    try {
      await api.moveData(String(new FormData(e.currentTarget).get('dir')));
      setMoving(false);
      load();
    } catch (x) {
      setError((x as Error).message);
    } finally {
      setBusy('');
    }
  }

  if (!info) return <PageHeader title="Хранилище и копии" subtitle={error || 'Загрузка…'} />;

  return (
    <>
      <PageHeader title="Хранилище и копии" subtitle="Где лежат данные CRM и куда сохраняются резервные копии. Видно только владельцу." />
      {info.restart_required && (
        <div className="mb-6 flex items-start gap-3 rounded-2xl border border-amber-300 bg-amber-50 p-4 text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-200">
          <RefreshCw className="mt-0.5 size-5 shrink-0" />
          <div><b>Данные перенесены.</b> Перезапустите сервер: значок CRM в трее → «Перезапустить сервер». До перезапуска CRM работает только на чтение.</div>
        </div>
      )}
      {error && <div className="mb-6"><ErrorText>{error}</ErrorText></div>}

      <Volumes info={info} />

      <div className="mt-6 grid gap-6 lg:grid-cols-2">
        <section className="card p-6">
          <h2 className="flex items-center gap-2 font-semibold"><FolderInput className="size-5 text-brand-600" />Папка данных</h2>
          <p className="mt-1 text-sm text-slate-500">База, ключи и сертификат сервера.</p>
          <code className="mt-4 block rounded-xl border border-slate-200 bg-slate-50 px-3 py-2.5 font-mono text-sm break-all dark:border-slate-700 dark:bg-slate-950">{info.data_dir}</code>
          <Button variant="secondary" className="mt-4" disabled={info.restart_required} onClick={() => { setError(''); setMoving(true); }}>Перенести в другую папку…</Button>
        </section>

        <section className="card p-6">
          <h2 className="flex items-center gap-2 font-semibold"><DatabaseBackup className="size-5 text-brand-600" />Резервные копии</h2>
          <p className="mt-1 text-sm text-slate-500">Каждый час, автоматически. Хранятся все за 48 часов и по одной в сутки за 30 дней.</p>
          <div className="mt-4 flex flex-col gap-2">
            {dirs.map((d, i) => (
              <div key={i} className="flex gap-2">
                <input className="input font-mono text-sm" value={d} placeholder="E:\CRM-backup" spellCheck={false}
                  onChange={(e) => setDirs(dirs.map((x, j) => (j === i ? e.target.value : x)))} />
                {dirs.length > 1 && (
                  <button className="rounded-xl px-2 text-slate-400 hover:bg-slate-100 hover:text-red-600 dark:hover:bg-slate-800" title="Убрать" onClick={() => setDirs(dirs.filter((_, j) => j !== i))}>
                    <Trash2 className="size-4" />
                  </button>
                )}
              </div>
            ))}
          </div>
          {dirs.length < 3 && (
            <button className="mt-2 inline-flex items-center gap-1.5 text-sm font-semibold text-brand-600 hover:underline" onClick={() => setDirs([...dirs, ''])}>
              <Plus className="size-4" />Ещё одна папка
            </button>
          )}
          {warnings.map((w) => <p key={w} className="mt-3 flex items-start gap-2 text-sm text-amber-700 dark:text-amber-300"><AlertTriangle className="mt-0.5 size-4 shrink-0" />{w}</p>)}
          <div className="mt-4 flex flex-wrap gap-2">
            <Button busy={busy === 'save'} onClick={saveDirs}>{saved ? 'Сохранено' : 'Сохранить папки'}</Button>
            <Button variant="secondary" busy={busy === 'run'} disabled={!info.backup_dirs.length} onClick={() => act('run', api.backupNow)}>Сделать копию сейчас</Button>
          </div>
          <div className="mt-4"><LastBackup last={last} /></div>
        </section>
      </div>

      {files.length > 0 && (
        <section className="card mt-6 overflow-hidden">
          <h2 className="px-6 pt-5 font-semibold">Последние копии <span className="font-normal text-slate-500">· {info.backup_dirs[0]}</span></h2>
          <ul className="mt-3 divide-y divide-slate-100 dark:divide-slate-800">
            {files.slice(0, 8).map((f) => (
              <li key={f.name} className="flex justify-between px-6 py-2.5 text-sm">
                <span className="font-mono text-xs">{f.name}</span>
                <span className="text-slate-500">{new Date(f.at).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })} · {bytes(f.size)}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      <div className="mt-6"><BackupScheme /></div>

      <Modal open={moving} onClose={() => setMoving(false)} title="Перенести данные">
        <form onSubmit={move} className="flex flex-col gap-4">
          <p className="text-sm text-slate-600 dark:text-slate-300">Сервер скопирует базу и ключи в новую папку. Старая папка останется — удалите её сами через неделю работы.</p>
          <Field label="Новая папка" hint="Только диск этого компьютера. Сетевые папки, NAS, OneDrive и Яндекс.Диск для базы не подходят — она на них повреждается.">
            <input name="dir" className="input font-mono text-sm" placeholder="D:\CRM\data" required spellCheck={false} />
          </Field>
          <ErrorText>{error}</ErrorText>
          <Button type="submit" busy={busy === 'move'}>Перенести</Button>
        </form>
      </Modal>
    </>
  );
}
