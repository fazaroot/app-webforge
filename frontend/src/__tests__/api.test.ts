import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  api, ApiError, getPowerStatus, isNativeWebForge, openBatterySettings, pickDirectory,
  requestIgnoreBatteryOptimizations, resetApiConfigCache, setKeepCpuAwake,
} from '../lib/api';

function jsonResponse(body: unknown, ok = true, status = 200): Response {
  return {
    ok,
    status,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

describe('api client', () => {
  beforeEach(() => {
    localStorage.clear();
    resetApiConfigCache();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('menyertakan header X-API-Key dari localStorage', async () => {
    localStorage.setItem('webforge_token', 'token-uji');
    localStorage.setItem('webforge_api_base', 'http://127.0.0.1:9999');
    resetApiConfigCache();
    const fetchMock = vi.fn(async () => jsonResponse({ status: 'ok', version: '0.1.0', uptime_seconds: 1 }));
    vi.stubGlobal('fetch', fetchMock);
    await api.health();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('http://127.0.0.1:9999/api/health');
    expect((init.headers as Record<string, string>)['X-API-Key']).toBe('token-uji');
  });

  it('memakai base default 127.0.0.1:8765 bila tidak ada konfigurasi', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ status: 'ok', version: '0.1.0', uptime_seconds: 1 }));
    vi.stubGlobal('fetch', fetchMock);
    await api.health();
    const [url] = fetchMock.mock.calls[0] as unknown as [string];
    expect(url).toBe('http://127.0.0.1:8765/api/health');
  });

  it('mengirim body JSON pada POST proyek', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ id: 'x' }, true, 201));
    vi.stubGlobal('fetch', fetchMock);
    await api.createProject('Situs Saya', 'static');
    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(init.method).toBe('POST');
    expect(JSON.parse(String(init.body))).toEqual({ name: 'Situs Saya', template: 'static' });
  });

  it('melempar ApiError dengan pesan dari field error', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ error: 'unauthorized' }, false, 401));
    vi.stubGlobal('fetch', fetchMock);
    await expect(api.projects()).rejects.toBeInstanceOf(ApiError);
    await expect(api.projects()).rejects.toThrow('unauthorized');
  });

  it('di browser biasa platform native tidak tersedia dan SAF menolak dengan jujur', async () => {
    expect(isNativeWebForge()).toBe(false);
    await expect(pickDirectory()).rejects.toThrow('hanya tersedia di aplikasi Android');
    await expect(openBatterySettings()).rejects.toThrow('hanya tersedia di aplikasi Android');
    await expect(getPowerStatus()).rejects.toThrow('hanya tersedia di aplikasi Android');
    await expect(setKeepCpuAwake(true)).rejects.toThrow('hanya tersedia di aplikasi Android');
    await expect(requestIgnoreBatteryOptimizations()).rejects.toThrow('hanya tersedia di aplikasi Android');
  });

  it('memanggil endpoint PHP sesuai kontrak', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ ok: true }));
    vi.stubGlobal('fetch', fetchMock);
    await api.phpInfo();
    expect((fetchMock.mock.calls[0] as unknown as [string])[0]).toContain('/api/runtimes/php/info');
    const settings = {
      memory_limit: '256M', upload_max_filesize: '32M', post_max_size: '32M',
      max_execution_time: 60, date_timezone: 'Asia/Jakarta', display_errors: true,
    };
    await api.putPhpSettings(settings);
    const [putUrl, putInit] = fetchMock.mock.calls[1] as unknown as [string, RequestInit];
    expect(putUrl).toContain('/api/php/settings');
    expect(putInit.method).toBe('PUT');
    expect(JSON.parse(String(putInit.body))).toEqual(settings);
    await api.phpCli('proj-1', ['-v']);
    const [cliUrl, cliInit] = fetchMock.mock.calls[2] as unknown as [string, RequestInit];
    expect(cliUrl).toContain('/api/projects/proj-1/php-cli');
    expect(JSON.parse(String(cliInit.body))).toEqual({ args: ['-v'] });
  });

  it('memanggil endpoint server v0.3.0 (config, validasi, reload, python, node) sesuai kontrak', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ ok: true }));
    vi.stubGlobal('fetch', fetchMock);
    await api.serverConfig('srv-1');
    expect((fetchMock.mock.calls[0] as unknown as [string])[0]).toContain('/api/servers/srv-1/config');
    await api.putServerConfig('srv-1', 'isi konfigurasi');
    const [putUrl, putInit] = fetchMock.mock.calls[1] as unknown as [string, RequestInit];
    expect(putUrl).toContain('/api/servers/srv-1/config');
    expect(putInit.method).toBe('PUT');
    expect(JSON.parse(String(putInit.body))).toEqual({ config_text: 'isi konfigurasi' });
    await api.validateServerConfig('srv-1', 'isi konfigurasi');
    const [valUrl, valInit] = fetchMock.mock.calls[2] as unknown as [string, RequestInit];
    expect(valUrl).toContain('/api/servers/srv-1/validate-config');
    expect(JSON.parse(String(valInit.body))).toEqual({ config_text: 'isi konfigurasi' });
    await api.reloadServer('srv-1');
    expect((fetchMock.mock.calls[3] as unknown as [string])[0]).toContain('/api/servers/srv-1/reload');
    await api.createServer({ project_id: 'p1', engine: 'caddy', port: 8080, bind_lan: false, proxy_port: 3000 });
    const [, createInit] = fetchMock.mock.calls[4] as unknown as [string, RequestInit];
    expect(JSON.parse(String(createInit.body))).toEqual({ project_id: 'p1', engine: 'caddy', port: 8080, bind_lan: false, proxy_port: 3000 });
    await api.pythonInfo();
    expect((fetchMock.mock.calls[5] as unknown as [string])[0]).toContain('/api/runtimes/python/info');
    await api.createPythonVenv('p1');
    expect((fetchMock.mock.calls[6] as unknown as [string])[0]).toContain('/api/projects/p1/python/venv');
    await api.installPythonDeps('p1');
    expect((fetchMock.mock.calls[7] as unknown as [string])[0]).toContain('/api/projects/p1/python/install');
    await api.projectNode('p1');
    expect((fetchMock.mock.calls[8] as unknown as [string])[0]).toContain('/api/projects/p1/node');
    await api.installNodeDeps('p1');
    expect((fetchMock.mock.calls[9] as unknown as [string])[0]).toContain('/api/projects/p1/node/install');
  });

  it('memanggil endpoint database v0.3.0 (tes koneksi, start/stop, kind eksternal) sesuai kontrak', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ ok: true, message: 'ok' }));
    vi.stubGlobal('fetch', fetchMock);
    const body = { name: 'db-jauh', kind: 'external-mysql' as const, host: 'db.contoh.com', port: 3306, username: 'app', password: 'rahasia', database_name: 'app_db' };
    await api.testDatabaseConnection(body);
    const [testUrl, testInit] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(testUrl).toContain('/api/databases/test-connection');
    expect(JSON.parse(String(testInit.body))).toEqual(body);
    await api.createDatabase(body);
    expect((fetchMock.mock.calls[1] as unknown as [string])[0]).toContain('/api/databases');
    await api.startDatabase('db-1');
    expect((fetchMock.mock.calls[2] as unknown as [string])[0]).toContain('/api/databases/db-1/start');
    await api.stopDatabase('db-1');
    expect((fetchMock.mock.calls[3] as unknown as [string])[0]).toContain('/api/databases/db-1/stop');
  });

  it('menyusun query string untuk daftar file', async () => {
    const fetchMock = vi.fn(async () => jsonResponse([]));
    vi.stubGlobal('fetch', fetchMock);
    await api.filesList('proj-1', 'public/img');
    const [url] = fetchMock.mock.calls[0] as unknown as [string];
    expect(url).toContain('/api/files/list?');
    expect(url).toContain('project_id=proj-1');
    expect(url).toContain('path=public%2Fimg');
  });
});
