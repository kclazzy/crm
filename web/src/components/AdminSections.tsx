import { type ComponentType } from 'react';
import { NavLink, useLocation } from 'react-router';
import { DatabaseBackup, Gauge, ShieldCheck, SlidersHorizontal, Users } from 'lucide-react';

/** Разделы администрирования и их вкладки — одно место для меню и вкладок. */
export const ADMIN_SECTIONS: { label: string; icon: ComponentType<{ className?: string }>; tabs: { to: string; label: string }[] }[] = [
  { label: 'Обзор', icon: Gauge, tabs: [{ to: '/admin', label: 'Обзор' }] },
  {
    label: 'Сотрудники',
    icon: Users,
    tabs: [
      { to: '/admin/staff', label: 'Сотрудники' },
      { to: '/admin/groups', label: 'Группы' },
      { to: '/admin/sessions', label: 'Активные входы' },
    ],
  },
  {
    label: 'Безопасность и доступ',
    icon: ShieldCheck,
    tabs: [
      { to: '/admin/remote', label: 'Удалённый доступ' },
      { to: '/admin/protection', label: 'Защита входа' },
      { to: '/admin/journal', label: 'Журнал' },
    ],
  },
  { label: 'Данные', icon: DatabaseBackup, tabs: [{ to: '/admin/storage', label: 'Хранилище и копии' }] },
  {
    label: 'Настройки',
    icon: SlidersHorizontal,
    tabs: [
      { to: '/admin/org', label: 'Рабочее время' },
      { to: '/admin/server', label: 'Сервер' },
    ],
  },
];

/** Раздел, к которому относится адрес. */
export function sectionOf(pathname: string) {
  return ADMIN_SECTIONS.find((s) => s.tabs.some((t) => t.to === pathname));
}

/** Вкладки раздела над страницей (если вкладок больше одной). */
export function AdminTabs() {
  const { pathname } = useLocation();
  const section = sectionOf(pathname);
  if (!section || section.tabs.length < 2) return null;
  return (
    <nav className="-mt-2 mb-6 flex gap-1 overflow-x-auto border-b border-slate-200 dark:border-slate-800" aria-label={section.label}>
      {section.tabs.map((t) => (
        <NavLink
          key={t.to}
          to={t.to}
          end
          className={({ isActive }) =>
            `-mb-px whitespace-nowrap border-b-2 px-3 py-2 text-sm font-medium transition ${
              isActive ? 'border-brand-600 text-brand-700 dark:text-brand-100' : 'border-transparent text-slate-500 hover:text-slate-800 dark:hover:text-slate-200'
            }`
          }
        >
          {t.label}
        </NavLink>
      ))}
    </nav>
  );
}
