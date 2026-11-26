import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Projects } from '../pages/Projects';
import { isNativeWebForge, pickDirectory } from '../lib/api';

vi.mock('../lib/api', () => ({
  api: {
    projects: vi.fn(async () => []),
    createProject: vi.fn(),
    registerProject: vi.fn(),
    scanProject: vi.fn(),
    deleteProject: vi.fn(),
    project: vi.fn(),
    phpInfo: vi.fn(),
    putProjectPhpVersion: vi.fn(),
  },
  isNativeWebForge: vi.fn(() => false),
  pickDirectory: vi.fn(),
}));

const mockIsNative = vi.mocked(isNativeWebForge);
const mockPick = vi.mocked(pickDirectory);

function renderProjects() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <Projects />
    </QueryClientProvider>,
  );
}

describe('Projects SAF', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsNative.mockReturnValue(false);
  });

  it('di web tombol SAF tidak tampil dan input path manual tetap ada', async () => {
    renderProjects();
    expect(await screen.findByText(/Belum ada proyek/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Pilih Folder \(SAF\)/ })).toBeNull();
    expect(screen.getByPlaceholderText(/Path absolut/)).toBeInTheDocument();
  });

  it('di native hasil impor mengisi form registrasi dan menampilkan jumlah file', async () => {
    mockIsNative.mockReturnValue(true);
    mockPick.mockResolvedValue({ path: '/data/workspace/imported/situs', name: 'situs', fileCount: 7 });
    renderProjects();
    fireEvent.click(await screen.findByRole('button', { name: /Pilih Folder \(SAF\)/ }));
    expect(await screen.findByText(/7 file disalin/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Path absolut/)).toHaveValue('/data/workspace/imported/situs');
    expect(screen.getAllByPlaceholderText('Nama proyek')[1]).toHaveValue('situs');
  });

  it('di native pembatalan menampilkan pesan error jujur dari plugin', async () => {
    mockIsNative.mockReturnValue(true);
    mockPick.mockRejectedValue(new Error('Pemilihan folder dibatalkan pengguna'));
    renderProjects();
    fireEvent.click(await screen.findByRole('button', { name: /Pilih Folder \(SAF\)/ }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Pemilihan folder dibatalkan pengguna');
  });
});
