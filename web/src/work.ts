// Рабочая часть: клиенты, задачи, чаты. Типы повторяют ответы сервера.
import { ApiError } from './api';

export type Priority = 1 | 2 | 3;
export type Status = 'new' | 'in_progress' | 'completed';
export type DeadlineState = 'ok' | 'due_soon' | 'overdue' | 'done';
export type TaskAccess = 'none' | 'path_only' | 'participant' | 'manage';
export type ClientAccess = 'none' | 'view' | 'manage';

export interface PersonRef { id: string; name: string }
export interface Person { id: string; display_name: string; role: string }

export interface TaskSummary {
  id: string;
  client_id: string;
  client_name: string;
  parent_id: string | null;
  title: string;
  access: TaskAccess;
  // Ниже — только если access не path_only.
  status?: Status;
  priority?: Priority;
  deadline_at?: number;
  deadline_state?: DeadlineState;
  completed_at?: number | null;
  archived_at?: number | null;
  created_at?: number;
  assignees?: PersonRef[];
  progress?: { done: number; total: number };
  unread?: number;
  children?: number;
  last_message_at?: number | null;
}

export interface Stage {
  id: string;
  title: string;
  completed_at: number | null;
  completed_by_name: string | null;
  sort_order: number;
}

export interface TaskCard extends TaskSummary {
  description: string;
  created_by: PersonRef;
  stages: Stage[];
  path: { id: string; title: string; readable: boolean }[];
  subtasks: TaskSummary[];
  parent_deadline_at: number | null;
  can_trash: boolean;
}

export interface Tag { name: string; color: string }

export interface Client {
  id: string;
  kind: 'person' | 'company';
  name: string;
  phone: string | null;
  email: string | null;
  description: string;
  group_id: string;
  group_name: string;
  manager_id: string | null;
  manager_name: string | null;
  created_by: string;
  created_at: number;
  deleted_at: number | null;
  tags: Tag[];
  counts: { open: number; overdue: number; done_30d: number; unread: number };
  access: ClientAccess;
}

export interface ClientCardT extends Client {
  can_trash: boolean;
  tasks: TaskSummary[];
}

export interface ClientInput {
  kind: 'person' | 'company';
  name: string;
  phone: string;
  email: string;
  description: string;
  group_id: string;
  manager_id: string;
  tags: string[];
}

export interface Group { id: string; name: string; members: string[]; clients: number }

export interface Attachment { sha256: string; file_name: string; size_bytes: number; mime: string }

export interface Message {
  id: string;
  sender_id: string;
  sender_name: string;
  kind: 'user' | 'system';
  content: string;
  created_at: number;
  edited_at: number | null;
  reply_to: { id: string; sender_name: string; excerpt: string } | null;
  attachments: Attachment[];
  can_edit: boolean;
}

export interface Thread extends TaskSummary {
  last_message: { sender_name: string; excerpt: string; created_at: number; kind: string } | null;
}

export interface Notification {
  id: string;
  task_id: string | null;
  kind: string;
  payload: Record<string, unknown>;
  created_at: number;
  read_at: number | null;
}

export interface OrgSettings {
  timezone: string;
  work_days: number[];
  work_start: string;
  work_end: string;
  holidays: string[];
  warn_hours: number;
  archive_after_days: number;
}

export interface Hit {
  kind: 'client' | 'task' | 'stage' | 'message';
  id: string;
  task_id: string | null;
  client_id: string | null;
  title: string;
  snippet: string;
}

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
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

const q = (params: Record<string, string | number | boolean | undefined | null>) => {
  const s = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== null && v !== '' && v !== false) s.set(k, String(v));
  const t = s.toString();
  return t ? `?${t}` : '';
};

/** Загрузка файла потоком: имя — в заголовке, чтобы не упираться в лимит JSON. */
async function upload<T>(path: string, file: File): Promise<T> {
  const res = await fetch(path, {
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'x-file-name': encodeURIComponent(file.name) },
    body: file,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(res.status, data.error ?? `Ошибка ${res.status}`);
  return data as T;
}

export const work = {
  people: () => req<Person[]>('GET', '/api/people'),
  groups: () => req<Group[]>('GET', '/api/groups'),
  tags: () => req<Tag[]>('GET', '/api/tags'),

  clients: (p: { q?: string; tag?: string; trash?: boolean } = {}) => req<Client[]>('GET', `/api/clients${q(p)}`),
  client: (id: string) => req<ClientCardT>('GET', `/api/clients/${id}`),
  createClient: (c: ClientInput) => req<{ id: string }>('POST', '/api/clients', c),
  updateClient: (id: string, c: ClientInput) => req<void>('PUT', `/api/clients/${id}`, c),
  trashClient: (id: string) => req<void>('DELETE', `/api/clients/${id}`),
  restoreClient: (id: string) => req<void>('POST', `/api/clients/${id}/restore`),
  purgeClient: (id: string) => req<void>('DELETE', `/api/clients/${id}/purge`),

  tasks: (p: { view?: 'mine'; status?: Status; client_id?: string; archived?: boolean; priority?: number } = {}) =>
    req<TaskSummary[]>('GET', `/api/tasks${q(p)}`),
  task: (id: string) => req<TaskCard>('GET', `/api/tasks/${id}`),
  createTask: (t: {
    client_id?: string; parent_id?: string; title: string; description: string; priority: Priority;
    deadline_at: number; assignees: string[]; stages: string[];
  }) => req<{ id: string }>('POST', '/api/tasks', t),
  updateTask: (id: string, p: Partial<{ title: string; description: string; priority: Priority; deadline_at: number; assignees: string[] }>) =>
    req<void>('PATCH', `/api/tasks/${id}`, p),
  setStatus: (id: string, status: Status) => req<{ open_stages: number }>('POST', `/api/tasks/${id}/status`, { status }),
  trashTask: (id: string) => req<void>('DELETE', `/api/tasks/${id}`),
  restoreTask: (id: string) => req<void>('POST', `/api/tasks/${id}/restore`),
  trashedTasks: () => req<{ id: string; title: string; client_name: string; deleted_at: number }[]>('GET', '/api/tasks/trash'),
  unarchive: (id: string) => req<void>('POST', `/api/tasks/${id}/unarchive`),
  addStage: (task: string, title: string) => req<{ id: string }>('POST', `/api/tasks/${task}/stages`, { title }),
  updateStage: (id: string, p: { done?: boolean; title?: string }) => req<void>('PATCH', `/api/stages/${id}`, p),
  deleteStage: (id: string) => req<void>('DELETE', `/api/stages/${id}`),

  messages: (task: string) => req<{ messages: Message[]; last_read: string | null }>('GET', `/api/tasks/${task}/messages`),
  post: (task: string, content: string, reply_to: string | null, attachments: { sha256: string; file_name: string }[]) =>
    req<{ id: string }>('POST', `/api/tasks/${task}/messages`, { content, reply_to, attachments }),
  edit: (id: string, content: string) => req<void>('PATCH', `/api/messages/${id}`, { content }),
  read: (task: string, message_id?: string) => req<void>('POST', `/api/tasks/${task}/read`, { message_id }),
  readAll: () => req<void>('POST', '/api/unread/read-all'),
  unread: () => req<{ total: number; tasks: Record<string, number> }>('GET', '/api/unread'),
  threads: (unread_only = false) => req<Thread[]>('GET', `/api/messenger${q({ unread_only })}`),
  upload: (file: File) => upload<Attachment>('/api/attachments', file),

  search: (text: string, archive = false) => req<Hit[]>('GET', `/api/search${q({ q: text, archive })}`),
  notifications: () => req<{ unread: number; items: Notification[]; quiet: boolean }>('GET', '/api/notifications'),
  readNotifications: () => req<void>('POST', '/api/notifications/read'),
  prefs: () => req<{ notify_always: boolean }>('GET', '/api/me/prefs'),
  setPrefs: (notify_always: boolean) => req<{ notify_always: boolean }>('PUT', '/api/me/prefs', { notify_always }),

  org: () => req<OrgSettings>('GET', '/api/org'),
  saveOrg: (o: OrgSettings) => req<OrgSettings>('PUT', '/api/admin/org', o),
  checkTime: (at: number) => req<{ working: boolean; reason: string | null }>('GET', `/api/org/check${q({ at })}`),

  createGroup: (name: string, members: string[]) => req<{ id: string }>('POST', '/api/admin/groups', { name, members }),
  updateGroup: (id: string, name: string, members: string[]) => req<void>('PUT', `/api/admin/groups/${id}`, { name, members }),
  deleteGroup: (id: string) => req<void>('DELETE', `/api/admin/groups/${id}`),

  importParse: (file: File) =>
    upload<{ file_name: string; headers: string[]; rows: string[][]; mapping: Record<string, number> }>('/api/import/clients/parse', file),
  importCheck: (rows: ImportRecord[], group_id: string) =>
    req<{ errors: string[]; duplicate: { id: string; name: string; by: string } | null }[]>('POST', '/api/import/clients/check', { rows, group_id }),
  importCommit: (file_name: string, group_id: string, rows: (ImportRecord & { action: 'create' | 'update' | 'skip'; target_id?: string })[]) =>
    req<{ batch_id: string; created: number; updated: number; skipped: number }>('POST', '/api/import/clients/commit', { file_name, group_id, rows }),
  importBatches: () =>
    req<{ id: string; file_name: string; created_at: number; stats: string; undone_at: number | null; can_undo: boolean }[]>('GET', '/api/import/batches'),
  importUndo: (id: string) => req<{ removed: number; kept: number }>('POST', `/api/import/batches/${id}/undo`),

  connectionKey: (user_id: string | null, pin: string | null) =>
    req<{ key: string; with_pin: boolean; invite_expires_at: number | null; addrs: string[] }>('POST', '/api/admin/connection-key', { user_id, pin }),
  vpn: () => req<VpnInfo>('GET', '/api/admin/vpn'),
  vpnSetup: (endpoint: string, port: number) => req<void>('PUT', '/api/admin/vpn', { endpoint, port }),
  vpnAdd: (name: string, user_id: string | null) =>
    req<{ id: string; address: string; conf: string; file_name: string; qr_svg: string }>('POST', '/api/admin/vpn/peers', { name, user_id }),
  vpnRemove: (id: string) => req<void>('DELETE', `/api/admin/vpn/peers/${id}`),
  restore: (name: string | null, dir: string | null) =>
    req<{ name: string; at: number; attachments_copied: number }>('POST', '/api/admin/restore', { name, dir }),
  moveOut: (dir: string) => req<{ file: string }>('POST', '/api/admin/move-out', { dir }),
};

export interface ImportRecord {
  name: string;
  kind?: string;
  phone?: string;
  email?: string;
  description?: string;
  tags?: string;
  manager?: string;
}

export interface VpnInfo {
  configured: boolean;
  endpoint: string | null;
  port: number;
  tunnel_up: boolean;
  conf_path: string;
  peers: { id: string; name: string; user_name: string | null; address: string; created_at: number; last_seen_at: number | null }[];
}

// ---------- Отображение ----------

export const statusName: Record<Status, string> = { new: 'Новая', in_progress: 'В работе', completed: 'Завершена' };

/** «Осталось 3 дня», «Осталось 5 ч», «Просрочено на 2 ч». */
export function deadlineText(at: number, state: DeadlineState, now = Date.now()): string {
  if (state === 'done') return 'Завершена';
  const diff = at - now;
  const abs = Math.abs(diff);
  const h = Math.round(abs / 3_600_000);
  const d = Math.round(abs / 86_400_000);
  const amount = abs < 3_600_000 ? `${Math.max(1, Math.round(abs / 60_000))} мин` : h < 48 ? `${h} ч` : `${d} ${plural(d, 'день', 'дня', 'дней')}`;
  return diff < 0 ? `Просрочено на ${amount}` : `Осталось ${amount}`;
}

export function plural(n: number, one: string, few: string, many: string) {
  const m10 = n % 10, m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}

export function dateTime(ms: number) {
  return new Date(ms).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

/** Значение для `<input type="datetime-local">` в местном времени. */
export function toLocalInput(ms: number) {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function fromLocalInput(s: string) {
  return new Date(s).getTime();
}

/** Дерево из плоского списка: дети по родителю. */
export function childrenOf(tasks: TaskSummary[]) {
  const ids = new Set(tasks.map((t) => t.id));
  const by = new Map<string | null, TaskSummary[]>();
  for (const t of tasks) {
    const parent = t.parent_id && ids.has(t.parent_id) ? t.parent_id : null;
    by.set(parent, [...(by.get(parent) ?? []), t]);
  }
  return by;
}
