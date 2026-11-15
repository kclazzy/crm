import { type FormEvent, useCallback, useEffect, useState } from 'react';
import { AlertTriangle, DatabaseZap, FolderInput, HardDrive, History, RefreshCw, Truck } from 'lucide-react';
import { api, type BackupFile, type BackupPolicy, type BackupStatus, bytes, type StorageInfo } from '../api';
import { BackupScheme } from '../components/BackupScheme';
import { BackupSettings } from '../components/BackupSettings';
import { PathInput } from '../components/FolderPicker';
import { Badge, Button, ErrorText, Field, Modal, PageHeader } from '../components/ui';
import { work } from '../work';

/** Что показать в отчёте импорта: таблица → подпись. Остальное (служебное) не перечисляем. */
const importLabels: [string, string][] = [
  ['clients', 'клиенты'], ['tasks', 'задачи'], ['messages', 'сообщения'], ['attachments', 'файлы'],
  ['users', 'новые сотрудники'], ['tags', 'новые метки'],
];

function importReport(r: { added: Record<string, number>; merged_users: number; attachments: number }) {
  const parts = importLabels.filter(([k]) => r.added[k]).map(([k, l]) => `${l} — ${r.added[k]}`);
  const what = parts.length ? `Добавлено: ${parts.join(', ')}.` : 'Новых записей нет — всё уже было в этой базе.';
  return `${what} Сотрудников совпало по логину: ${r.merged_users}, вложений скопировано: ${r.attachments}.`;
}

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
            <p className="mt-1 text-xs text-slate-500">
              {v.days_left != null
                ? v.days_left > 3650 ? 'Прогноз: места хватит на годы.' : `Прогноз: при нынешнем темпе места хватит примерно на ${v.days_left} дн.`
                : 'Прогноз появится через несколько дней работы.'}
            </p>
            {v.level !== 'ok' && (
              <p className="mt-3 flex items-center gap-2 text-sm text-amber-700 dark:text-amber-300"><AlertTriangle className="size-4" />Диск заполнен на {pct}% — освободите место.</p>
            )}
          </section>
        );
      })}
    </div>
  );
}

export function AdminStorage() {
  const [info, setInfo] = useState<StorageInfo | null>(null);
  const [last, setLast] = useState<BackupStatus | null>(null);
  const [files, setFiles] = useState<BackupFile[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<'' | 'save' | 'run' | 'move'>('');
  const [moving, setMoving] = useState(false);
  const [restoring, setRestoring] = useState<BackupFile | null>(null);
  const [movingOut, setMovingOut] = useState(false);
  const [importing, setImporting] = useState(false);
  const [importMode, setImportMode] = useState<'merge' | 'replace'>('merge');
  const [done, setDone] = useState('');

  const load = useCallback(async () => {
    try {
      const [s, b] = await Promise.all([api.storage(), api.backups()]);
      setInfo(s);
      setLast(b.last);
      setFiles(b.files);
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

  const saveBackup = (dirs: string[], policy: BackupPolicy) =>
    act('save', async () => {
      const r = await api.saveBackup(dirs, policy);
      setWarnings(r.warnings);
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

      <div className="mt-6">
        <BackupSettings key={JSON.stringify([info.backup_dirs, info.policy])} dirs={info.backup_dirs} policy={info.policy} last={last}
          hasSavedDirs={info.backup_dirs.length > 0} busy={busy} warnings={warnings} onSave={saveBackup} onRun={() => act('run', api.backupNow)} />
      </div>

      <div className="mt-6">
        <section className="card p-6">
          <h2 className="flex items-center gap-2 font-semibold"><FolderInput className="size-5 text-brand-600" />Папка данных</h2>
          <p className="mt-1 text-sm text-slate-500">База, ключи и сертификат сервера.</p>
          <code className="mt-4 block rounded-xl border border-slate-200 bg-slate-50 px-3 py-2.5 font-mono text-sm break-all dark:border-slate-700 dark:bg-slate-950">{info.data_dir}</code>
          <p className="mt-2 text-xs text-slate-500">
            Папка закрыта для обычных пользователей Windows — так защищены база и ключи: открыть её может только служба CRM и
            администратор компьютера (проводник спросит разрешение, нажмите «Продолжить»). Копировать базу вручную не нужно —
            для этого есть резервные копии и пакет переноса ниже.
          </p>
          <Button variant="secondary" className="mt-4" disabled={info.restart_required} onClick={() => { setError(''); setMoving(true); }}>Перенести в другую папку…</Button>
        </section>
        <section className="card mt-6 p-6">
          <h2 className="flex items-center gap-2 font-semibold"><Truck className="size-5 text-brand-600" />Переезд на другой компьютер</h2>
          <p className="mt-1 text-sm text-slate-500">
            Сервер сделает свежую копию и соберёт <b>пакет переноса</b> (база, вложения, ключи сервера) — например, на флешку. После этого этот сервер перестанет принимать изменения.
            На новом компьютере: установите CRM и выполните от администратора <code className="font-mono">crm-server restore E:\crm_move_….crmpack</code>, затем «Перезапустить сервер».
            Сотрудникам переподключаться не нужно, если у нового компьютера то же имя в сети.
          </p>
          <Button variant="secondary" className="mt-4" disabled={info.restart_required || info.backup_dirs.length === 0} onClick={() => { setError(''); setMovingOut(true); }}>Подготовить пакет переноса…</Button>
          {info.backup_dirs.length === 0 && <p className="mt-2 text-xs text-slate-500">Сначала выберите папку резервных копий.</p>}
        </section>
        <section className="card mt-6 p-6">
          <h2 className="flex items-center gap-2 font-semibold"><DatabaseZap className="size-5 text-brand-600" />Импорт базы с другого сервера</h2>
          <p className="mt-1 text-sm text-slate-500">
            Перенос данных со старого сервера на этот: укажите пакет переноса (<code className="font-mono">.crmpack</code>), копию
            (<code className="font-mono">.db.zst</code>) или папку копий — возьмётся самая свежая. Можно <b>заменить</b> базу этого сервера
            или <b>добавить</b> записи старого к уже существующим.
          </p>
          <Button variant="secondary" className="mt-4" disabled={info.restart_required} onClick={() => { setError(''); setImportMode('merge'); setImporting(true); }}>Импортировать базу…</Button>
        </section>
        {done && <div className="mt-4 rounded-xl bg-emerald-50 px-4 py-3 text-sm text-emerald-800 dark:bg-emerald-500/10 dark:text-emerald-200">{done}</div>}

      </div>

      {files.length > 0 && (
        <section className="card mt-6 overflow-hidden">
          <h2 className="px-6 pt-5 font-semibold">Последние копии <span className="font-normal text-slate-500">· {info.backup_dirs[0]}</span></h2>
          <ul className="mt-3 divide-y divide-slate-100 dark:divide-slate-800">
            {files.slice(0, 8).map((f) => (
              <li key={f.name} className="flex flex-wrap items-center justify-between gap-2 px-6 py-2.5 text-sm">
                <span className="font-mono text-xs">{f.name}</span>
                <span className="flex items-center gap-3 text-slate-500">
                  {new Date(f.at).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })} · {bytes(f.size)}
                  <Button variant="ghost" className="px-2 py-1 text-xs" disabled={info.restart_required} onClick={() => { setError(''); setRestoring(f); }}><History className="size-3.5" />Восстановить</Button>
                </span>
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
            <PathInput name="dir" placeholder="D:\CRM\data" required />
          </Field>
          <ErrorText>{error}</ErrorText>
          <Button type="submit" busy={busy === 'move'}>Перенести</Button>
        </form>
      </Modal>
      <Modal open={!!restoring} onClose={() => setRestoring(null)} title="Восстановить из копии?">
        <p className="text-sm text-slate-600 dark:text-slate-300">
          Данные вернутся к состоянию на <b>{restoring && new Date(restoring.at).toLocaleString('ru-RU')}</b>. Всё, что сделано после, пропадёт из рабочей базы
          (текущая база сохранится рядом как <code className="font-mono">before_restore_….db</code>). Копия будет проверена; встанет на место после перезапуска сервера.
        </p>
        <ErrorText>{error}</ErrorText>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setRestoring(null)}>Отмена</Button>
          <Button variant="danger" busy={busy === 'move'} onClick={async () => {
            if (!restoring) return;
            setBusy('move');
            setError('');
            try {
              const r = await work.restore(restoring.name, null);
              setRestoring(null);
              setDone(`Копия ${r.name} проверена. Перезапустите сервер (трей → «Перезапустить сервер») — данные вернутся.`);
              load();
            } catch (x) { setError((x as Error).message); } finally { setBusy(''); }
          }}>Восстановить</Button>
        </div>
      </Modal>

      <Modal open={importing} onClose={() => setImporting(false)} title="Импорт базы">
        <form className="flex flex-col gap-4" onSubmit={async (e) => {
          e.preventDefault();
          const source = String(new FormData(e.currentTarget).get('source'));
          setBusy('move');
          setError('');
          try {
            const r = await work.importDb(source, importMode);
            setImporting(false);
            setDone('restart' in r
              ? `Копия ${r.staged} проверена и заменит базу после перезапуска сервера (трей → «Перезапустить сервер»).`
              : importReport(r));
            load();
          } catch (x) { setError((x as Error).message); } finally { setBusy(''); }
        }}>
          <Field label="Откуда" hint="Файл или папка на компьютере с CRM — флешка, сетевая папка. Копия от более старой версии CRM подойдёт.">
            <PathInput name="source" placeholder="E:\crm_move_….crmpack" required backups />
          </Field>
          <fieldset className="flex flex-col gap-2">
            <legend className="mb-1 text-sm font-medium">Как импортировать</legend>
            {([
              ['merge', 'Добавить к существующим', 'Клиенты, задачи, чаты и файлы старого сервера добавятся к данным этого. Сотрудники с тем же логином сольются в одного, владелец — с владельцем. Повторный импорт той же копии ничего не задвоит. Перезапуск не нужен.'],
              ['replace', 'С заменой', 'База этого сервера будет заменена копией целиком; всё, что сделано здесь, уйдёт из рабочей базы (сохранится рядом как before_restore_….db). Нужен перезапуск сервера.'],
            ] as const).map(([v, title, hint]) => (
              <label key={v} className={`flex cursor-pointer gap-3 rounded-xl border p-3 text-sm ${importMode === v ? 'border-brand-500 bg-brand-50 dark:bg-brand-500/10' : 'border-slate-200 dark:border-slate-700'}`}>
                <input type="radio" name="mode" value={v} checked={importMode === v} onChange={() => setImportMode(v)} className="mt-0.5" />
                <span><b>{title}</b><span className="mt-0.5 block text-slate-500">{hint}</span></span>
              </label>
            ))}
          </fieldset>
          <ErrorText>{error}</ErrorText>
          <Button type="submit" variant={importMode === 'replace' ? 'danger' : 'primary'} busy={busy === 'move'}>
            {importMode === 'replace' ? 'Заменить базу этого сервера' : 'Добавить данные'}
          </Button>
        </form>
      </Modal>

      <Modal open={movingOut} onClose={() => setMovingOut(false)} title="Пакет переноса">
        <form className="flex flex-col gap-4" onSubmit={async (e) => {
          e.preventDefault();
          setBusy('move');
          setError('');
          try {
            const r = await work.moveOut(String(new FormData(e.currentTarget).get('dir')));
            setMovingOut(false);
            setDone(`Пакет готов: ${r.file}. Этот сервер теперь только для чтения.`);
            load();
          } catch (x) { setError((x as Error).message); } finally { setBusy(''); }
        }}>
          <Field label="Куда сохранить" hint="Флешка или сетевая папка, доступная этому компьютеру">
            <PathInput name="dir" placeholder="E:\" required />
          </Field>
          <ErrorText>{error}</ErrorText>
          <Button type="submit" busy={busy === 'move'}>Собрать пакет и остановить приём изменений</Button>
        </form>
      </Modal>
    </>
  );
}
