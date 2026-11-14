import { NavLink, Outlet, useNavigate } from 'react-router';
import { Activity, BookOpen, Building2, DatabaseBackup, KeyRound, LifeBuoy, ListChecks, LogOut, MessagesSquare, Server, Users } from 'lucide-react';
import type { ComponentType } from 'react';
import { api, roleName } from '../api';
import { useAuth } from '../auth';
import { Avatar } from './ui';

function Item({ to, icon: Icon, label, soon }: { to: string; icon: ComponentType<{ className?: string }>; label: string; soon?: boolean }) {
  return (
    <NavLink
      to={to}
      end={to === '/'}
      className={({ isActive }) =>
        `group flex items-center gap-3 rounded-xl px-3 py-2 text-sm font-medium transition ${
          isActive
            ? 'bg-brand-50 text-brand-700 dark:bg-brand-500/15 dark:text-brand-100'
            : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100'
        }`
      }
    >
      <Icon className="size-[18px]" />
      <span className="flex-1">{label}</span>
      {soon && <span className="rounded-md bg-slate-100 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-slate-500 dark:bg-slate-800">скоро</span>}
    </NavLink>
  );
}

export function Shell() {
  const { me, setMe } = useAuth();
  const navigate = useNavigate();
  if (!me) return null;
  const isOwner = me.role === 'host';

  async function logout() {
    await api.logout().catch(() => {});
    setMe(null);
    navigate('/login', { replace: true });
  }

  return (
    <div className="flex min-h-dvh">
      <aside className="sticky top-0 flex h-dvh w-64 shrink-0 flex-col border-r border-slate-200/80 bg-white/70 px-3 py-5 backdrop-blur dark:border-slate-800 dark:bg-slate-900/60">
        <div className="mb-7 flex items-center gap-2.5 px-3 text-[17px] font-bold tracking-tight">
          <img src="/logo.png" alt="" className="size-8 rounded-lg" /> CRM
        </div>
        <nav className="flex flex-col gap-1">
          <Item to="/" icon={Building2} label="Клиенты" soon />
          <Item to="/tasks" icon={ListChecks} label="Задачи" soon />
          <Item to="/messenger" icon={MessagesSquare} label="Мессенджер" soon />
        </nav>
        {isOwner && (
          <>
            <p className="mb-2 mt-8 px-3 text-[11px] font-semibold uppercase tracking-wider text-slate-400">Администрирование</p>
            <nav className="flex flex-col gap-1">
              <Item to="/admin/staff" icon={Users} label="Сотрудники" />
              <Item to="/admin/sessions" icon={KeyRound} label="Активные входы" />
              <Item to="/admin/storage" icon={DatabaseBackup} label="Хранилище и копии" />
              <Item to="/admin/server" icon={Server} label="Сервер" />
              <Item to="/admin/help" icon={BookOpen} label="Инструкция администратора" />
            </nav>
          </>
        )}
        <nav className="mt-auto mb-2 flex flex-col gap-1">
          <Item to="/help" icon={LifeBuoy} label="Справка" />
        </nav>
        <div className="flex items-center gap-3 rounded-xl p-2 hover:bg-slate-100 dark:hover:bg-slate-800">
          <Avatar name={me.display_name} size="sm" />
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-semibold">{me.display_name}</p>
            <p className="truncate text-xs text-slate-500">{roleName[me.role]}</p>
          </div>
          <button onClick={logout} title="Выйти" className="rounded-lg p-1.5 text-slate-400 hover:bg-white hover:text-red-600 dark:hover:bg-slate-900">
            <LogOut className="size-4" />
          </button>
        </div>
      </aside>
      <main className="min-w-0 flex-1 px-6 py-10 sm:px-10 lg:px-14">
        <div className="mx-auto max-w-5xl animate-rise">
          <Outlet />
        </div>
      </main>
    </div>
  );
}

export function ComingSoon({ title, text }: { title: string; text: string }) {
  return (
    <div className="card mt-6 flex flex-col items-center px-8 py-20 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-brand-50 text-brand-600 dark:bg-brand-500/15">
        <Activity className="size-7" />
      </div>
      <h1 className="mt-5 text-xl font-bold">{title}</h1>
      <p className="mt-2 max-w-md text-slate-500">{text}</p>
    </div>
  );
}
