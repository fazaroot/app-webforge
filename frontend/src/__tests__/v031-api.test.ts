import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  api, getCoreStatus, getStartupLog, parseCoreStatus, resetApiConfigCache, startCore,
} from '../lib/api';
import { bootstrapBackoffMs, bootstrapSchedule, runCoreBootstrap } from '../lib/bootstrap';

function jsonResponse(body: unknown, ok = true, status = 200): Response {
  return {
    ok,
    status,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

describe('jadwal backoff bootstrap (fungsi murni)', () => {
  it('mulai 500ms, naik, dan dibatasi 2000ms', () => {
    expect(bootstrapBackoffMs(0)).toBe(500);
    expect(bootstrapBackoffMs(1)).toBe(1000);
    expect(bootstrapBackoffMs(2)).toBe(2000);
    expect(bootstrapBackoffMs(3)).toBe(2000);
    expect(bootstrapBackoffMs(10)).toBe(2000);
  });

  it('jadwal total tidak melebihi 30 detik dan monoton naik', () => {
    const delays = bootstrapSchedule();
    expect(delays[0]).toBe(500);
    expect(delays[1]).toBe(1000);
    expect(Math.max(...delays)).toBe(2000);
    const total = delays.reduce((a, b) => a + b, 0);
    expect(total).toBeLessThanOrEqual(30000);
    expect(total + 2000).toBeGreaterThan(30000);
    for (let i = 1; i < delays.length; i += 1) {
      expect(delays[i]).toBeGreaterThanOrEqual(delays[i - 1]);
    }
  });

  it('runCoreBootstrap memanggil startCore lalu polling sampai sehat', async () => {
    const start = vi.fn(async () => ({}));
    const health = vi.fn()
      .mockRejectedValueOnce(new Error('connection refused'))
      .mockRejectedValueOnce(new Error('connection refused'))
      .mockResolvedValue({ status: 'ok' });
    const sleeps: number[] = [];
    const result = await runCoreBootstrap({
      startCore: start,
      health,
      getStatus: async () => ({ startupState: 'starting', assetState: 'extracting', loadError: null }),
      sleep: async (ms) => { sleeps.push(ms); },
    });
    expect(start).toHaveBeenCalledTimes(1);
    expect(result.ok).toBe(true);
    expect(result.attempts).toBe(3);
    expect(sleeps).toEqual([500, 1000]);
  });

  it('runCoreBootstrap berhenti dengan loadError saat startupState error', async () => {
    const result = await runCoreBootstrap({
      startCore: async () => ({}),
      health: async () => { throw new Error('connection refused'); },
      getStatus: async () => ({ startupState: 'error', assetState: 'ready', loadError: 'libwebforge_core.so gagal dimuat' }),
      sleep: async () => undefined,
    });
    expect(result.ok).toBe(false);
    expect(result.loadError).toBe('libwebforge_core.so gagal dimuat');
    expect(result.error).toBe('libwebforge_core.so gagal dimuat');
    expect(result.attempts).toBe(1);
  });

  it('runCoreBootstrap menyerah setelah jadwal habis dengan error health terakhir', async () => {
    const result = await runCoreBootstrap({
      startCore: async () => ({}),
      health: async () => { throw new Error('connection refused'); },
      sleep: async () => undefined,
      maxMs: 1500,
    });
    expect(result.ok).toBe(false);
    expect(result.error).toBe('connection refused');
    expect(result.attempts).toBe(3);
  });
});

describe('api v0.3.1', () => {
  beforeEach(() => {
    localStorage.clear();
    resetApiConfigCache();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('runtimes memakai cache secara default dan ?fresh=1 bila fresh=true', async () => {
    const fetchMock = vi.fn(async () => jsonResponse([]));
    vi.stubGlobal('fetch', fetchMock);
    await api.runtimes();
    expect((fetchMock.mock.calls[0] as unknown as [string])[0]).toBe('http://127.0.0.1:8765/api/runtimes');
    await api.runtimes(true);
    expect((fetchMock.mock.calls[1] as unknown as [string])[0]).toBe('http://127.0.0.1:8765/api/runtimes?fresh=1');
    // Konteks react-query bukan boolean true: tidak boleh memicu fresh.
    await api.runtimes({ queryKey: ['runtimes'] } as unknown as boolean);
    expect((fetchMock.mock.calls[2] as unknown as [string])[0]).toBe('http://127.0.0.1:8765/api/runtimes');
  });

  it('testRuntime POST ke /api/runtimes/:id/test dan selfTest POST ke /api/diagnostics/self-test', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ ok: true }));
    vi.stubGlobal('fetch', fetchMock);
    await api.testRuntime('php');
    const [testUrl, testInit] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(testUrl).toBe('http://127.0.0.1:8765/api/runtimes/php/test');
    expect(testInit.method).toBe('POST');
    await api.selfTest();
    const [selfUrl, selfInit] = fetchMock.mock.calls[1] as unknown as [string, RequestInit];
    expect(selfUrl).toBe('http://127.0.0.1:8765/api/diagnostics/self-test');
    expect(selfInit.method).toBe('POST');
  });

  it('parseCoreStatus memetakan startupState, assetState, dan loadError', () => {
    expect(parseCoreStatus({ startupState: 'error', assetState: 'ready', loadError: 'gagal muat' }))
      .toEqual({ startupState: 'error', assetState: 'ready', loadError: 'gagal muat' });
    expect(parseCoreStatus({})).toEqual({ startupState: null, assetState: null, loadError: null });
  });

  it('fungsi plugin bootstrap menolak dengan jujur di browser biasa', async () => {
    await expect(startCore()).rejects.toThrow('hanya tersedia di aplikasi Android');
    await expect(getCoreStatus()).rejects.toThrow('hanya tersedia di aplikasi Android');
    await expect(getStartupLog()).rejects.toThrow('hanya tersedia di aplikasi Android');
  });
});
