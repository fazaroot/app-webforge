import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import App from '../App';
import { Servers } from '../pages/Servers';
import { Projects } from '../pages/Projects';
import { Settings } from '../pages/Settings';
import { Security } from '../pages/Security';
import { PhpManager } from '../components/PhpManager';
import { btnPrimary } from '../components/ui';
import { api, isNativeWebForge } from '../lib/api';
import { useAppStore } from '../store';
import type { PhpRuntimeInfo, Project, RuntimeInfo, ServerInstance } from '../lib/types';

vi.mock('../lib/api', () => ({
  api: {
    health: vi.fn(),
    systemInfo: vi.fn(),
    monitoringCurrent: vi.fn(),
    network: vi.fn(),
    servers: vi.fn(async () => []),
    projects: vi.fn(async () => []),
    project: vi.fn(),
    createProject: vi.fn(),
    registerProject: vi.fn(),
    scanProject: vi.fn(),
    deleteProject: vi.fn(),
    runtimes: vi.fn(async () => []),
    serverLogs: vi.fn(async () => []),
    createServer: vi.fn(),
    startServer: vi.fn(),
    stopServer: vi.fn(),
    restartServer: vi.fn(),
    reloadServer: vi.fn(),
    deleteServer: vi.fn(),
    testLan: vi.fn(),
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
    putProjectPhpVersion: vi.fn(),
    projectComposer: vi.fn(),
    runComposer: vi.fn(),
    settings: vi.fn(async () => ({})),
    putSettings: vi.fn(),
    mailSettings: vi.fn(),
    putMailSettings: vi.fn(),
    testMail: vi.fn(),
    securityStatus: vi.fn(),
    sftpStatus: vi.fn(),
    putSftpSettings: vi.fn(),
    startSftp: vi.fn(),
    stopSftp: vi.fn(),
    resetSftpPassword: vi.fn(),
  },
  isNativeWebForge: vi.fn(() => false),
  pickDirectory: vi.fn(),
  getPowerStatus: vi.fn(),
  openBatterySettings: vi.fn(async () => undefined),
  requestIgnoreBatteryOptimizations: vi.fn(async () => ({ ignoringBatteryOptimizations: true })),
  setKeepCpuAwake: vi.fn(),
  resetApiConfigCache: vi.fn(),
  resolveApiConfig: vi.fn(async () => ({ base: 'http://127.0.0.1:8765', token: 'token-uji-panjang' })),
  startCore: vi.fn(async () => undefined),
  getCoreStatus: vi.fn(async () => ({ startupState: 'ready', assetState: 'ready', loadError: null })),
}));

const mockApi = vi.mocked(api);
const mockIsNative = vi.mocked(isNativeWebForge);

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

const phpProject: Project = {
  id: 'p1',
  name: 'Situs Laravel',
  slug: 'situs-laravel',
  path: '/data/ws/situs-laravel',
  document_root: 'public',
  created_at: '2026-11-26T00:00:00Z',
  updated_at: '2026-11-26T00:00:00Z',
  php_version: '8.4',
  framework: {
    framework: 'Laravel',
    language: 'PHP',
    version: '11.0',
    recommended_runtime: 'php',
    recommended_document_root: 'public',
    readiness: 'ready',
    notes: [],
    detected_files: ['artisan'],
    php_version_constraint: '^8.2',
    recommended_php_version: '8.2',
  },
};

const phpInfo: PhpRuntimeInfo = {
  available: true,
  version: '8.3.14',
  extensions: ['mbstring', 'curl', 'gd'],
  binary_path: '/data/php83',
  error: null,
  selected_version: '8.3',
  versions: [
    { version: '8.2', patch_version: null, available: false, fpm_available: false, extensions: [], binary_path: null, fpm_binary_path: null, error: 'Binary PHP 8.2 tidak ditemukan' },
    { version: '8.3', patch_version: '8.3.14', available: true, fpm_available: true, extensions: ['mbstring', 'curl', 'gd'], binary_path: '/data/php83', fpm_binary_path: '/data/php-fpm83', error: null },
    { version: '8.4', patch_version: '8.4.2', available: true, fpm_available: true, extensions: ['mbstring', 'intl'], binary_path: '/data/php84', fpm_binary_path: '/data/php-fpm84', error: null },
  ],
  composer: { available: true, version: '2.8.0', path: '/data/composer', error: null },
};

const runtimes: RuntimeInfo[] = [
  { id: 'static', name: 'WebForge Static Server', available: true, version: '0.5.0', reason: null },
  { id: 'php', name: 'PHP', available: true, version: '8.3.14', reason: null },
  { id: 'apache', name: 'Apache', available: true, version: '2.4.62', reason: null },
  { id: 'nginx', name: 'Nginx', available: true, version: '1.26.2', reason: null },
  { id: 'caddy', name: 'Caddy', available: true, version: '2.8.4', reason: null },
];

const fpmServer: ServerInstance = {
  id: 'srv-fpm', project_id: 'p1', project_name: 'Situs Laravel', engine: 'apache', port: 8080,
  bind_lan: false, document_root: 'public', status: 'running', pid: 42, started_at: '2026-11-26T00:00:00Z',
  uptime_seconds: 60, url_local: 'http://127.0.0.1:8080', url_lan: null, error_message: null,
  config_text: null, proxy_port: null, command: null, php_version: '8.4', backend_kind: 'php-fpm', backend_port: 19084,
};

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  document.documentElement.classList.remove('dark');
  stubMatchMedia(false);
  mockIsNative.mockReturnValue(false);
  useAppStore.setState({ section: 'dashboard', activeProjectId: null });
  useAppStore.getState().setTheme('light');
  mockApi.health.mockResolvedValue({ status: 'ok', version: '0.5.0', uptime_seconds: 10 });
  mockApi.systemInfo.mockResolvedValue({
    version: '0.5.0', os: 'android', arch: 'arm64', data_dir: '/data', workspace_dir: '/data/ws',
    management_port: 8765, started_at: '2026-11-26T00:00:00Z', runtimes: [],
  });
  mockApi.monitoringCurrent.mockResolvedValue({
    cpu_percent: 1, mem_total_bytes: 100, mem_used_bytes: 10, storage_total_bytes: 100, storage_used_bytes: 10,
    processes: [], ports: [],
  });
  mockApi.network.mockResolvedValue({ lan_ip: null, interfaces: [], management_url: 'http://127.0.0.1:8765' });
  mockApi.projects.mockResolvedValue([phpProject]);
  mockApi.project.mockResolvedValue(phpProject);
  mockApi.servers.mockResolvedValue([]);
  mockApi.runtimes.mockResolvedValue(runtimes);
  mockApi.phpInfo.mockResolvedValue(phpInfo);
  mockApi.phpSettings.mockResolvedValue({
    memory_limit: '256M', upload_max_filesize: '32M', post_max_size: '32M',
    max_execution_time: 60, date_timezone: 'Asia/Jakarta', display_errors: true,
  });
  mockApi.projectComposer.mockResolvedValue({
    available: true, version: '2.8.0', has_composer_json: true, has_lock: true,
    vendor_present: false, php_version: '8.4', recommended_php_version: '8.2', error: null,
  });
  mockApi.runComposer.mockResolvedValue({
    exit_code: 0, stdout: 'Generating autoload files', stderr: '', timed_out: false, duration_ms: 1234,
  });
  mockApi.mailSettings.mockResolvedValue({
    configured: true, host: 'smtp.contoh.com', port: 587, security: 'starttls',
    username: 'user@contoh.com', from: 'noreply@contoh.com', has_password: true,
  });
  mockApi.putMailSettings.mockImplementation(async (body) => ({
    configured: true, host: body.host, port: body.port, security: body.security,
    username: body.username, from: body.from, has_password: true,
  }));
  mockApi.testMail.mockResolvedValue({ ok: true, error: null, output: '250 OK', duration_ms: 42 });
  mockApi.securityStatus.mockResolvedValue({
    api_token_set: true, management_bind: '127.0.0.1', lan_servers: 0, warnings: [],
  });
  mockApi.sftpStatus.mockResolvedValue({
    running: false, bind_lan: false, port: 2222, username: 'webforge',
    host_key_fingerprint: 'SHA256:abc', root: '/data/ws', started_at: null, last_error: null, connections: [],
  });
  mockApi.startSftp.mockResolvedValue({
    running: true, bind_lan: false, port: 2222, username: 'webforge',
    host_key_fingerprint: 'SHA256:abc', root: '/data/ws', started_at: '2026-11-26T00:00:00Z', last_error: null, connections: [],
  });
  mockApi.stopSftp.mockResolvedValue({
    running: false, bind_lan: false, port: 2222, username: 'webforge',
    host_key_fingerprint: 'SHA256:abc', root: '/data/ws', started_at: null, last_error: null, connections: [],
  });
  mockApi.resetSftpPassword.mockResolvedValue({ password: 'sandi-baru-123' });
});

describe('Servers v0.5.0', () => {
  it('pemilih versi PHP memakai default proyek, opsi tidak tersedia nonaktif, Apache+PHP aktif via FPM', async () => {
    mockApi.createServer.mockResolvedValue({ ...fpmServer, status: 'stopped' });
    const { container } = renderWithQuery(<Servers />);
    const projectSelect = container.querySelectorAll('select')[0];
    await waitFor(() => expect(projectSelect.querySelectorAll('option').length).toBeGreaterThan(1));
    fireEvent.change(projectSelect, { target: { value: 'p1' } });
    const versionSelect = (await screen.findAllByLabelText(/Versi PHP/))[0];
    await waitFor(() => expect(versionSelect).toHaveValue('8.4'));
    expect(versionSelect.querySelector('option[value="8.2"]')).toBeDisabled();
    const engineSelect = screen.getByDisplayValue('PHP built-in server');
    fireEvent.change(engineSelect, { target: { value: 'apache' } });
    expect(await screen.findByText(/Apache menjalankan PHP via PHP-FPM/)).toBeInTheDocument();
    const createButton = screen.getByRole('button', { name: 'Buat server' });
    await waitFor(() => expect(createButton).not.toBeDisabled());
    fireEvent.click(createButton);
    await waitFor(() => expect(mockApi.createServer).toHaveBeenCalledWith(expect.objectContaining({
      project_id: 'p1', engine: 'apache', php_version: '8.4',
    })));
  });

  it('kartu server menampilkan php_version dan backend_kind PHP-FPM', async () => {
    mockApi.servers.mockResolvedValue([fpmServer]);
    renderWithQuery(<Servers />);
    expect(await screen.findByText('Versi PHP: 8.4')).toBeInTheDocument();
    expect(screen.getByText('Backend: PHP-FPM')).toBeInTheDocument();
  });

  it('tombol disabled punya alasan terbaca dan gaya disabled eksplisit', async () => {
    expect(btnPrimary).toContain('disabled:cursor-not-allowed');
    expect(btnPrimary).toContain('disabled:bg-slate-300');
    expect(btnPrimary).not.toContain('opacity-50');
    const { container } = renderWithQuery(<Servers />);
    const createButton = await screen.findByRole('button', { name: 'Buat server' });
    expect(createButton).toBeDisabled();
    expect(screen.getByText(/Tombol nonaktif: Pilih proyek terlebih dahulu/)).toBeInTheDocument();
    await waitFor(() => expect(container.querySelectorAll('select')[0].querySelectorAll('option').length).toBeGreaterThan(1));
    fireEvent.change(container.querySelectorAll('select')[0], { target: { value: 'p1' } });
    await screen.findByLabelText('Versi PHP');
    fireEvent.change(screen.getByPlaceholderText(/Port \(1024/), { target: { value: '80' } });
    expect(await screen.findByText(/Tombol nonaktif: Port tidak valid/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Buat server' })).toBeDisabled();
  });
});

describe('PhpManager dan Composer v0.5.0', () => {
  it('menampilkan status FPM per versi, info Composer, dan menjalankan install/require', async () => {
    useAppStore.setState({ activeProjectId: 'p1' });
    renderWithQuery(<PhpManager />);
    expect(await screen.findByText(/PHP 8\.3: tersedia · PHP-FPM: tersedia/)).toBeInTheDocument();
    expect(screen.getAllByText(/Composer: tersedia \(2\.8\.0\)/).length).toBeGreaterThan(0);
    expect(await screen.findByText(/composer\.json: ada/)).toBeInTheDocument();
    expect(screen.getByText(/membutuhkan jaringan internet/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Composer Install' }));
    await waitFor(() => expect(mockApi.runComposer).toHaveBeenCalledWith('p1', { action: 'install', php_version: '8.3' }));
    expect(await screen.findByText(/Generating autoload files/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Paket untuk Require'), { target: { value: 'monolog/monolog' } });
    fireEvent.click(screen.getByRole('button', { name: 'Require Paket' }));
    await waitFor(() => expect(mockApi.runComposer).toHaveBeenCalledWith('p1', { action: 'require', package: 'monolog/monolog', php_version: '8.3' }));
  });
});

describe('Projects v0.5.0', () => {
  it('menampilkan rekomendasi PHP framework dan mengubah versi PHP proyek', async () => {
    mockApi.putProjectPhpVersion.mockResolvedValue({ ...phpProject, php_version: '8.3' });
    renderWithQuery(<Projects />);
    fireEvent.click(await screen.findByRole('button', { name: /Situs Laravel/ }));
    expect(await screen.findByText(/Constraint versi PHP:/)).toBeInTheDocument();
    expect(screen.getAllByText(/\^8\.2/).length).toBeGreaterThan(0);
    expect(screen.getByText(/Rekomendasi versi PHP: 8\.2/)).toBeInTheDocument();
    const select = await screen.findByLabelText('Versi PHP proyek');
    fireEvent.change(select, { target: { value: '8.3' } });
    await waitFor(() => expect(mockApi.putProjectPhpVersion).toHaveBeenCalledWith('p1', '8.3'));
  });
});

describe('Settings SMTP v0.5.0', () => {
  it('menyimpan SMTP tanpa mengirim password kosong dan mengirim email tes', async () => {
    renderWithQuery(<Settings />);
    const host = await screen.findByLabelText('Host SMTP');
    await waitFor(() => expect(host).toHaveValue('smtp.contoh.com'));
    expect(screen.getByLabelText('Password SMTP')).toHaveValue('');
    fireEvent.change(host, { target: { value: 'smtp.baru.com' } });
    fireEvent.click(screen.getByRole('button', { name: 'Simpan SMTP' }));
    await waitFor(() => expect(mockApi.putMailSettings).toHaveBeenCalledWith(expect.objectContaining({ host: 'smtp.baru.com', security: 'starttls' })));
    const sent = mockApi.putMailSettings.mock.calls[0][0] as unknown as Record<string, unknown>;
    expect(sent).not.toHaveProperty('password');
    fireEvent.change(screen.getByLabelText('Tujuan email tes'), { target: { value: 'tujuan@contoh.com' } });
    fireEvent.click(screen.getByRole('button', { name: 'Kirim Email Tes' }));
    await waitFor(() => expect(mockApi.testMail).toHaveBeenCalledWith('tujuan@contoh.com'));
    expect(await screen.findByText(/Email tes terkirim/)).toBeInTheDocument();
    expect(screen.getByText(/250 OK/)).toBeInTheDocument();
  });

  it('menampilkan branding Faza X TCP di Tentang', async () => {
    renderWithQuery(<Settings />);
    expect(await screen.findByText('Tentang')).toBeInTheDocument();
    expect(screen.getByText(/Faza X TCP/)).toBeInTheDocument();
  });
});

describe('SFTP v0.5.0', () => {
  it('start SFTP memanggil endpoint dan menampilkan fingerprint serta root', async () => {
    renderWithQuery(<Security />);
    expect(await screen.findByText(/SHA256:abc/)).toBeInTheDocument();
    expect(screen.getByText(/Root workspace:/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Start SFTP' }));
    await waitFor(() => expect(mockApi.startSftp).toHaveBeenCalledTimes(1));
  });

  it('stop SFTP saat berjalan dan peringatan LAN tampil', async () => {
    mockApi.sftpStatus.mockResolvedValue({
      running: true, bind_lan: true, port: 2222, username: 'webforge',
      host_key_fingerprint: 'SHA256:abc', root: '/data/ws', started_at: '2026-11-26T00:00:00Z', last_error: null,
      connections: [{ id: 'c1', username: 'webforge', remote: '192.168.1.5:5000', connected_at: '2026-11-26T00:00:00Z', last_activity: '2026-11-26T00:01:00Z' }],
    });
    renderWithQuery(<Security />);
    expect(await screen.findByText(/Peringatan keamanan:/)).toBeInTheDocument();
    expect(screen.getByText(/webforge dari 192\.168\.1\.5:5000/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Stop SFTP' }));
    await waitFor(() => expect(mockApi.stopSftp).toHaveBeenCalledTimes(1));
  });

  it('reset password menampilkan password sekali dan bisa disembunyikan', async () => {
    renderWithQuery(<Security />);
    fireEvent.click(await screen.findByRole('button', { name: 'Reset Password SFTP' }));
    await waitFor(() => expect(mockApi.resetSftpPassword).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('sandi-baru-123')).toBeInTheDocument();
    expect(screen.getByText(/hanya ditampilkan sekali/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Sembunyikan password' }));
    await waitFor(() => expect(screen.queryByText('sandi-baru-123')).toBeNull());
  });
});

describe('Branding dan tema v0.5.0', () => {
  it('header menampilkan Faza X TCP dan toggle tema tetap berfungsi dengan kelas glass', async () => {
    const { container } = renderWithQuery(<App />);
    expect(await screen.findByText(/Faza X TCP · Dashboard/)).toBeInTheDocument();
    expect(container.querySelector('.wf-app')).not.toBeNull();
    expect(container.querySelector('.wf-header')).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Ganti ke tema gelap' }));
    await waitFor(() => expect(document.documentElement.classList.contains('dark')).toBe(true));
    expect(useAppStore.getState().theme).toBe('dark');
    fireEvent.click(screen.getByRole('button', { name: 'Ganti ke tema terang' }));
    await waitFor(() => expect(document.documentElement.classList.contains('dark')).toBe(false));
    expect(useAppStore.getState().theme).toBe('light');
  });
});
