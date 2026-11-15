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
  reset_requested: boolean;
  two_factor: boolean;
}

export type TwoFactorPolicy = 'off' | 'remote' | 'all';

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

export interface ServerInfo {
  hostname: string;
  version: string;
  addresses: string[];
  ca_fingerprint: string | null;
}

export interface Volume {
  path: string;
  roles: string[];
  total_bytes: number;
  free_bytes: number;
  crm_bytes: number;
  level: 'ok' | 'warning' | 'critical';
  /** Через сколько дней том заполнится при нынешнем темпе (нужна история за несколько дней). */
  days_left: number | null;
}

export interface BackupPolicy {
  enabled: boolean;
  every_hours: number;
  keep_all_hours: number;
  keep_days: number;
}

export interface StorageInfo {
  data_dir: string;
  backup_dirs: string[];
  volumes: Volume[];
  restart_required: boolean;
  policy: BackupPolicy;
  expected_copies: number;
}

export interface BackupFile {
  name: string;
  at: number;
  size: number;
}

export interface BackupStatus {
  at: number;
  ok: boolean;
  file: string | null;
  size: number | null;
  error: string | null;
}

export class ApiError extends Error {
  constructor(public status: number, message: string, public data: { need_code?: boolean; setup_2fa?: boolean } = {}) {
    super(message);
  }
}

/** Страница настройки кода из приложения (второй фактор). */
export const SECURITY_PATH = '/security';
export const NEED_CODE_EVENT = 'crm:need-code';

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 204) return undefined as T;
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    // Организация требует код, а он не настроен: сервер пускает только в профиль.
    if (data.setup_2fa && location.pathname !== SECURITY_PATH) location.assign(SECURITY_PATH);
    // Сессию открыли без кода, а работаем из интернета: подтвердить кодом (окно в Shell).
    if (data.need_code && !path.startsWith('/api/auth/')) window.dispatchEvent(new Event(NEED_CODE_EVENT));
    throw new ApiError(res.status, data.error ?? `Ошибка ${res.status}`, data);
  }
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
  login: (username: string, password: string, code?: string) =>
    request<{ user: Me; must_setup_2fa: boolean }>('POST', '/api/auth/login', { username, password, code, ...device() }),
  acceptInvite: (token: string, password: string) =>
    request<{ user: Me; must_setup_2fa: boolean }>('POST', '/api/invites/accept', { token, password, ...device() }),
  verifyCode: (code: string) => request<void>('POST', '/api/auth/verify', { code }),
  twoFactor: () => request<{ enabled: boolean; backup_left: number; policy: TwoFactorPolicy }>('GET', '/api/me/2fa'),
  twoFactorStart: () => request<{ secret: string; url: string; qr_svg: string }>('POST', '/api/me/2fa/start'),
  twoFactorConfirm: (code: string) => request<{ backup_codes: string[] }>('POST', '/api/me/2fa/confirm', { code }),
  twoFactorDisable: () => request<void>('DELETE', '/api/me/2fa'),
  twoFactorReset: (id: string) => request<void>('POST', `/api/admin/users/${id}/2fa/reset`),
  twoFactorPolicy: () => request<{ policy: TwoFactorPolicy }>('GET', '/api/admin/2fa'),
  setTwoFactorPolicy: (policy: TwoFactorPolicy) => request<{ policy: TwoFactorPolicy }>('PUT', '/api/admin/2fa', { policy }),
  logout: () => request<void>('POST', '/api/auth/logout'),
  forgot: (username: string) => request<{ ok: boolean }>('POST', '/api/auth/forgot', { username }),
  recover: (username: string, code: string, new_password: string) =>
    request<{ ok: boolean }>('POST', '/api/auth/recover', { username, code, new_password }),
  security: () =>
    request<{ blocked: { ip: string; count: number; seconds_ago: number }[]; failed_logins: { ip: string; count: number; last_at: number }[] }>('GET', '/api/admin/security'),
  recoveryStatus: () => request<{ unused: number }>('GET', '/api/admin/recovery-codes'),
  newRecoveryCodes: () => request<{ codes: string[] }>('POST', '/api/admin/recovery-codes'),
  health: () => request<Health>('GET', '/api/health'),
  staff: () => request<StaffMember[]>('GET', '/api/admin/users'),
  addStaff: (username: string, display_name: string, role: Role) =>
    request<{ user: Me } & Invite>('POST', '/api/admin/users', { username, display_name, role }),
  disableUser: (id: string) => request<void>('POST', `/api/admin/users/${id}/disable`),
  enableUser: (id: string) => request<void>('POST', `/api/admin/users/${id}/enable`),
  deleteUser: (id: string, reassignTo?: string) =>
    request<void>('DELETE', `/api/admin/users/${id}${reassignTo ? `?reassign_to=${encodeURIComponent(reassignTo)}` : ''}`),
  resetAccess: (id: string) => request<Invite>('POST', `/api/admin/users/${id}/reset`),
  sessions: () => request<Session[]>('GET', '/api/admin/sessions'),
  revokeSession: (id: string) => request<void>('DELETE', `/api/admin/sessions/${id}`),
  server: () => request<ServerInfo>('GET', '/api/admin/server'),
  storage: () => request<StorageInfo>('GET', '/api/admin/storage'),
  saveBackup: (dirs: string[], policy: BackupPolicy) =>
    request<{ dirs: string[]; warnings: string[] }>('PUT', '/api/admin/backup', { dirs, policy }),
  backupNow: () => request<BackupFile>('POST', '/api/admin/backup/run'),
  backups: () => request<{ last: BackupStatus | null; files: BackupFile[] }>('GET', '/api/admin/backups'),
  moveData: (data_dir: string) => request<{ data_dir: string; restart_required: boolean }>('POST', '/api/admin/storage/move', { data_dir }),
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

export function bytes(n: number): string {
  if (n < 1024) return `${n} Б`;
  const units = ['КБ', 'МБ', 'ГБ', 'ТБ'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toLocaleString('ru-RU', { maximumFractionDigits: v < 10 ? 1 : 0 })} ${units[i]}`;
}
