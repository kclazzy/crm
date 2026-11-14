// Клиент API сервера CRM. Сессия — в HttpOnly-cookie: токен в JS не хранится.

export type Role = 'host' | 'manager' | 'employee';

export interface Me {
  id: string;
  username: string;
  display_name: string;
  role: Role;
}

export interface StaffMember {
  id: string;
  username: string;
  display_name: string;
  role: Role;
  state: 'active' | 'invited' | 'disabled';
  created_at: number;
}

export interface Session {
  id: string;
  username: string;
  display_name: string;
  device_name: string;
  ip: string;
  created_at: number;
  last_seen_at: number;
}

export interface Invite {
  invite_token: string;
  invite_expires_at: number;
}

export interface Health {
  status: string;
  version: string;
  ca_fingerprint: string | null;
}

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 204) return undefined as T;
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(res.status, data.error ?? `Ошибка ${res.status}`);
  return data as T;
}

/** Постоянный идентификатор этого браузера — сервер привязывает к нему сессию. */
function device() {
  let id = '';
  try {
    id = localStorage.getItem('crm_device') ?? '';
    if (!id) {
      id = crypto.randomUUID();
      localStorage.setItem('crm_device', id);
    }
  } catch {
    id = crypto.randomUUID();
  }
  const ua = navigator.userAgent;
  const browser = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Браузер';
  const os = /Windows/.test(ua) ? 'Windows' : /Android/.test(ua) ? 'Android' : /iPhone|iPad/.test(ua) ? 'iOS' : /Mac OS/.test(ua) ? 'macOS' : /Linux/.test(ua) ? 'Linux' : '';
  return { device_id: id, device_name: os ? `${browser} · ${os}` : browser };
}

export const api = {
  me: () => request<Me>('GET', '/api/me'),
  login: (username: string, password: string) =>
    request<{ user: Me }>('POST', '/api/auth/login', { username, password, ...device() }),
  acceptInvite: (token: string, password: string) =>
    request<{ user: Me }>('POST', '/api/invites/accept', { token, password, ...device() }),
  logout: () => request<void>('POST', '/api/auth/logout'),
  health: () => request<Health>('GET', '/api/health'),
  staff: () => request<StaffMember[]>('GET', '/api/admin/users'),
  addStaff: (username: string, display_name: string, role: Role) =>
    request<{ user: Me } & Invite>('POST', '/api/admin/users', { username, display_name, role }),
  resetAccess: (id: string) => request<Invite>('POST', `/api/admin/users/${id}/reset`),
  sessions: () => request<Session[]>('GET', '/api/admin/sessions'),
  revokeSession: (id: string) => request<void>('DELETE', `/api/admin/sessions/${id}`),
};

export const roleName: Record<Role, string> = {
  host: 'Владелец',
  manager: 'Руководитель',
  employee: 'Сотрудник',
};

export function timeAgo(ms: number): string {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return 'только что';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} мин назад`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} ч назад`;
  return new Date(ms).toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' });
}
