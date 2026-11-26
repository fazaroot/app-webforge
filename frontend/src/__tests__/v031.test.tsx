import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import App from '../App';
import { OfflineBanner } from '../components/OfflineBanner';
import { RuntimePanel } from '../components/RuntimePanel';
import { SelfTestPanel, StartupLogPanel } from '../components/DiagnosticsPanel';
import { api, getCoreStatus, getStartupLog, isNativeWebForge, startCore } from '../lib/api';
import { useAppStore } from '../store';

vi.mock('../lib/api', () => ({
  api: {
    health: vi.fn(),
    systemInfo: vi.fn(),
    diagnostics: vi.fn(),
    selfTest: vi.fn(),
    runtimes: vi.fn(async () => []),
    testRuntime: vi.fn(),
    projects: vi.fn(async () => []),
    servers: vi.fn(async () => []),
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
    logs: vi.fn(async () => []),
    logsExport: vi.fn(),
    gitStatus: vi.fn(),
    settings: vi.fn(),
    putSettings: vi.fn(),
    securityStatus: vi.fn(),
  },
  isNativeWebForge: vi.fn(() => false),
  startCore: vi.fn(async () => undefined),
  getCoreStatus: vi.fn(),
  getStartupLog: vi.fn(),
  pickDirectory: vi.fn(),
  openBatterySettings: vi.fn(async () => undefined),
  getPowerStatus: vi.fn(),
  setKeepCpuAwake: vi.fn(),
  requestIgnoreBatteryOptimizations: vi.fn(),
  resetApiConfigCache: vi.fn(),
  resolveApiConfig: vi.fn(async () => ({ base: 'http://127.0.0.1:8765', token: '' })),
}));

const mockApi = vi.mocked(api);
const mockIsNative = vi.mocked(isNativeWebForge);
const mockStartCore = vi.mocked(startCore);
const mockGetCoreStatus = vi.mocked(getCoreStatus);
const mockGetStartupLog = vi.mocked(getStartupLog);

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
  mockIsNative.mockReturnValue(false);
  mockStartCore.mockResolvedValue(undefined);
  mockApi.health.mockResolvedValue({ status: 'ok', version: '0.3.1', uptime_seconds: 10 });
  mockApi.systemInfo.mockResolvedValue({
    version: '0.3.1', os: 'android', arch: 'arm64', data_dir: '/data', workspace_dir: '/data/ws',
    management_port: 8765, started_at: '2026-11-26T00:00:00Z', runtimes: [],
  });
  mockApi.monitoringCurrent.mockResolvedValue({
    cpu_percent: 1, mem_total_bytes: 100, mem_used_bytes: 10,
    storage_total_bytes: 100, storage_used_bytes: 10, processes: [], ports: [],
  });
  mockApi.network.mockResolvedValue({ lan_ip: null, interfaces: [], management_url: 'http://127.0.0.1:8765' });
});

describe('OfflineBanner loadError', () => {
  it('menampilkan loadError apa adanya bila tersedia', () => {
    render(<OfflineBanner onRetry={vi.fn()} loadError="libwebforge_core.so gagal dimuat: dlopen failed" />);
    expect(screen.getByText(/Penyebab: libwebforge_core\.so gagal dimuat: dlopen failed/)).toBeInTheDocument();
  });

  it('tanpa loadError tidak menampilkan baris penyebab', () => {
    render(<OfflineBanner onRetry={vi.fn()} />);
    expect(screen.queryByText(/Penyebab:/)).toBeNull();
  });
});

describe('RuntimePanel Tes Runtime', () => {
  it('menampilkan stdout/stderr/exit code/durasi dan memperbarui status kartu', async () => {
    mockApi.testRuntime.mockResolvedValue({
      id: 'php', ok: true, version: '8.3.14',
      stdout: 'PHP 8.3.14 (cli) (built: Nov 26 2026)', stderr: 'peringatan uji dari stderr',
      exit_code: 0, duration_ms: 42, error: null,
    });
    renderWithQuery(
      <RuntimePanel runtimes={[{ id: 'php', name: 'PHP', available: false, version: null, reason: 'probe lama gagal' }]} />,
    );
    expect(screen.getByText('Belum tersedia')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Tes Runtime' }));
    await waitFor(() => expect(mockApi.testRuntime).toHaveBeenCalledWith('php'));
    expect(await screen.findByText('PHP 8.3.14 (cli) (built: Nov 26 2026)')).toBeInTheDocument();
    expect(screen.getByText('peringatan uji dari stderr')).toBeInTheDocument();
    expect(screen.getByText('Exit code: 0')).toBeInTheDocument();
    expect(screen.getByText('Durasi: 42 ms')).toBeInTheDocument();
    expect(screen.getByText('Tes lolos')).toBeInTheDocument();
    // Status kartu diperbarui dari hasil tes segar.
    expect(screen.getByText('Tersedia')).toBeInTheDocument();
    expect(screen.getByText(/Versi 8\.3\.14/)).toBeInTheDocument();
  });

  it('Probe ulang semua memanggil runtimes dengan fresh=true', async () => {
    mockApi.runtimes.mockResolvedValue([
      { id: 'php', name: 'PHP', available: true, version: '8.3.14', reason: null },
    ]);
    renderWithQuery(
      <RuntimePanel runtimes={[{ id: 'php', name: 'PHP', available: false, version: null, reason: 'probe lama gagal' }]} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Probe ulang semua/ }));
    await waitFor(() => expect(mockApi.runtimes).toHaveBeenCalledWith(true));
  });
});

describe('SelfTestPanel', () => {
  it('merender hasil per engine dengan detail dan error persis', async () => {
    mockApi.selfTest.mockResolvedValue({
      results: [
        { engine: 'static', ok: true, duration_ms: 12, detail: 'HTTP 200 dari server uji', error: null },
        { engine: 'php', ok: false, duration_ms: 3000, detail: 'server tidak merespons', error: 'timeout menunggu port' },
      ],
    });
    renderWithQuery(<SelfTestPanel />);
    fireEvent.click(screen.getByRole('button', { name: 'Uji Semua Engine' }));
    expect(await screen.findByText('static')).toBeInTheDocument();
    expect(screen.getByText('Lolos')).toBeInTheDocument();
    expect(screen.getByText(/HTTP 200 dari server uji/)).toBeInTheDocument();
    expect(screen.getByText('php')).toBeInTheDocument();
    expect(screen.getByText('Gagal')).toBeInTheDocument();
    expect(screen.getByText(/server tidak merespons/)).toBeInTheDocument();
    expect(screen.getByText(/Error: timeout menunggu port/)).toBeInTheDocument();
    expect(screen.getByText('Durasi: 3000 ms')).toBeInTheDocument();
  });

  it('menampilkan skeleton selama self-test berjalan', async () => {
    let resolveSelfTest: (v: { results: [] }) => void = () => undefined;
    mockApi.selfTest.mockImplementation(() => new Promise((res) => { resolveSelfTest = res; }));
    renderWithQuery(<SelfTestPanel />);
    fireEvent.click(screen.getByRole('button', { name: 'Uji Semua Engine' }));
    expect(await screen.findByText('Menguji semua engine…')).toBeInTheDocument();
    expect(screen.getAllByTestId('skeleton').length).toBeGreaterThan(0);
    resolveSelfTest({ results: [] });
    expect(await screen.findByText('Core tidak mengembalikan hasil engine apa pun.')).toBeInTheDocument();
  });
});

describe('StartupLogPanel', () => {
  it('native: menampilkan startupState, assetState, dan entri log', async () => {
    mockIsNative.mockReturnValue(true);
    mockGetCoreStatus.mockResolvedValue({ startupState: 'ready', assetState: 'extracted', loadError: null });
    mockGetStartupLog.mockResolvedValue([
      { ts: '2026-11-26T04:00:00Z', message: 'core mulai' },
      { ts: '2026-11-26T04:00:01Z', message: 'aset selesai diekstrak' },
    ]);
    renderWithQuery(<StartupLogPanel />);
    expect(await screen.findByText(/core mulai/)).toBeInTheDocument();
    expect(screen.getByText(/aset selesai diekstrak/)).toBeInTheDocument();
    expect(screen.getByText(/Status startup:/)).toBeInTheDocument();
    expect(screen.getByText('ready')).toBeInTheDocument();
    expect(screen.getByText('extracted')).toBeInTheDocument();
  });

  it('web: menampilkan catatan jujur tidak tersedia tanpa memanggil plugin', () => {
    mockIsNative.mockReturnValue(false);
    renderWithQuery(<StartupLogPanel />);
    expect(screen.getByText(/hanya tersedia di aplikasi Android/)).toBeInTheDocument();
    expect(mockGetStartupLog).not.toHaveBeenCalled();
  });
});

describe('bootstrap core di App (native)', () => {
  it('menampilkan skeleton Menyalakan core… selama startup, lalu Core online', async () => {
    mockIsNative.mockReturnValue(true);
    mockGetCoreStatus.mockResolvedValue({ startupState: 'starting', assetState: 'extracting', loadError: null });
    mockApi.health.mockRejectedValue(new Error('connection refused'));
    renderWithQuery(<App />);
    expect((await screen.findAllByText('Menyalakan core…')).length).toBeGreaterThan(0);
    expect(screen.queryByText(/Tidak dapat terhubung ke WebForge Core/)).toBeNull();
    expect(mockStartCore).toHaveBeenCalled();
    mockApi.health.mockResolvedValue({ status: 'ok', version: '0.3.1', uptime_seconds: 1 });
    expect(await screen.findByText('Core online', {}, { timeout: 4000 })).toBeInTheDocument();
  });

  it('setelah gagal menampilkan loadError dan Coba lagi mengulang bootstrap penuh', async () => {
    mockIsNative.mockReturnValue(true);
    mockGetCoreStatus.mockResolvedValue({ startupState: 'error', assetState: 'ready', loadError: 'libwebforge_core.so gagal dimuat' });
    mockApi.health.mockRejectedValue(new Error('connection refused'));
    renderWithQuery(<App />);
    expect(await screen.findByText(/Penyebab: libwebforge_core\.so gagal dimuat/)).toBeInTheDocument();
    expect(mockStartCore).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: /coba lagi/i }));
    await waitFor(() => expect(mockStartCore).toHaveBeenCalledTimes(2));
  });
});
