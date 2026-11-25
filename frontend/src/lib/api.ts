import { Capacitor, registerPlugin } from '@capacitor/core';
import type {
  BackupResult, DatabaseInfo, Diagnostics, DirSize, FileEntry, FileRead, FileSearchHit,
  FrameworkInfo, GitStatus, Health, LogEntry, MonitoringCurrent, MonitoringPoint, NetworkInfo,
  Project, QueryResult, RuntimeInfo, SecurityStatus, ServerInstance, ServerLog, SettingsMap,
  SystemInfo, TableInfo, TerminalHistoryItem, TerminalResult,
} from './types';

export interface PickDirectoryResult {
  path: string;
  name: string;
  fileCount: number;
}

interface WebForgePlugin {
  getApiBase(): Promise<{ value: string } | string>;
  getToken(): Promise<{ value: string } | string>;
  getStatus(): Promise<Record<string, unknown>>;
  startCore(): Promise<Record<string, unknown>>;
  stopCore(): Promise<Record<string, unknown>>;
  pickDirectory(): Promise<PickDirectoryResult>;
  openBatterySettings(): Promise<{ opened: boolean }>;
}

declare global {
  interface Window {
    WebForgeBridge?: { getApiBase?: () => string; getToken?: () => string };
  }
}

const plugin: WebForgePlugin | null = (() => {
  try {
    return registerPlugin<WebForgePlugin>('WebForge');
  } catch {
    return null;
  }
})();

export interface ApiConfig { base: string; token: string }

let cached: ApiConfig | null = null;

function unwrap(v: { value: string } | string | null | undefined): string {
  if (!v) return '';
  return typeof v === 'string' ? v : v.value ?? '';
}

export async function resolveApiConfig(force = false): Promise<ApiConfig> {
  if (cached && !force) return cached;
  let base = '';
  let token = '';
  if (Capacitor.isNativePlatform() && plugin) {
    try {
      base = unwrap(await plugin.getApiBase());
      token = unwrap(await plugin.getToken());
    } catch {
      base = '';
      token = '';
    }
  }
  if (!base && window.WebForgeBridge?.getApiBase) {
    try { base = window.WebForgeBridge.getApiBase() ?? ''; } catch { base = ''; }
  }
  if (!token && window.WebForgeBridge?.getToken) {
    try { token = window.WebForgeBridge.getToken() ?? ''; } catch { token = ''; }
  }
  if (!base) base = localStorage.getItem('webforge_api_base') || 'http://127.0.0.1:8765';
  if (!token) token = localStorage.getItem('webforge_token') || '';
  cached = { base: base.replace(/\/$/, ''), token };
  return cached;
}

export function resetApiConfigCache(): void { cached = null; }

/** True hanya di aplikasi Android dengan plugin WebForge terpasang. */
export function isNativeWebForge(): boolean {
  return Capacitor.isNativePlatform() && plugin !== null;
}

/**
 * SAF: plugin native membuka pemilih folder Android, menyalin pohon
 * dokumen ke workspace app-managed, lalu mengembalikan path nyata dan
 * jumlah file. Pembatalan/kegagalan diteruskan apa adanya sebagai error.
 */
export async function pickDirectory(): Promise<PickDirectoryResult> {
  if (!isNativeWebForge() || !plugin) {
    throw new Error('Pemilih folder SAF hanya tersedia di aplikasi Android.');
  }
  return plugin.pickDirectory();
}

/** Membuka pengaturan pengecualian optimasi baterai Android untuk aplikasi ini. */
export async function openBatterySettings(): Promise<void> {
  if (!isNativeWebForge() || !plugin) {
    throw new Error('Pengaturan baterai hanya tersedia di aplikasi Android.');
  }
  await plugin.openBatterySettings();
}

export class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const { base, token } = await resolveApiConfig();
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (token) headers['X-API-Key'] = token;
  const res = await fetch(`${base}${path}`, { ...init, headers: { ...headers, ...(init?.headers ?? {}) } });
  if (!res.ok) {
    let message = `HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch { /* abaikan */ }
    throw new ApiError(message, res.status);
  }
  return (await res.json()) as T;
}

async function requestText(path: string): Promise<string> {
  const { base, token } = await resolveApiConfig();
  const headers: Record<string, string> = {};
  if (token) headers['X-API-Key'] = token;
  const res = await fetch(`${base}${path}`, { headers });
  if (!res.ok) throw new ApiError(`HTTP ${res.status}`, res.status);
  return res.text();
}

function qs(params: Record<string, string | number | undefined>): string {
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== '') sp.set(k, String(v));
  }
  const s = sp.toString();
  return s ? `?${s}` : '';
}

export const api = {
  health: () => request<Health>('/api/health'),
  systemInfo: () => request<SystemInfo>('/api/system/info'),
  diagnostics: () => request<Diagnostics>('/api/system/diagnostics'),
  runtimes: () => request<RuntimeInfo[]>('/api/runtimes'),

  projects: () => request<Project[]>('/api/projects'),
  createProject: (name: string, template?: string) =>
    request<Project>('/api/projects', { method: 'POST', body: JSON.stringify({ name, template }) }),
  registerProject: (name: string, path: string) =>
    request<Project>('/api/projects/register', { method: 'POST', body: JSON.stringify({ name, path }) }),
  project: (id: string) => request<Project>(`/api/projects/${id}`),
  deleteProject: (id: string, deleteFiles: boolean) =>
    request<{ ok: boolean }>(`/api/projects/${id}`, { method: 'DELETE', body: JSON.stringify({ delete_files: deleteFiles }) }),
  scanProject: (id: string) => request<FrameworkInfo>(`/api/projects/${id}/scan`, { method: 'POST' }),

  servers: () => request<ServerInstance[]>('/api/servers'),
  createServer: (body: { project_id: string; engine: string; port: number; bind_lan: boolean; document_root?: string }) =>
    request<ServerInstance>('/api/servers', { method: 'POST', body: JSON.stringify(body) }),
  deleteServer: (id: string) => request<{ ok: boolean }>(`/api/servers/${id}`, { method: 'DELETE' }),
  startServer: (id: string) => request<ServerInstance>(`/api/servers/${id}/start`, { method: 'POST' }),
  stopServer: (id: string) => request<ServerInstance>(`/api/servers/${id}/stop`, { method: 'POST' }),
  restartServer: (id: string) => request<ServerInstance>(`/api/servers/${id}/restart`, { method: 'POST' }),
  serverLogs: (id: string, limit = 200) => request<ServerLog[]>(`/api/servers/${id}/logs${qs({ limit })}`),

  filesList: (projectId: string, path = '') => request<FileEntry[]>(`/api/files/list${qs({ project_id: projectId, path })}`),
  filesRead: (projectId: string, path: string) => request<FileRead>(`/api/files/read${qs({ project_id: projectId, path })}`),
  filesWrite: (projectId: string, path: string, content: string) =>
    request<{ ok: boolean; size: number }>('/api/files/write', { method: 'POST', body: JSON.stringify({ project_id: projectId, path, content }) }),
  filesMkdir: (projectId: string, path: string) =>
    request<{ ok: boolean }>('/api/files/mkdir', { method: 'POST', body: JSON.stringify({ project_id: projectId, path }) }),
  filesRename: (projectId: string, from: string, to: string) =>
    request<{ ok: boolean }>('/api/files/rename', { method: 'POST', body: JSON.stringify({ project_id: projectId, from, to }) }),
  filesDelete: (projectId: string, path: string) =>
    request<{ ok: boolean }>('/api/files/delete', { method: 'POST', body: JSON.stringify({ project_id: projectId, path }) }),
  filesCopy: (projectId: string, from: string, to: string) =>
    request<{ ok: boolean }>('/api/files/copy', { method: 'POST', body: JSON.stringify({ project_id: projectId, from, to }) }),
  filesMove: (projectId: string, from: string, to: string) =>
    request<{ ok: boolean }>('/api/files/move', { method: 'POST', body: JSON.stringify({ project_id: projectId, from, to }) }),
  filesSearch: (projectId: string, q: string) => request<FileSearchHit[]>(`/api/files/search${qs({ project_id: projectId, q })}`),
  filesDirSize: (projectId: string, path: string) => request<DirSize>(`/api/files/dir-size${qs({ project_id: projectId, path })}`),

  databases: () => request<DatabaseInfo[]>('/api/databases'),
  createDatabase: (name: string) => request<DatabaseInfo>('/api/databases', { method: 'POST', body: JSON.stringify({ name }) }),
  deleteDatabase: (id: string) => request<{ ok: boolean }>(`/api/databases/${id}`, { method: 'DELETE' }),
  dbTables: (id: string) => request<TableInfo[]>(`/api/databases/${id}/tables`),
  dbQuery: (id: string, sql: string) => request<QueryResult>(`/api/databases/${id}/query`, { method: 'POST', body: JSON.stringify({ sql }) }),
  dbBackup: (id: string) => request<BackupResult>(`/api/databases/${id}/backup`, { method: 'POST' }),
  dbRestore: (id: string, backupPath: string) =>
    request<{ ok: boolean }>(`/api/databases/${id}/restore`, { method: 'POST', body: JSON.stringify({ backup_path: backupPath }) }),

  monitoringCurrent: () => request<MonitoringCurrent>('/api/monitoring/current'),
  monitoringHistory: (limit = 60) => request<MonitoringPoint[]>(`/api/monitoring/history${qs({ limit })}`),

  terminalExecute: (command: string, projectId?: string) =>
    request<TerminalResult>('/api/terminal/execute', { method: 'POST', body: JSON.stringify({ command, project_id: projectId ?? null }) }),
  terminalHistory: (limit = 50) => request<TerminalHistoryItem[]>(`/api/terminal/history${qs({ limit })}`),

  network: () => request<NetworkInfo>('/api/network'),
  logs: (severity = '', search = '', limit = 200) => request<LogEntry[]>(`/api/logs${qs({ severity, search, limit })}`),
  logsExport: () => requestText('/api/logs/export'),
  gitStatus: (projectId: string) => request<GitStatus>(`/api/tools/git/status${qs({ project_id: projectId })}`),
  settings: () => request<SettingsMap>('/api/settings'),
  putSettings: (values: SettingsMap) => request<SettingsMap>('/api/settings', { method: 'PUT', body: JSON.stringify(values) }),
  securityStatus: () => request<SecurityStatus>('/api/security/status'),
};

export async function terminalWsUrl(): Promise<string> {
  const { base, token } = await resolveApiConfig();
  const wsBase = base.replace(/^http/, 'ws');
  return `${wsBase}/ws/terminal?token=${encodeURIComponent(token)}`;
}
