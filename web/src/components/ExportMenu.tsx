import { Download, FileSpreadsheet, FileText } from 'lucide-react';
import { Menu } from './Menu';

/** «Выгрузить» с выбором формата: что даёт Excel и что — CSV. */
export function ExportMenu({ url, label = 'Выгрузить', compact = false }: { url: string; label?: string; compact?: boolean }) {
  const href = (format: 'xlsx' | 'csv') => (format === 'csv' ? `${url}${url.includes('?') ? '&' : '?'}format=csv` : url);
  const item = (format: 'xlsx' | 'csv', icon: React.ReactNode, title: string, hint: string, close: () => void) => (
    <a href={href(format)} onClick={close} className="flex gap-3 px-4 py-2.5 hover:bg-slate-50 dark:hover:bg-slate-800">
      <span className="mt-0.5 text-brand-600">{icon}</span>
      <span>
        <span className="block text-sm font-semibold">{title}</span>
        <span className="block text-xs text-slate-500">{hint}</span>
      </span>
    </a>
  );
  return (
    <Menu right button={(toggle) => (
      <button onClick={toggle} className={compact
        ? 'inline-flex items-center gap-1 text-sm text-slate-500 hover:text-brand-600'
        : 'inline-flex items-center gap-2 rounded-xl px-3 py-2.5 text-sm font-semibold text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800'}>
        <Download className="size-4" />{label}
      </button>
    )}>
      {(close) => (
        <div className="w-80">
          {item('xlsx', <FileSpreadsheet className="size-5" />, 'Excel (.xlsx)',
            'Таблица с заголовками и шириной колонок — открыть в Excel или Google Таблицах. На листе до 1 048 576 строк, остальное — на следующих листах.', close)}
          {item('csv', <FileText className="size-5" />, 'CSV (.csv)',
            'Простой текст без ограничения строк, быстрее на больших объёмах. Открывается в Excel (разделитель «;», UTF-8), 1С и других программах; без оформления.', close)}
        </div>
      )}
    </Menu>
  );
}
