import { StrictMode, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, Navigate, Route, Routes } from 'react-router';
import { Loader2 } from 'lucide-react';
import './index.css';
import { AuthProvider, useAuth } from './auth';
import { ComingSoon, Shell } from './components/Shell';
import { Login } from './pages/Login';
import { Invite } from './pages/Invite';
import { AdminStaff } from './pages/AdminStaff';
import { AdminSessions } from './pages/AdminSessions';
import { AdminServer } from './pages/AdminServer';

function Gate({ children, owner = false }: { children: ReactNode; owner?: boolean }) {
  const { me, loading } = useAuth();
  if (loading) return <div className="grid h-dvh place-items-center"><Loader2 className="size-7 animate-spin text-brand-500" /></div>;
  if (!me) return <Navigate to="/login" replace />;
  // Админ-панель — только владельцу; остальным сервер всё равно ответит 403.
  if (owner && me.role !== 'host') return <Navigate to="/" replace />;
  return <>{children}</>;
}

const soon = 'Раздел появится в следующем обновлении.';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AuthProvider>
      <BrowserRouter>
        <Routes>
          <Route path="/login" element={<Login />} />
          <Route path="/invite" element={<Invite />} />
          <Route element={<Gate><Shell /></Gate>}>
            <Route index element={<ComingSoon title="Клиенты" text={`Карточки клиентов и дерево задач. ${soon}`} />} />
            <Route path="tasks" element={<ComingSoon title="Задачи" text={`Все задачи, сроки и этапы. ${soon}`} />} />
            <Route path="messenger" element={<ComingSoon title="Мессенджер" text={`Чаты по всем задачам в одном месте. ${soon}`} />} />
            <Route path="admin/staff" element={<Gate owner><AdminStaff /></Gate>} />
            <Route path="admin/sessions" element={<Gate owner><AdminSessions /></Gate>} />
            <Route path="admin/server" element={<Gate owner><AdminServer /></Gate>} />
          </Route>
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </BrowserRouter>
    </AuthProvider>
  </StrictMode>,
);
