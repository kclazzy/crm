import { useEffect, useState, type ComponentType } from 'react';
import { NavLink, Outlet, useLocation, useNavigate } from 'react-router';
import {
  Archive, Building2, HelpCircle, LayoutList, LifeBuoy, ListChecks, LogOut, Menu, MessagesSquare, Search,
  Settings, ShieldCheck, Trash2, WifiOff, X, ChevronDown,
} from 'lucide-react';
import { api, roleName } from '../api';
import { useAuth } from '../auth';
import { useLive } from '../live';
import { Avatar } from './ui';
import { Bell } from './Bell';
import { StepUp } from './StepUp';
import { SearchPalette } from './Search';
import { ADMIN_SECTIONS, AdminTabs, sectionOf } from './AdminSections';
import { ConfirmProvider } from './Confirm';
import { ToastProvider } from './Toast';
import { Tour } from './Tour';
import { helpFor } from './Hint';

function Item({ to, icon: Icon, label, count, onClick, tour }: { to: string; icon: ComponentType<{ className?: string }>; label: string; count?: number; onClick?: () => void; tour?: string }) {
  return (
    <NavLink
      data-tour={tour}
      to={to}
      end={to === '/' || to === '/tasks'}
      onClick={onClick}
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
      {!!count && <span className="rounded-full bg-brand-600 px-1.5 py-0.5 text-[11px] font-bold text-white">{count}</span>}
    </NavLink>
  );
}

/** Пункт раздела администрирования: подсвечен на любой его вкладке. */
function AdminItem({ to, icon: Icon, label, active, onClick }: { to: string; icon: ComponentType<{ className?: string }>; label: string; active: boolean; onClick: () => void }) {
  return (
    <NavLink
      to={to}
      end
      onClick={onClick}
      className={`flex items-center gap-3 rounded-xl px-3 py-2 text-sm font-medium transition ${
        active
          ? 'bg-brand-50 text-brand-700 dark:bg-brand-500/15 dark:text-brand-100'
          : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100'
      }`}
    >
      <Icon className="size-[18px]" />
      <span className="flex-1">{label}</span>
    </NavLink>
  );
}

/** Нижнее меню телефона: главное под большим пальцем. */
function BottomNav({ unread, onSearch }: { unread: number; onSearch: () => void }) {
  const cls = ({ isActive }: { isActive: boolean }) => `flex flex-1 flex-col items-center gap-0.5 py-2 text-[11px] font-medium ${isActive ? 'text-brand-600' : 'text-slate-500'}`;
  return (
    <nav className="fixed inset-x-0 bottom-0 z-30 flex border-t border-slate-200 bg-white/95 pb-[env(safe-area-inset-bottom)] backdrop-blur lg:hidden dark:border-slate-800 dark:bg-slate-900/95">
      <NavLink to="/" end className={cls}><ListChecks className="size-5" />Мои</NavLink>
      <NavLink to="/clients" className={cls}><Building2 className="size-5" />Клиенты</NavLink>
      <NavLink to="/messenger" className={cls}>
        <span className="relative"><MessagesSquare className="size-5" />{unread > 0 && <span className="absolute -right-2 -top-1 rounded-full bg-red-600 px-1 text-[9px] font-bold text-white">{unread}</span>}</span>Чаты
      </NavLink>
      <button onClick={onSearch} className="flex flex-1 flex-col items-center gap-0.5 py-2 text-[11px] font-medium text-slate-500"><Search className="size-5" />Поиск</button>
    </nav>
  );
}

export function Shell() {
  const { me, setMe } = useAuth();
  const { unread, online } = useLive();
  const { pathname } = useLocation();
  // «Настройки» свёрнуты, пока вы не в них: в меню остаётся работа, а не 8 пунктов администрирования.
  const [settingsOpen, setSettingsOpen] = useState(pathname.startsWith('/admin'));
  useEffect(() => { if (pathname.startsWith('/admin')) setSettingsOpen(true); }, [pathname]);
  const navigate = useNavigate();
  const [search, setSearch] = useState(false);
  const [menu, setMenu] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setSearch(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  if (!me) return null;
  const isOwner = me.role === 'host';
  const isManager = me.role !== 'employee';
  const close = () => setMenu(false);

  async function logout() {
    await api.logout().catch(() => {});
    setMe(null);
    navigate('/login', { replace: true });
  }

  const sidebar = (
    <>
      <div className="mb-5 flex items-center gap-2.5 px-3 text-[17px] font-bold tracking-tight">
        <img src="/logo.png" alt="" className="size-8 rounded-lg" /> CRM
        <button className="ml-auto lg:hidden" onClick={close} aria-label="Закрыть меню"><X className="size-5" /></button>
      </div>
      <button onClick={() => { close(); setSearch(true); }} className="mb-5 flex items-center gap-2 rounded-xl border border-slate-200 px-3 py-2 text-sm text-slate-500 hover:border-brand-400 dark:border-slate-700">
        <Search className="size-4" /><span className="flex-1 text-left">Поиск</span><kbd className="rounded bg-slate-100 px-1.5 text-[11px] dark:bg-slate-800">Ctrl K</kbd>
      </button>
      <nav className="flex flex-col gap-1">
        <Item to="/" icon={ListChecks} label="Мои задачи" onClick={close} tour="tasks" />
        {isManager && <Item to="/tasks" icon={LayoutList} label="Все задачи" onClick={close} />}
        <Item to="/clients" icon={Building2} label="Клиенты" onClick={close} />
        <Item to="/messenger" icon={MessagesSquare} label="Мессенджер" count={unread} onClick={close} tour="chats" />
        <Item to="/archive" icon={Archive} label="Архив" onClick={close} />
        {isManager && <Item to="/trash" icon={Trash2} label="Корзина" onClick={close} />}
      </nav>
      {isOwner && (
        <>
          <button data-tour="admin" onClick={() => setSettingsOpen((o) => !o)} className="mt-6 flex w-full items-center gap-3 rounded-xl px-3 py-2 text-[15px] font-medium text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800">
            <Settings className="size-[18px]" /><span className="flex-1 text-left">Администрирование</span>
            <ChevronDown className={`size-4 transition ${settingsOpen ? 'rotate-180' : ''}`} />
          </button>
          <nav className={`ml-3 flex-col gap-1 border-l border-slate-200 pl-2 dark:border-slate-800 ${settingsOpen ? 'flex' : 'hidden'}`}>
            {ADMIN_SECTIONS.map((sec) => (
              <AdminItem key={sec.label} to={sec.tabs[0].to} icon={sec.icon} label={sec.label} active={sectionOf(pathname) === sec} onClick={close} />
            ))}
          </nav>
        </>
      )}
      <nav className="mb-2 mt-auto flex flex-col gap-1 pt-6">
        <Item to="/security" icon={ShieldCheck} label="Код из приложения" onClick={close} tour="code" />
        <Item to="/help" icon={LifeBuoy} label="Справка" onClick={close} />
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
    </>
  );

  return (
    <ToastProvider>
    <div className="flex min-h-dvh">
      <aside className="sticky top-0 hidden h-dvh w-64 shrink-0 flex-col overflow-y-auto border-r border-slate-200/80 bg-white/70 px-3 py-5 backdrop-blur lg:flex dark:border-slate-800 dark:bg-slate-900/60">
        {sidebar}
      </aside>
      {menu && (
        <div className="fixed inset-0 z-40 bg-slate-950/40 lg:hidden" onClick={close}>
          <aside className="flex h-full w-72 flex-col overflow-y-auto bg-white px-3 py-5 dark:bg-slate-900" onClick={(e) => e.stopPropagation()}>{sidebar}</aside>
        </div>
      )}
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="sticky top-0 z-20 flex items-center gap-2 border-b border-slate-200/80 bg-slate-50/90 px-4 py-2 backdrop-blur lg:justify-end lg:border-none lg:bg-transparent lg:px-8 dark:border-slate-800 dark:bg-slate-950/90 lg:dark:bg-transparent">
          <button className="rounded-xl p-2 lg:hidden" onClick={() => setMenu(true)} aria-label="Меню"><Menu className="size-5" /></button>
          <span className="flex-1 font-bold lg:hidden">CRM</span>
          {!online && <span className="inline-flex items-center gap-1 rounded-full bg-amber-50 px-2.5 py-1 text-xs text-amber-800 dark:bg-amber-500/10 dark:text-amber-300" title="Обновления придут после восстановления связи"><WifiOff className="size-3.5" />Нет связи с сервером</span>}
          {!pathname.includes('help') && (
            <NavLink to={helpFor(pathname)} title="Справка по этой странице" className="inline-flex items-center gap-1.5 rounded-xl px-2.5 py-2 text-sm font-medium text-slate-500 hover:bg-slate-100 hover:text-brand-600 dark:hover:bg-slate-800">
              <HelpCircle className="size-4" /><span className="hidden sm:inline">Справка</span>
            </NavLink>
          )}
          <span data-tour="bell"><Bell /></span>
        </header>
        <main className="min-w-0 flex-1 px-4 pb-24 pt-4 sm:px-8 lg:px-12 lg:pb-10">
          <div className="mx-auto max-w-6xl animate-rise">
            <ConfirmProvider>
              {pathname.startsWith('/admin') && <AdminTabs />}
              <Outlet />
            </ConfirmProvider>
          </div>
        </main>
      </div>
      <BottomNav unread={unread} onSearch={() => setSearch(true)} />
      <StepUp />
      <Tour user={me.id} steps={[
        { target: 'tasks', title: 'Здесь ваши задачи', text: 'Где вы исполнитель или постановщик. Красные — просроченные, сверху — что важно сегодня. Нажмите задачу, чтобы открыть её.' },
        { target: 'chats', title: 'Чаты задач', text: 'У каждой задачи свой чат — он справа в карточке задачи. Здесь все чаты сразу, непрочитанные отмечены числом.' },
        { target: 'bell', title: 'Колокольчик', text: 'Напоминания о сроках, новые назначения и важные события. Ненужные можно удалить крестиком.' },
        { target: 'code', title: 'Код из приложения', text: 'Нужен, чтобы входить из дома: 6 цифр в приложении на телефоне. Настройка занимает минуту.' },
        ...(isOwner ? [{ target: 'admin', title: 'Администрирование', text: 'Начинайте с «Обзора»: там всё, что требует внимания, и полоса первоначальной настройки.' }] : []),
      ]} />
      <SearchPalette open={search} onClose={() => setSearch(false)} />
    </div>
    </ToastProvider>
  );
}
