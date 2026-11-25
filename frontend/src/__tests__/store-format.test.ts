import { describe, expect, it } from 'vitest';
import { useAppStore } from '../store';
import { formatBytes, formatPercent, formatUptime, maskToken } from '../lib/format';

describe('store', () => {
  it('setTheme menyimpan pilihan ke localStorage dan state', () => {
    useAppStore.getState().setTheme('dark');
    expect(useAppStore.getState().theme).toBe('dark');
    expect(localStorage.getItem('webforge_theme')).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('setSection dan proyek aktif bekerja', () => {
    useAppStore.getState().setSection('servers');
    expect(useAppStore.getState().section).toBe('servers');
    useAppStore.getState().setActiveProjectId('p1');
    expect(useAppStore.getState().activeProjectId).toBe('p1');
    expect(localStorage.getItem('webforge_active_project')).toBe('p1');
  });
});

describe('format', () => {
  it('formatBytes mengubah satuan dengan benar', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(formatBytes(null)).toBe('Tidak tersedia');
  });

  it('formatUptime dan formatPercent jujur untuk null', () => {
    expect(formatUptime(90)).toBe('1 mnt 30 dtk');
    expect(formatUptime(3660)).toBe('1 jam 1 mnt');
    expect(formatUptime(null)).toBe('Tidak tersedia');
    expect(formatPercent(12.34)).toBe('12.3%');
    expect(formatPercent(null)).toBe('Tidak tersedia');
  });

  it('maskToken tidak menampilkan token penuh', () => {
    const masked = maskToken('abcdef0123456789');
    expect(masked).not.toContain('abcdef');
    expect(masked).toContain('16 karakter');
    expect(maskToken('')).toBe('Belum terpasang');
  });
});
