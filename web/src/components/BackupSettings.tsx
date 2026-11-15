import { PathInput } from './FolderPicker';
import { useState } from 'react';
import { AlertTriangle, CheckCircle2, Clock, DatabaseBackup, FolderOpen, Plus, Trash2, XCircle } from 'lucide-react';
import { type BackupPolicy, type BackupStatus, bytes, timeAgo } from '../api';
import { Button } from './ui';

const EVERY = [
  [1, 'Каждый час'], [2, 'Каждые 2 часа'], [3, 'Каждые 3 часа'], [4, 'Каждые 4 часа'],
  [6, 'Каждые 6 часов'], [8, 'Каждые 8 часов'], [12, 'Два раза в день'], [24, 'Раз в день'],
] as const;
const KEEP_ALL = [[6, '6 часов'], [12, '12 часов'], [24, 'сутки'], [48, '2 суток'], [72, '3 суток'], [168, 'неделю']] as const;
const KEEP_DAYS = [[7, 'неделю'], [14, '2 недели'], [30, 'месяц'], [60, '2 месяца'], [90, '3 месяца'], [180, 'полгода'], [365, 'год']] as const;

function label<T extends readonly (readonly [number, string])[]>(list: T, v: number) {
  return list.find(([k]) => k === v)?.[1] ?? `${v}`;
}

/** Сколько копий будет в папке — та же формула, что на сервере (Policy::expected_copies). */
function expected(p: BackupPolicy) {
  return Math.max(1, Math.floor(p.keep_all_hours / p.every_hours)) + Math.max(0, p.keep_days - Math.ceil(p.keep_all_hours / 24));
}

function Step({ n, icon: Icon, title, hint, children }: { n: number; icon: typeof Clock; title: string; hint: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-4">
      <div className="grid size-8 shrink-0 place-items-center rounded-full bg-brand-600 text-sm font-bold text-white">{n}</div>
      <div className="min-w-0 flex-1">
        <h3 className="flex items-center gap-2 font-semibold"><Icon className="size-4 text-brand-600" />{title}</h3>
        <p className="mt-0.5 text-sm text-slate-500">{hint}</p>
        <div className="mt-3">{children}</div>
      </div>
    </div>
  );
}

export function BackupSettings(props: {
  dirs: string[];
  policy: BackupPolicy;
  last: BackupStatus | null;
  hasSavedDirs: boolean;
  busy: '' | 'save' | 'run' | 'move';
  warnings: string[];
  onSave: (dirs: string[], policy: BackupPolicy) => void;
  onRun: () => void;
}) {
  const [dirs, setDirs] = useState(props.dirs.length ? props.dirs : ['']);
  const [p, setP] = useState<BackupPolicy>(props.policy);
  const set = (patch: Partial<BackupPolicy>) => {
    const next = { ...p, ...patch };
    // Ежедневные копии не могут храниться меньше, чем все копии.
    if (next.keep_days * 24 < next.keep_all_hours) next.keep_days = KEEP_DAYS.find(([d]) => d * 24 >= next.keep_all_hours)?.[0] ?? 365;
    setP(next);
  };
  const count = expected(p);
  const size = props.last?.size ? bytes(props.last.size * count) : null;
  const select = 'input w-auto min-w-48 py-2';

  return (
    <section className="card p-6 sm:p-8">
      <h2 className="flex items-center gap-2 text-lg font-semibold"><DatabaseBackup className="size-5 text-brand-600" />Резервные копии</h2>
      <p className="mt-2 max-w-3xl text-[15px] leading-relaxed text-slate-600 dark:text-slate-300">
        Резервная копия — это сохранённый снимок <b>всех</b> данных CRM: клиентов, задач, переписки и сотрудников. Если компьютер сломается,
        выйдет из строя диск или что-то удалят по ошибке, данные можно вернуть из копии. Копии делаются сами, в фоне — сотрудникам это не мешает.
      </p>

      <div className="mt-8 flex flex-col gap-8">
        <Step n={1} icon={FolderOpen} title="Куда сохранять" hint="Лучше всего — второй диск, флешка или сетевое хранилище (NAS). Можно указать до трёх папок: копия попадёт в каждую.">
          <div className="flex max-w-xl flex-col gap-2">
            {dirs.map((d, i) => (
              <div key={i} className="flex gap-2">
                <PathInput value={d} placeholder="E:\CRM-backup" onChange={(v) => setDirs(dirs.map((x, j) => (j === i ? v : x)))} />
                {dirs.length > 1 && (
                  <button className="rounded-xl px-2 text-slate-400 hover:bg-slate-100 hover:text-red-600 dark:hover:bg-slate-800" title="Убрать папку" onClick={() => setDirs(dirs.filter((_, j) => j !== i))}>
                    <Trash2 className="size-4" />
                  </button>
                )}
              </div>
            ))}
            {dirs.length < 3 && (
              <button className="inline-flex w-fit items-center gap-1.5 text-sm font-semibold text-brand-600 hover:underline" onClick={() => setDirs([...dirs, ''])}>
                <Plus className="size-4" />Добавить ещё папку
              </button>
            )}
          </div>
          {props.warnings.map((w) => <p key={w} className="mt-3 flex max-w-xl items-start gap-2 text-sm text-amber-700 dark:text-amber-300"><AlertTriangle className="mt-0.5 size-4 shrink-0" />{w}</p>)}
        </Step>

        <Step n={2} icon={Clock} title="Как часто" hint="Чем чаще — тем меньше работы потеряется при сбое. Для офиса подходит «каждый час».">
          <div className="flex flex-wrap items-center gap-4">
            <select className={select} value={p.every_hours} onChange={(e) => set({ every_hours: Number(e.target.value) })} disabled={!p.enabled}>
              {EVERY.map(([v, t]) => <option key={v} value={v}>{t}</option>)}
            </select>
            <label className="flex cursor-pointer items-center gap-2 text-sm">
              <input type="checkbox" className="size-4 accent-brand-600" checked={p.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
              Делать копии автоматически
            </label>
          </div>
        </Step>

        <Step n={3} icon={DatabaseBackup} title="Сколько хранить" hint="Старые копии удаляются сами, чтобы не переполнить диск.">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-[15px]">
            <span>Хранить все копии за</span>
            <select className={select} value={p.keep_all_hours} onChange={(e) => set({ keep_all_hours: Number(e.target.value) })}>
              {KEEP_ALL.map(([v, t]) => <option key={v} value={v}>{t}</option>)}
            </select>
            <span>, затем по одной копии в день ещё</span>
            <select className={select} value={p.keep_days} onChange={(e) => set({ keep_days: Number(e.target.value) })}>
              {KEEP_DAYS.filter(([d]) => d * 24 >= p.keep_all_hours).map(([v, t]) => <option key={v} value={v}>{t}</option>)}
            </select>
          </div>
        </Step>
      </div>

      <div className="mt-8 rounded-2xl bg-brand-50 p-4 text-[15px] text-brand-900 dark:bg-brand-500/10 dark:text-brand-100">
        {p.enabled ? (
          <>
            <b>Итого:</b> {label(EVERY, p.every_hours).toLowerCase()} сервер сохраняет копию. Все копии хранятся {label(KEEP_ALL, p.keep_all_hours)}, затем по одной в день — {label(KEEP_DAYS, p.keep_days)}.
            В папке будет около <b>{count}</b> копий{size && <> — примерно <b>{size}</b></>}.
          </>
        ) : (
          <><b>Автоматические копии выключены.</b> Делайте копию вручную кнопкой ниже — иначе при сбое данные не восстановить.</>
        )}
      </div>

      <div className="mt-6 flex flex-wrap items-center gap-3">
        <Button busy={props.busy === 'save'} onClick={() => props.onSave(dirs.map((d) => d.trim()).filter(Boolean), p)}>Сохранить настройки</Button>
        <Button variant="secondary" busy={props.busy === 'run'} disabled={!props.hasSavedDirs} onClick={props.onRun}>Сделать копию сейчас</Button>
      </div>
      <div className="mt-4">
        {!props.last && <p className="flex items-center gap-2 text-sm text-amber-700 dark:text-amber-300"><AlertTriangle className="size-4" />Копий ещё не было — укажите папку и сохраните настройки.</p>}
        {props.last && !props.last.ok && <p className="flex items-start gap-2 text-sm text-red-600"><XCircle className="mt-0.5 size-4 shrink-0" />Последняя попытка {timeAgo(props.last.at)} не удалась: {props.last.error}</p>}
        {props.last?.ok && <p className="flex items-center gap-2 text-sm text-emerald-700 dark:text-emerald-300"><CheckCircle2 className="size-4" />Последняя копия {timeAgo(props.last.at)} · {bytes(props.last.size ?? 0)} · целостность проверена</p>}
      </div>
    </section>
  );
}
