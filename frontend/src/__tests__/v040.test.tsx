import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Servers } from '../pages/Servers';
import { SelfTestResults } from '../components/DiagnosticsPanel';
import { api, getPowerStatus, isNativeWebForge } from '../lib/api';
import type { Project, RuntimeInfo, ServerInstance } from '../lib/types';

vi.mock('../lib/api', () => ({
  api: {
    servers: vi.fn(async () => []),
    projects: vi.fn(async () => []),
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
    mailSettings: vi.fn(async () => ({ configured: false, host: '', port: 587, security: 'starttls', username: '', from: '', has_password: false })),
    putMailSettings: vi.fn(),
    testMail: vi.fn(),
    sftpStatus: vi.fn(),
    putSftpSettings: vi.fn(),
    startSftp: vi.fn(),
    stopSftp: vi.fn(),
    resetSftpPassword: vi.fn(),
  },
  isNativeWebForge: vi.fn(() => false),
  getPowerStatus: vi.fn(),
  openBatterySettings: vi.fn(async () => undefined),
  requestIgnoreBatteryOptimizations: vi.fn(async () => ({ ignoringBatteryOptimizations: true })),
}));

const mockApi = vi.mocked(api);
const mockIsNative = vi.mocked(isNativeWebForge);
const mockGetPowerStatus = vi.mocked(getPowerStatus);

function renderWithQuery(ui: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

const phpProject: Project = {
  id: 'p1',
  name: 'Situs PHP',
  slug: 'situs-php',
  path: '/data/ws/situs-php',
  document_root: '',
  created_at: '2026-11-26T00:00:00Z',
  updated_at: '2026-11-26T00:00:00Z',
  framework: {
    framework: 'Native PHP',
    language: 'PHP',
    version: null,
    recommended_runtime: 'php',
    recommended_document_root: '',
    readiness: 'ready',
    notes: [],
    detected_files: ['index.php'],
  },
};

const runtimes: RuntimeInfo[] = [
  { id: 'static', name: 'WebForge Static Server', available: true, version: '0.4.0', reason: null },
  { id: 'php', name: 'PHP', available: true, version: '8.3.14', reason: null },
  { id: 'nginx', name: 'Nginx', available: true, version: '1.26.2', reason: null },
  { id: 'apache', name: 'Apache', available: true, version: '2.4.62', reason: null },
  { id: 'caddy', name: 'Caddy', available: true, version: '2.8.4', reason: null },
];

const degradedServer: ServerInstance = {
  id: 'srv-1',
  project_id: 'p1',
  project_name: 'Situs PHP',
  engine: 'nginx',
  port: 8080,
  bind_lan: true,
  document_root: '',
  status: 'degraded',
  pid: 1234,
  started_at: '2026-11-26T00:00:00Z',
  uptime_seconds: 120,
  url_local: 'http://127.0.0.1:8080',
  url_lan: 'http://192.168.1.20:8080',
  error_message: null,
  in_process: false,
  config_text: null,
  proxy_port: null,
  command: null,
  backend_port: 18080,
  warning_message: 'Server menjawab HTTP 403 pada /; periksa index document root dan izin folder proyek',
  last_health_at: '2026-11-26T00:01:00Z',
  last_health_status: 403,
  last_health_error: null,
  last_health_target: 'http://192.168.1.20:8080/',
  health_failures: 2,
  auto_restart_count: 1,
};

beforeEach(() => {
  vi.clearAllMocks();
  mockIsNative.mockReturnValue(false);
  mockApi.servers.mockResolvedValue([degradedServer]);
  mockApi.projects.mockResolvedValue([phpProject]);
  mockApi.runtimes.mockResolvedValue(runtimes);
  mockApi.testLan.mockResolvedValue({
    ok: true,
    url: 'http://192.168.1.20:8080/',
    host: '192.168.1.20',
    port: 8080,
    status_code: 200,
    duration_ms: 12,
    target: 'http://192.168.1.20:8080/',
    error: null,
    warning: null,
  });
});

describe('Servers v0.4.0', () => {
  it('menampilkan status degraded, peringatan docroot, backend komposit, dan bukti kesehatan', async () => {
    renderWithQuery(<Servers />);
    expect(await screen.findByText('Degraded')).toBeInTheDocument();
    expect(screen.getByText(/Server menjawab HTTP 403 pada \//)).toBeInTheDocument();
    expect(screen.getByText(/Backend PHP komposit internal/)).toBeInTheDocument();
    expect(screen.getByText(/127\.0\.0\.1:18080/)).toBeInTheDocument();
    expect(screen.getByText(/Kesehatan terakhir: http:\/\/192\.168\.1\.20:8080\//)).toBeInTheDocument();
    expect(screen.getByText(/gagal beruntun 2/)).toBeInTheDocument();
    expect(screen.getByText(/restart otomatis 1/)).toBeInTheDocument();
  });

  it('menjalankan Tes Akses LAN nyata dan menampilkan hasilnya', async () => {
    renderWithQuery(<Servers />);
    const button = await screen.findByRole('button', { name: /Tes Akses LAN/ }, { timeout: 10000 });
    fireEvent.click(button);
    await waitFor(() => expect(mockApi.testLan).toHaveBeenCalledWith('srv-1'), { timeout: 10000 });
    expect(await screen.findByText(/Tes Akses LAN: berhasil/, {}, { timeout: 10000 })).toBeInTheDocument();
    expect(screen.getByText(/HTTP 200 · 12 ms/)).toBeInTheDocument();
  });

  it('menampilkan banner baterai saat server aktif tetapi pengecualian baterai belum diberikan', async () => {
    mockIsNative.mockReturnValue(true);
    mockGetPowerStatus.mockResolvedValue({ ignoringBatteryOptimizations: false, keepCpuAwake: true });
    renderWithQuery(<Servers />);
    expect(await screen.findByText(/Baterai aplikasi masih dibatasi Android/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Minta pengecualian baterai/ })).toBeInTheDocument();
    expect(screen.getByText(/tidak dapat dideteksi atau diubah lewat API Android/)).toBeInTheDocument();
  });

  it('Apache+PHP aktif via PHP-FPM dan mode komposit ditampilkan sebelum server dibuat', async () => {
    const { container } = renderWithQuery(<Servers />);
    await screen.findByText('Degraded');
    const projectSelect = container.querySelectorAll('select')[0];
    fireEvent.change(projectSelect, { target: { value: 'p1' } });
    const engineSelect = await screen.findByDisplayValue('PHP built-in server');
    fireEvent.change(engineSelect, { target: { value: 'apache' } });
    expect(await screen.findByText(/Apache menjalankan PHP via PHP-FPM/)).toBeInTheDocument();
    fireEvent.change(engineSelect, { target: { value: 'nginx' } });
    expect(await screen.findByText(/Mode komposit PHP/)).toBeInTheDocument();
  });
});

describe('SelfTestResults v0.4.0', () => {
  it('menampilkan hasil loopback dan LAN secara terpisah', () => {
    render(
      <SelfTestResults
        results={[
          {
            engine: 'caddy',
            ok: false,
            duration_ms: 100,
            detail: 'Marker terverifikasi via HTTP GET / di 127.0.0.1:18080',
            error: 'Jalur LAN gagal: timeout',
            lan_ok: false,
            lan_detail: null,
            lan_error: 'timeout membaca respons HTTP',
          },
        ]}
      />,
    );
    expect(screen.getByText(/Loopback: Marker terverifikasi/)).toBeInTheDocument();
    expect(screen.getByText(/LAN: Gagal/)).toBeInTheDocument();
    expect(screen.getByText(/Error LAN: timeout membaca respons HTTP/)).toBeInTheDocument();
  });
});
