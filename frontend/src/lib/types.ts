export interface Health { status: string; version: string; uptime_seconds: number }
export interface RuntimeInfo { id: string; name: string; available: boolean; version: string | null; reason: string | null; source?: string }
export interface SystemInfo { version: string; os: string; arch: string; data_dir: string; workspace_dir: string; management_port: number; started_at: string; runtimes: RuntimeInfo[] }
export interface DiagnosticCheck { name: string; status: 'ok' | 'warn' | 'error'; detail: string }
export interface Diagnostics { generated_at: string; checks: DiagnosticCheck[]; runtimes: RuntimeInfo[]; ports: unknown[]; storage: Record<string, unknown> }
export interface FrameworkInfo { framework: string; language: string; version: string | null; recommended_runtime: string; recommended_document_root: string; readiness: string; notes: string[]; detected_files: string[] }
export interface Project { id: string; name: string; slug: string; path: string; document_root: string; created_at: string; updated_at: string; framework: FrameworkInfo | null }
export interface ServerInstance { id: string; project_id: string; project_name: string; engine: string; port: number; bind_lan: boolean; document_root: string; status: 'stopped' | 'starting' | 'running' | 'stopping' | 'error'; pid: number | null; started_at: string | null; uptime_seconds: number; url_local: string; url_lan: string | null; error_message: string | null; in_process?: boolean }
export interface ServerLog { id: string; level: string; message: string; created_at: string }
export interface FileEntry { name: string; path: string; is_dir: boolean; size: number; modified_at: string }
export interface FileRead { path: string; content: string; size: number }
export interface FileSearchHit { path: string; name: string; is_dir: boolean }
export interface DirSize { path: string; bytes: number; files: number }
export interface DatabaseInfo { id: string; name: string; path: string; size_bytes: number; created_at: string }
export interface TableInfo { name: string; columns: { name: string; type: string; pk: boolean }[] }
export interface QueryResult { columns: string[]; rows: unknown[][]; rows_affected: number }
export interface BackupResult { backup_path: string; created_at: string }
export interface MonitoringCurrent { cpu_percent: number | null; mem_total_bytes: number | null; mem_used_bytes: number | null; storage_total_bytes: number | null; storage_used_bytes: number | null; processes: { pid: number; name: string; server_id: string | null; uptime_seconds: number }[]; ports: { port: number; state: string }[]; unavailable_reason?: string | null }
export interface MonitoringPoint { recorded_at: string; cpu_percent: number | null; mem_used_bytes: number | null; storage_used_bytes: number | null }
export interface TerminalResult { exit_code: number; stdout: string; stderr: string; duration_ms: number }
export interface TerminalHistoryItem { id: string; command: string; exit_code: number; created_at: string }
export interface NetworkInfo { lan_ip: string | null; interfaces: { name: string; ip: string }[]; management_url: string }
export interface LogEntry { id: string; category: string; severity: string; message: string; created_at: string }
export interface GitStatus { available: boolean; reason: string | null; branch: string | null; clean: boolean; entries: string[] }
export interface SecurityStatus { api_token_set: boolean; management_bind: string; lan_servers: number; warnings: string[] }
export type SettingsMap = Record<string, unknown>;
