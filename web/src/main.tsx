import { StrictMode, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, Navigate, Route, Routes } from 'react-router';
import { Loader2 } from 'lucide-react';
import './index.css';
import { AuthProvider, useAuth } from './auth';
import { Shell } from './components/Shell';
import { LiveProvider } from './live';
import { ClientCard, Clients } from './pages/Clients';
import { Archive, TaskList, TaskPage } from './pages/Tasks';
import { Messenger } from './pages/Messenger';
import { ImportClients } from './pages/ImportClients';
import { Trash } from './pages/Trash';
import { AdminGroups, AdminOrg, AdminVpn } from './pages/AdminWork';
import { Login } from './pages/Login';
import { Invite } from './pages/Invite';
import { Forgot } from './pages/Forgot';
import { AdminStaff } from './pages/AdminStaff';
import { AdminSessions } from './pages/AdminSessions';
import { AdminServer } from './pages/AdminServer';
import { AdminStorage } from './pages/AdminStorage';
import { AdminHelp, UserHelp } from './pages/Help';

function Gate({ children, owner = false }: { children: ReactNode; owner?: boolean }) {
  const { me, loading } = useAuth();
  if (loading) return <div className="grid h-dvh place-items-center"><Loader2 className="size-7 animate-spin text-brand-500" /></div>;
  if (!me) return <Navigate to="/login" replace />;
  // Админ-панель — только владельцу; остальным сервер всё равно ответит 403.
  if (owner && me.role !== 'host') return <Navigate to="/" replace />;
  return <>{children}</>;
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AuthProvider>
      <BrowserRouter>
        <Routes>
          <Route path="/login" element={<Login />} />
          <Route path="/invite" element={<Invite />} />
          <Route path="/forgot" element={<Forgot />} />
          <Route element={<Gate><LiveProvider><Shell /></LiveProvider></Gate>}>
            <Route index element={<TaskList mode="mine" />} />
            <Route path="tasks" element={<TaskList mode="all" />} />
            <Route path="archive" element={<Archive />} />
            <Route path="tasks/:id" element={<TaskPage />} />
            <Route path="clients" element={<Clients />} />
            <Route path="clients/import" element={<ImportClients />} />
            <Route path="clients/:id" element={<ClientCard />} />
            <Route path="messenger" element={<Messenger />} />
            <Route path="trash" element={<Trash />} />
            <Route path="admin/groups" element={<Gate owner><AdminGroups /></Gate>} />
            <Route path="admin/org" element={<Gate owner><AdminOrg /></Gate>} />
            <Route path="admin/vpn" element={<Gate owner><AdminVpn /></Gate>} />
            <Route path="admin/staff" element={<Gate owner><AdminStaff /></Gate>} />
            <Route path="admin/sessions" element={<Gate owner><AdminSessions /></Gate>} />
            <Route path="admin/storage" element={<Gate owner><AdminStorage /></Gate>} />
            <Route path="admin/server" element={<Gate owner><AdminServer /></Gate>} />
            <Route path="admin/help" element={<Gate owner><AdminHelp /></Gate>} />
            <Route path="help" element={<UserHelp />} />
          </Route>
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </BrowserRouter>
    </AuthProvider>
  </StrictMode>,
);
