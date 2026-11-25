import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Servers } from '../pages/Servers';
import { Settings } from '../pages/Settings';
import { PhpManager, PhpReadiness } from '../components/PhpManager';
import {
  api, getPowerStatus, isNativeWebForge, requestIgnoreBatteryOptimizations, setKeepCpuAwake,
} from '../lib/api';
import type { PhpSettings, Project } from '../lib/types';

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
    deleteServer: vi.fn(),
    phpInfo: vi.fn(),
    phpSettings: vi.fn(),
    putPhpSettings: vi.fn(),
    phpCli: vi.fn(),
    settings: vi.fn(async () => ({})),
    putSettings: vi.fn(),
    health: vi.fn(),
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
const mockIsNative = vi.mocked(isNativeWebForge);
const mockGetPower = vi.mocked(getPowerStatus);
const mockSetKeepCpu = vi.mocked(setKeepCpuAwake);
const mockRequestExemption = vi.mocked(requestIgnoreBatteryOptimizations);

const laravel: Project = {
  id: 'p-laravel', name: 'Situs Laravel', slug: 'situs-laravel', path: '/ws/situs-laravel',
  document_root: 'public', created_at: '2026-11-25T00:00:00Z', updated_at: '2026-11-25T00:00:00Z',
  framework: {
    framework: 'Laravel', language: 'PHP', version: '11.0', recommended_runtime: 'php',
    recommended_document_root: 'public', readiness: 'needs_runtime', notes: [], detected_files: ['artisan'],
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

const phpSettings: PhpSettings = {
  memory_limit: '256M', upload_max_filesize: '32M', post_max_size: '32M',
  max_execution_time: 60, date_timezone: 'Asia/Jakarta', display_errors: true,
};

function renderWithQuery(ui: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mockIsNative.mockReturnValue(false);
  mockApi.projects.mockResolvedValue([laravel, staticProject]);
  mockApi.servers.mockResolvedValue([]);
  mockApi.runtimes.mockResolvedValue([]);
  mockApi.phpInfo.mockResolvedValue({
    available: true, version: '8.3.0', extensions: ['mbstring', 'json'], binary_path: '/data/php', error: null,
  });
  mockApi.phpSettings.mockResolvedValue(phpSettings);
});

async function selectProjectInServers(id: string) {
  await screen.findAllByRole('option', { name: 'Situs Laravel' });
  const projectSelect = screen.getAllByRole('combobox')[0];
  fireEvent.change(projectSelect, { target: { value: id } });
  return projectSelect;
}

describe('Servers engine PHP', () => {
  it('proyek PHP mendapat pilihan PHP built-in server dengan document root rekomendasi, proyek static tidak', async () => {
    renderWithQuery(<Servers />);
    const projectSelect = await selectProjectInServers('p-laravel');
    expect(await screen.findByRole('option', { name: 'PHP built-in server' })).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Document root relatif/)).toHaveValue('public');

    fireEvent.change(projectSelect, { target: { value: 'p-static' } });
    await waitFor(() => expect(screen.queryByRole('option', { name: 'PHP built-in server' })).toBeNull());
    expect(screen.getByText(/Engine: WebForge Static Server/)).toBeInTheDocument();
  });

  it('membuat server PHP mengirim engine php dan document root rekomendasi', async () => {
    mockApi.createServer.mockResolvedValue({
      id: 's1', project_id: 'p-laravel', project_name: 'Situs Laravel', engine: 'php', port: 8080,
      bind_lan: false, document_root: 'public', status: 'stopped', pid: null, started_at: null,
      uptime_seconds: 0, url_local: 'http://127.0.0.1:8080', url_lan: null, error_message: null,
    });
    renderWithQuery(<Servers />);
    await selectProjectInServers('p-laravel');
    fireEvent.click(await screen.findByRole('button', { name: 'Buat server' }));
    await waitFor(() => expect(mockApi.createServer).toHaveBeenCalledWith({
      project_id: 'p-laravel', engine: 'php', port: 8080, bind_lan: false, document_root: 'public',
    }));
  });

  it('error 422 dari backend ditampilkan apa adanya', async () => {
    mockApi.createServer.mockRejectedValue(new Error('Runtime PHP belum tersedia di perangkat ini'));
    renderWithQuery(<Servers />);
    await selectProjectInServers('p-laravel');
    fireEvent.click(await screen.findByRole('button', { name: 'Buat server' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Runtime PHP belum tersedia di perangkat ini');
  });
});

describe('PhpManager', () => {
  it('editor php.ini memuat nilai dari GET dan menyimpan via PUT', async () => {
    mockApi.putPhpSettings.mockImplementation(async (v) => v);
    renderWithQuery(<PhpManager />);
    const memory = await screen.findByLabelText('memory_limit');
    expect(memory).toHaveValue('256M');
    expect(screen.getByLabelText(/max_execution_time/)).toHaveValue('60');
    expect(screen.getByLabelText(/display_errors/)).toBeChecked();
    expect(screen.getByText(/Versi 8.3.0/)).toBeInTheDocument();
    expect(screen.getByText('mbstring')).toBeInTheDocument();

    fireEvent.change(memory, { target: { value: '512M' } });
    fireEvent.click(screen.getByRole('button', { name: 'Simpan php.ini' }));
    await waitFor(() => expect(mockApi.putPhpSettings).toHaveBeenCalledWith({ ...phpSettings, memory_limit: '512M' }));
    expect(await screen.findByText('Pengaturan PHP tersimpan.')).toBeInTheDocument();
  });

  it('status PHP belum tersedia menampilkan error backend dan tombol muat ulang memanggil ulang endpoint', async () => {
    mockApi.phpInfo.mockResolvedValue({
      available: false, version: null, extensions: [], binary_path: null, error: 'Binary PHP tidak ditemukan',
    });
    renderWithQuery(<PhpManager />);
    expect(await screen.findByText('PHP belum tersedia')).toBeInTheDocument();
    expect(screen.getByText('Binary PHP tidak ditemukan')).toBeInTheDocument();
    const before = mockApi.phpInfo.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Muat ulang info PHP' }));
    await waitFor(() => expect(mockApi.phpInfo.mock.calls.length).toBeGreaterThan(before));
  });
});

describe('PhpReadiness', () => {
  it('menandai ekstensi yang kurang dan yang terpenuhi dari daftar nyata', async () => {
    mockApi.phpInfo.mockResolvedValue({
      available: true, version: '8.3.0',
      extensions: ['mbstring', 'openssl', 'pdo', 'tokenizer', 'xml', 'ctype', 'json', 'fileinfo'],
      binary_path: '/data/php', error: null,
    });
    renderWithQuery(<PhpReadiness project={laravel} />);
    const bcmath = await screen.findByText('bcmath');
    expect(bcmath.closest('li')).toHaveTextContent('Kurang');
    expect(screen.getByText('mbstring').closest('li')).toHaveTextContent('Terpenuhi');
  });

  it('bila PHP belum tersedia menampilkan status itu, bukan daftar terpenuhi', async () => {
    mockApi.phpInfo.mockResolvedValue({
      available: false, version: null, extensions: [], binary_path: null, error: 'Binary PHP tidak ditemukan',
    });
    renderWithQuery(<PhpReadiness project={laravel} />);
    expect(await screen.findByText(/PHP belum tersedia/)).toBeInTheDocument();
    expect(screen.queryByText('Terpenuhi')).toBeNull();
  });

  it('proyek static tidak menampilkan readiness PHP', () => {
    renderWithQuery(<PhpReadiness project={staticProject} />);
    expect(screen.queryByText(/Kesiapan ekstensi PHP/)).toBeNull();
  });
});

describe('Settings anti-stuck', () => {
  it('di native toggle WakeLock memanggil plugin dan tombol pengecualian baterai bekerja', async () => {
    mockIsNative.mockReturnValue(true);
    mockGetPower.mockResolvedValue({ ignoringBatteryOptimizations: false, keepCpuAwake: false });
    mockSetKeepCpu.mockResolvedValue({ keepCpuAwake: true });
    mockRequestExemption.mockResolvedValue({ ignoringBatteryOptimizations: true });
    renderWithQuery(<Settings />);
    const toggle = await screen.findByLabelText(/Jaga CPU tetap bangun saat server berjalan/);
    await waitFor(() => expect(toggle).not.toBeDisabled());
    expect(screen.getByText(/belum aktif/)).toBeInTheDocument();
    fireEvent.click(toggle);
    await waitFor(() => expect(mockSetKeepCpu).toHaveBeenCalledWith(true));
    fireEvent.click(screen.getByRole('button', { name: 'Minta pengecualian optimasi baterai' }));
    await waitFor(() => expect(mockRequestExemption).toHaveBeenCalledTimes(1));
    expect(screen.getByText('Autostart')).toBeInTheDocument();
    expect(screen.getByText('Tanpa batasan')).toBeInTheDocument();
  });

  it('di web section anti-stuck menjelaskan hanya tersedia di aplikasi Android', async () => {
    mockIsNative.mockReturnValue(false);
    renderWithQuery(<Settings />);
    expect(await screen.findByText(/hanya tersedia di aplikasi Android/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/Jaga CPU tetap bangun/)).toBeNull();
  });
});
