import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router';
import { CheckCircle2, ChevronRight, Download, FileSpreadsheet, Undo2 } from 'lucide-react';
import { timeAgo } from '../api';
import { type ImportRecord, work } from '../work';
import { BackButton, Badge, Button, ErrorText, PageHeader } from '../components/ui';
import { useGroups } from '../components/work';

const fields: { key: keyof ImportRecord; label: string; required?: boolean }[] = [
  { key: 'name', label: 'Название / ФИО', required: true },
  { key: 'kind', label: 'Тип' },
  { key: 'phone', label: 'Телефон' },
  { key: 'email', label: 'Email' },
  { key: 'description', label: 'Описание' },
  { key: 'tags', label: 'Теги' },
  { key: 'manager', label: 'Менеджер' },
];

type Parsed = Awaited<ReturnType<typeof work.importParse>>;
type Check = Awaited<ReturnType<typeof work.importCheck>>[number];
type Action = 'create' | 'update' | 'skip';

function Step({ n, title, active, done }: { n: number; title: string; active: boolean; done: boolean }) {
  return (
    <div className={`flex items-center gap-2 text-sm ${active ? 'font-semibold text-brand-700 dark:text-brand-100' : 'text-slate-500'}`}>
      <span className={`grid size-6 place-items-center rounded-full text-xs ${done ? 'bg-emerald-500 text-white' : active ? 'bg-brand-600 text-white' : 'bg-slate-200 dark:bg-slate-700'}`}>{done ? '✓' : n}</span>{title}
    </div>
  );
}

/** Импорт клиентов: файл → колонки → проверка и дубли → импорт (отмена — сутки). */
export function ImportClients() {
  const groups = useGroups();
  const [step, setStep] = useState(1);
  const [parsed, setParsed] = useState<Parsed | null>(null);
  const [mapping, setMapping] = useState<Record<string, number | ''>>({});
  const [group, setGroup] = useState('default');
  const [checks, setChecks] = useState<Check[]>([]);
  const [actions, setActions] = useState<Action[]>([]);
  const [result, setResult] = useState<{ created: number; updated: number; skipped: number } | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [batches, setBatches] = useState<Awaited<ReturnType<typeof work.importBatches>>>([]);

  const loadBatches = () => work.importBatches().then(setBatches).catch(() => {});
  useEffect(() => void loadBatches(), []);

  const records: ImportRecord[] = useMemo(
    () => (parsed?.rows ?? []).map((r) => {
      const rec: ImportRecord = { name: '' };
      for (const f of fields) {
        const i = mapping[f.key];
        if (i !== '' && i !== undefined) rec[f.key] = (r[i] ?? '').trim();
      }
      return rec;
    }),
    [parsed, mapping],
  );

  async function onFile(file: File) {
    setBusy(true);
    setError('');
    try {
      const p = await work.importParse(file);
      setParsed(p);
      const m: Record<string, number | ''> = {};
      for (const f of fields) m[f.key] = p.mapping[f.key] ?? '';
      setMapping(m);
      setStep(2);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function check() {
    setBusy(true);
    setError('');
    try {
      const c = await work.importCheck(records, group);
      setChecks(c);
      setActions(c.map((x) => (x.errors.length ? 'skip' : x.duplicate ? 'skip' : 'create')));
      setStep(3);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function commit() {
    if (!parsed) return;
    setBusy(true);
    setError('');
    try {
      const rows = records.map((r, i) => ({ ...r, action: actions[i], target_id: actions[i] === 'update' ? checks[i].duplicate?.id : undefined }));
      setResult(await work.importCommit(parsed.file_name, group, rows));
      setStep(4);
      loadBatches();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const counts = { create: actions.filter((a) => a === 'create').length, update: actions.filter((a) => a === 'update').length, skip: actions.filter((a) => a === 'skip').length };

  return (
    <>
      <nav className="mb-3 flex items-center gap-3 text-sm text-slate-500"><BackButton fallback="/clients" /><Link to="/clients" className="hover:text-brand-600">Клиенты</Link><ChevronRight className="size-4" /></nav>
      <PageHeader title="Импорт клиентов" subtitle="Из Excel (.xlsx) или CSV. Ничего не запишется, пока вы не подтвердите." />
      <div className="mb-6 flex flex-wrap gap-5">
        {['Файл', 'Колонки', 'Проверка', 'Готово'].map((t, i) => <Step key={t} n={i + 1} title={t} active={step === i + 1} done={step > i + 1} />)}
      </div>
      <ErrorText>{error}</ErrorText>

      {step === 1 && (
        <div className="card flex flex-col items-center gap-4 px-6 py-12 text-center">
          <FileSpreadsheet className="size-12 text-brand-500" />
          <p className="max-w-md text-slate-600 dark:text-slate-300">Первая строка файла — заголовки колонок: «ФИО», «Телефон», «Email», «Описание»… Колонки сопоставятся сами, их можно поправить.</p>
          <label className="cursor-pointer">
            <span className="inline-flex items-center gap-2 rounded-xl bg-brand-600 px-5 py-2.5 text-sm font-semibold text-white hover:bg-brand-700">{busy ? 'Читаю…' : 'Выбрать файл'}</span>
            <input type="file" hidden accept=".xlsx,.xls,.csv,.txt" onChange={(e) => e.target.files?.[0] && void onFile(e.target.files[0])} />
          </label>
          <a href="/api/import/clients/template" className="inline-flex items-center gap-1.5 text-sm text-brand-600"><Download className="size-4" />Скачать шаблон</a>
        </div>
      )}

      {step === 2 && parsed && (
        <div className="card p-5">
          <p className="mb-4 text-sm text-slate-500">{parsed.file_name}: строк — {parsed.rows.length}.</p>
          <div className="grid gap-3 sm:grid-cols-2">
            {fields.map((f) => (
              <label key={f.key} className="flex items-center gap-3 text-sm">
                <span className="w-36 shrink-0 font-medium">{f.label}{f.required && ' *'}</span>
                <select className="input py-2" value={mapping[f.key] ?? ''} onChange={(e) => setMapping({ ...mapping, [f.key]: e.target.value === '' ? '' : Number(e.target.value) })}>
                  <option value="">— не загружать —</option>
                  {parsed.headers.map((h, i) => <option key={i} value={i}>{h || `Колонка ${i + 1}`}</option>)}
                </select>
              </label>
            ))}
            <label className="flex items-center gap-3 text-sm">
              <span className="w-36 shrink-0 font-medium">В группу</span>
              <select className="input py-2" value={group} onChange={(e) => setGroup(e.target.value)}>{groups.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}</select>
            </label>
          </div>
          <div className="mt-5 overflow-x-auto">
            <p className="mb-2 text-sm font-semibold">Первые строки</p>
            <table className="w-full text-left text-xs">
              <thead><tr>{fields.filter((f) => mapping[f.key] !== '').map((f) => <th key={f.key} className="px-2 py-1 text-slate-500">{f.label}</th>)}</tr></thead>
              <tbody>{records.slice(0, 5).map((r, i) => <tr key={i} className="border-t border-slate-100 dark:border-slate-800">{fields.filter((f) => mapping[f.key] !== '').map((f) => <td key={f.key} className="max-w-48 truncate px-2 py-1">{r[f.key]}</td>)}</tr>)}</tbody>
            </table>
          </div>
          <div className="mt-5 flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setStep(1)}>Другой файл</Button>
            <Button busy={busy} disabled={mapping.name === ''} onClick={check}>Проверить</Button>
          </div>
        </div>
      )}

      {step === 3 && (
        <div className="card p-5">
          <p className="mb-3 text-sm">Будет создано: <b>{counts.create}</b> · обновлено: <b>{counts.update}</b> · пропущено: <b>{counts.skip}</b></p>
          <div className="max-h-[55vh] overflow-auto">
            <table className="w-full text-left text-sm">
              <thead className="sticky top-0 bg-white text-xs text-slate-500 dark:bg-slate-900"><tr><th className="px-2 py-1.5">#</th><th className="px-2">Название</th><th className="px-2">Проверка</th><th className="px-2">Действие</th></tr></thead>
              <tbody>
                {records.map((r, i) => (
                  <tr key={i} className="border-t border-slate-100 dark:border-slate-800">
                    <td className="px-2 py-1.5 text-slate-400">{i + 2}</td>
                    <td className="px-2">{r.name || <i className="text-slate-400">пусто</i>}</td>
                    <td className="px-2">
                      {checks[i]?.errors.length ? <span className="text-red-600">{checks[i].errors.join('; ')}</span>
                        : checks[i]?.duplicate ? <Badge tone="amber">похож на «{checks[i].duplicate!.name}» ({checks[i].duplicate!.by})</Badge>
                        : <Badge tone="green">ок</Badge>}
                    </td>
                    <td className="px-2">
                      <select className="input py-1 text-sm" value={actions[i]} disabled={!!checks[i]?.errors.length}
                        onChange={(e) => setActions(actions.map((a, j) => (j === i ? (e.target.value as Action) : a)))}>
                        <option value="create">Создать</option>
                        {checks[i]?.duplicate && <option value="update">Обновить существующего</option>}
                        <option value="skip">Пропустить</option>
                      </select>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="mt-5 flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setStep(2)}>Назад</Button>
            <Button busy={busy} disabled={counts.create + counts.update === 0} onClick={commit}>Импортировать</Button>
          </div>
        </div>
      )}

      {step === 4 && result && (
        <div className="card flex flex-col items-center gap-3 px-6 py-12 text-center">
          <CheckCircle2 className="size-12 text-emerald-500" />
          <p className="text-lg font-semibold">Готово: создано {result.created}, обновлено {result.updated}, пропущено {result.skipped}</p>
          <p className="text-sm text-slate-500">Если что-то пошло не так — импорт можно отменить в течение суток (ниже).</p>
          <Link to="/clients"><Button>К клиентам</Button></Link>
        </div>
      )}

      {batches.length > 0 && (
        <div className="card mt-8 p-5">
          <h3 className="mb-3 font-semibold">Последние импорты</h3>
          <ul className="divide-y divide-slate-100 text-sm dark:divide-slate-800">
            {batches.map((b) => {
              const s = (() => { try { return JSON.parse(b.stats) as { created: number }; } catch { return { created: 0 }; } })();
              return (
                <li key={b.id} className="flex items-center gap-3 py-2">
                  <span className="flex-1">{b.file_name} · {timeAgo(b.created_at)} · создано {s.created}</span>
                  {b.undone_at ? <Badge tone="slate">отменён</Badge> : b.can_undo && (
                    <Button variant="ghost" className="px-2 py-1 text-xs" onClick={async () => {
                      const r = await work.importUndo(b.id);
                      setError(r.kept ? `Удалено ${r.removed}; ${r.kept} оставлены — по ним уже заведены задачи.` : '');
                      loadBatches();
                    }}><Undo2 className="size-3.5" />Отменить</Button>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </>
  );
}
