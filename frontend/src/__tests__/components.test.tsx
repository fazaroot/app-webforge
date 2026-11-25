import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { RuntimeBadge } from '../components/RuntimeBadge';
import { OfflineBanner } from '../components/OfflineBanner';

describe('RuntimeBadge', () => {
  it('runtime unavailable menampilkan badge dan alasan tanpa tombol start', () => {
    render(<RuntimeBadge runtime={{ id: 'php', name: 'PHP', available: false, version: null, reason: 'Belum dibundel di Phase 1' }} />);
    expect(screen.getByText('Belum tersedia')).toBeInTheDocument();
    expect(screen.getByText('Belum dibundel di Phase 1')).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.queryByText(/start/i)).toBeNull();
  });

  it('runtime available menampilkan versi tanpa tombol start', () => {
    render(<RuntimeBadge runtime={{ id: 'static', name: 'WebForge Static Server', available: true, version: '0.1.0', reason: null }} />);
    expect(screen.getByText('Tersedia')).toBeInTheDocument();
    expect(screen.getByText(/Versi 0.1.0/)).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
  });
});

describe('OfflineBanner', () => {
  it('menampilkan status Core offline dan tombol coba lagi yang berfungsi', () => {
    const onRetry = vi.fn();
    render(<OfflineBanner onRetry={onRetry} />);
    expect(screen.getByText('Core offline')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /coba lagi/i }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});
