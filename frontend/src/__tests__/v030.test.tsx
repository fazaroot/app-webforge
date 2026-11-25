import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import App from '../App';
import { Dashboard } from '../pages/Dashboard';
import { Servers } from '../pages/Servers';
import { Databases } from '../pages/Databases';
import { api } from '../lib/api';
import type { Project, ServerInstance } from '../lib/types';
import { useAppStore } from '../store';

vi.mock('../lib/api', () => ({
  api: {
    health: vi.fn(),
    systemInfo: vi.fn(),
    projects: vi.fn(async () => []),
    servers: vi.fn(async () => []),
    runtimes: vi.fn(async () => []),
    monitoringCurrent: vi.fn(),
    network: vi.fn(),
    serverLogs: vi.fn(async () => []),
    createServer: vi.fn(),
    startServer: vi.fn(),
    stopServer: vi.fn(),
    restartServer: vi.fn(),
    reloadServer: vi.fn(),
    deleteServer: vi.fn(),
    serverConfig: vi.fn(),
    putServerConfig: vi.fn(),
    validateServerConfig: vi.fn(),
    pythonInfo: vi.fn(),
    projectPython: vi.fn(),
    createPythonVenv: vi.fn(),
    installPythonDeps: vi.fn(),
    projectNode: vi.fn(),
    installNodeDeps: vi.fn(),
    phpInfo: vi.fn(),
    phpSettings: vi.fn(),
    putPhpSettings: vi.fn(),
    phpCli: vi.fn(),
    databases: vi.fn(async () => []),
    createDatabase: vi.fn(),
    testDatabaseConnection: vi.fn(),
    startDatabase: vi.fn(),
    stopDatabase: vi.fn(),
    deleteDatabase: vi.fn(),
    dbTables: vi.fn(async () => []),
    dbQuery: vi.fn(),
    dbBackup: vi.fn(),
    dbRestore: vi.fn(),
  },
  isNativeWebForge: vi.fn(() => false),
  pickDirectory: vi.fn(),
  openBatterySettings: vi.fn(async () => undefined),
  getPowerStatus: vi.fn(),
  setKeepCpuAwake: vi.fn(),
  requestIgnoreBatteryOptimizations: vi.fn(),
  resetApiConfigCache: vi.fn(),
  resolveApiConfig: vi.fn(async () => ({ base: 'http://127.0.0.1:8765', token: '' })),
}));

const mockApi = vi.mocked(api);

const laravel: Project = {
  id: 'p-laravel', name: 'Situs Laravel', slug: 'situs-laravel', path: '/ws/situs-laravel',
  document_root: 'public', created_at: '2026-11-25T00:00:00Z', updated_at: '2026-11-25T00:00:00Z',
  framework: {
    framework: 'Laravel', language: 'PHP', version: '11.0', recommended_runtime: 'php',
    recommended_document_root: 'public', readiness: 'needs_runtime', notes: [], detected_files: ['artisan'],
  },
};
const flask: Project = {
  id: 'p-flask', name: 'API Flask', slug: 'api-flask', path: '/ws/api-flask',
  document_root: '.', created_at: '2026-11-25T00:00:00Z', updated_at: '2026-11-25T00:00:00Z',
  framework: {
    framework: 'Flask', language: 'Python', version: null, recommended_runtime: 'python',
    recommended_document_root: '.', readiness: 'needs_dependencies', notes: [], detected_files: ['requirements.txt', 'app.py'],
  },
};
const staticProject: Project = {
  id: 'p-static', name: 'Situs Statis', slug: 'situs-statis', path: '/ws/situs-statis',
  document_root: '.', created_at: '2026-11-25T00:00:00Z', updated_at: '2026-11-25T00:00:00Z',
  framework: {
    framework: 'Static HTML', language: 'Static', version: null, recommended_runtime: 'static',
    recommended_document_root: '.', readiness: 'ready', notes: [], detected_files: ['index.html'],
  },
};
const caddyServer: ServerInstance = {
  id: 'srv-caddy', project_id: 'p-static', project_name: 'Situs Statis', engine: 'caddy', port: 8080,
  bind_lan: false, document_root: '.', status: 'running', pid: 123, started_at: '2026-11-25T00:00:00Z',
  uptime_seconds: 60, url_local: 'http://127.0.0.1:8080', url_lan: null, error_message: null,
  config_text: 'localhost:8080\n', proxy_port: null, command: null,
};

function renderWithQuery(ui: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

function stubMatchMedia(dark: boolean) {
  vi.stubGlobal('matchMedia', vi.fn().mockImplementation((query: string) => ({
    matches: dark,
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })));
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  document.documentElement.classList.remove('dark');
  stubMatchMedia(false);
  useAppStore.setState({ section: 'dashboard', activeProjectId: null });
  useAppStore.getState().setTheme('light');
  mockApi.health.mockResolvedValue({ status: 'ok', version: '0.3.0', uptime_seconds: 10 });
  mockApi.systemInfo.mockResolvedValue({
    version: '0.3.0', os: 'android', arch: 'arm64', data_dir: '/data', workspace_dir: '/data/ws',
    management_port: 8765, started_at: '2026-11-25T00:00:00Z', runtimes: [],
  });
  mockApi.monitoringCurrent.mockResolvedValue({
    cpu_percent: 1, mem_total_bytes: 100, mem_used_bytes: 10, storage_total_bytes: 100, storage_used_bytes: 10,
    processes: [], ports: [],
  });
  mockApi.network.mockResolvedValue({ lan_ip: null, interfaces: [], management_url: 'http://127.0.0.1:8765' });
  mockApi.projects.mockResolvedValue([laravel, flask, staticProject]);
  mockApi.servers.mockResolvedValue([]);
  mockApi.runtimes.mockResolvedValue([]);
  mockApi.phpInfo.mockResolvedValue({
    available: false, version: null, extensions: [], binary_path: null, error: 'Binary PHP tidak ditemukan',
  });
  mockApi.phpSettings.mockResolvedValue({
    memory_limit: '256M', upload_max_filesize: '32M', post_max_size: '32M',
    max_execution_time: 60, date_timezone: 'Asia/Jakarta', display_errors: true,
  });
  mockApi.databases.mockResolvedValue([]);
});

describe('skeleton loading', () => {
  it('Dashboard menampilkan skeleton saat health masih memuat', () => {
    mockApi.health.mockImplementation(() => new Promise(() => undefined));
    renderWithQuery(<Dashboard />);
    expect(screen.getAllByTestId('skeleton').length).toBeGreaterThan(0);
    expect(screen.getAllByRole('status', { name: 'Memuat' }).length).toBeGreaterThan(0);
  });

  it('Servers menampilkan skeleton saat daftar server masih memuat', () => {
    mockApi.servers.mockImplementation(() => new Promise(() => undefined));
    renderWithQuery(<Servers />);
    expect(screen.getAllByTestId('skeleton').length).toBeGreaterThan(0);
  });
});

describe('tombol tema header', () => {
  it('mengganti class dark pada dokumen dan tersimpan di store', async () => {
    renderWithQuery(<App />);
    fireEvent.click(await screen.findByRole('button', { name: 'Ganti ke tema gelap' }));
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(useAppStore.getState().theme).toBe('dark');
    fireEvent.click(screen.getByRole('button', { name: 'Ganti ke tema terang' }));
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    expect(useAppStore.getState().theme).toBe('light');
  });

  it('dari tema system dengan preferensi gelap, tombol menetapkan tema terang secara eksplisit', async () => {
    stubMatchMedia(true);
    useAppStore.getState().setTheme('system');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    renderWithQuery(<App />);
    fireEvent.click(await screen.findByRole('button', { name: 'Ganti ke tema terang' }));
    expect(useAppStore.getState().theme).toBe('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });
});

describe('Servers engine dinamis', () => {
  beforeEach(() => {
    mockApi.runtimes.mockResolvedValue([
      { id: 'static', name: 'WebForge Static Server', available: true, version: '0.3.0', reason: null },
      { id: 'php', name: 'PHP', available: false, version: null, reason: 'Binary PHP belum dibundel' },
      { id: 'caddy', name: 'Caddy', available: true, version: '2.8.0', reason: null },
      { id: 'nginx', name: 'Nginx', available: false, version: null, reason: 'Binary Nginx belum dibundel' },
      { id: 'python', name: 'Python', available: true, version: '3.12.0', reason: null },
    ]);
  });

  async function selectProject(id: string) {
    await screen.findAllByRole('option', { name: 'Situs Laravel' });
    fireEvent.change(screen.getAllByRole('combobox')[0], { target: { value: id } });
  }

  it('menyembunyikan engine yang runtime-nya belum tersedia dan menjelaskan alasannya', async () => {
    renderWithQuery(<Servers />);
    await selectProject('p-laravel');
    expect(await screen.findByRole('option', { name: 'Caddy' })).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'PHP built-in server' })).toBeNull();
    expect(screen.queryByRole('option', { name: 'Nginx' })).toBeNull();
    expect(screen.queryByRole('option', { name: 'Python' })).toBeNull();
    expect(screen.getByText(/tidak ditampilkan karena runtime-nya belum tersedia/)).toHaveTextContent('PHP built-in server (Binary PHP belum dibundel)');
  });

  it('proyek Python mendapat engine Python dan panel Python', async () => {
    mockApi.pythonInfo.mockResolvedValue({
      available: true, version: '3.12.0', executable: '/data/python', error: null, pip_available: true,
    });
    mockApi.projectPython.mockResolvedValue({
      venv_exists: false, venv_path: null, requirements_exists: true,
      packages: [{ name: 'flask', version: '3.0.0' }], entry_kind: 'script', entry: 'app.py',
    });
    renderWithQuery(<Servers />);
    await selectProject('p-flask');
    expect(await screen.findByRole('option', { name: 'Python' })).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'PHP built-in server' })).toBeNull();
    expect(await screen.findByText(/Python: API Flask/)).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Buat venv' })).toBeInTheDocument();
    expect(screen.getByText(/flask 3.0.0/)).toBeInTheDocument();
  });

  it('tombol "Pakai engine ini" mengisi pilihan engine dan memunculkan input proxy', async () => {
    renderWithQuery(<Servers />);
    await selectProject('p-laravel');
    const pakai = await screen.findAllByRole('button', { name: 'Pakai engine ini' });
    // Runtime tersedia & relevan untuk proyek Laravel: static dan caddy (urutan daftar runtime).
    expect(pakai).toHaveLength(2);
    fireEvent.click(pakai[1]);
    await waitFor(() => expect(screen.getAllByRole('combobox')[1]).toHaveValue('caddy'));
    expect(screen.getByText(/Port backend reverse proxy \(opsional\)/)).toBeInTheDocument();
  });
});

describe('panel konfigurasi server', () => {
  it('memuat config_text, memanggil validasi, dan menampilkan output backend apa adanya', async () => {
    mockApi.servers.mockResolvedValue([caddyServer]);
    mockApi.runtimes.mockResolvedValue([
      { id: 'caddy', name: 'Caddy', available: true, version: '2.8.0', reason: null },
    ]);
    mockApi.serverConfig.mockResolvedValue({
      engine: 'caddy', config_text: 'localhost:8080\n', generated_config: 'localhost:8080 { file_server }',
      config_path: '/ws/situs-statis/Caddyfile', editable: true,
    });
    mockApi.validateServerConfig.mockResolvedValue({ valid: true, output: 'Valid configuration' });
    mockApi.reloadServer.mockResolvedValue(caddyServer);
    renderWithQuery(<Servers />);
    fireEvent.click(await screen.findByRole('button', { name: 'Konfigurasi' }));
    const textarea = await screen.findByLabelText('Konfigurasi server');
    expect(textarea).toHaveValue('localhost:8080\n');
    fireEvent.click(screen.getByRole('button', { name: 'Validasi' }));
    await waitFor(() => expect(mockApi.validateServerConfig).toHaveBeenCalledWith('srv-caddy', 'localhost:8080\n'));
    expect(await screen.findByText('Konfigurasi valid.')).toBeInTheDocument();
    expect(screen.getByText('Valid configuration')).toBeInTheDocument();
    const reloadButtons = screen.getAllByRole('button', { name: /Reload/ });
    expect(reloadButtons.length).toBeGreaterThan(0);
    fireEvent.click(reloadButtons[0]);
    await waitFor(() => expect(mockApi.reloadServer).toHaveBeenCalledWith('srv-caddy'));
  });

  it('server static tidak menampilkan tombol Konfigurasi maupun Reload', async () => {
    mockApi.servers.mockResolvedValue([{ ...caddyServer, id: 'srv-static', engine: 'static', config_text: null }]);
    renderWithQuery(<Servers />);
    expect(await screen.findByRole('button', { name: /Stop/ })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Konfigurasi' })).toBeNull();
    expect(screen.queryByRole('button', { name: /Reload/ })).toBeNull();
  });
});

describe('Databases v0.3.0', () => {
  it('menyembunyikan PostgreSQL/MariaDB lokal bila runtime belum tersedia, eksternal tetap tampil dan Tes Koneksi nyata', async () => {
    mockApi.runtimes.mockResolvedValue([
      { id: 'postgresql', name: 'PostgreSQL', available: false, version: null, reason: 'Belum dibundel' },
      { id: 'mariadb', name: 'MariaDB', available: false, version: null, reason: 'Belum dibundel' },
    ]);
    mockApi.testDatabaseConnection.mockResolvedValue({ ok: true, message: 'Koneksi berhasil ke db.contoh.com' });
    renderWithQuery(<Databases />);
    expect(await screen.findByRole('option', { name: 'MySQL eksternal' })).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'PostgreSQL lokal' })).toBeNull();
    expect(screen.queryByRole('option', { name: 'MariaDB lokal' })).toBeNull();
    fireEvent.change(screen.getAllByRole('combobox')[0], { target: { value: 'external-mysql' } });
    fireEvent.change(screen.getByPlaceholderText(/Host/), { target: { value: 'db.contoh.com' } });
    fireEvent.change(screen.getByPlaceholderText('Nama database baru'), { target: { value: 'db-jauh' } });
    fireEvent.click(screen.getByRole('button', { name: 'Tes Koneksi' }));
    await waitFor(() => expect(mockApi.testDatabaseConnection).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'external-mysql', host: 'db.contoh.com', port: 3306 }),
    ));
    expect(await screen.findByText('Koneksi berhasil ke db.contoh.com')).toBeInTheDocument();
  });

  it('database PostgreSQL lokal menampilkan tombol Start saat berhenti', async () => {
    mockApi.databases.mockResolvedValue([{
      id: 'db-pg', name: 'pg-lokal', path: '/data/pg', size_bytes: 0, created_at: '2026-11-25T00:00:00Z',
      kind: 'postgresql', status: 'stopped', host: null, port: 5433, username: null, database_name: 'pg-lokal', error_message: null,
    }]);
    mockApi.startDatabase.mockResolvedValue({
      id: 'db-pg', name: 'pg-lokal', path: '/data/pg', size_bytes: 0, created_at: '2026-11-25T00:00:00Z',
      kind: 'postgresql', status: 'running', host: null, port: 5433, username: null, database_name: 'pg-lokal', error_message: null,
    });
    renderWithQuery(<Databases />);
    fireEvent.click(await screen.findByRole('button', { name: /Start/ }));
    await waitFor(() => expect(mockApi.startDatabase).toHaveBeenCalledWith('db-pg'));
  });
});
