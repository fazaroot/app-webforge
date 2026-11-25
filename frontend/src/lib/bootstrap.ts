import type { CoreStatus } from './types';

/** Batas total polling bootstrap core saat aplikasi native dibuka. */
export const BOOTSTRAP_MAX_MS = 30000;
export const BOOTSTRAP_FIRST_DELAY_MS = 500;
export const BOOTSTRAP_MAX_DELAY_MS = 2000;

/**
 * Jeda sebelum percobaan polling berikutnya (fungsi murni):
 * mulai 500ms, berlipat dua tiap percobaan, dibatasi 2000ms.
 */
export function bootstrapBackoffMs(attempt: number): number {
  if (attempt <= 0) return BOOTSTRAP_FIRST_DELAY_MS;
  return Math.min(BOOTSTRAP_FIRST_DELAY_MS * 2 ** attempt, BOOTSTRAP_MAX_DELAY_MS);
}

/**
 * Jadwal jeda antar-percobaan polling (fungsi murni). Jumlah seluruh jeda
 * tidak melebihi maxMs, sehingga total polling berhenti di sekitar maxMs.
 */
export function bootstrapSchedule(maxMs: number = BOOTSTRAP_MAX_MS): number[] {
  const delays: number[] = [];
  let elapsed = 0;
  for (let attempt = 0; ; attempt += 1) {
    const delay = bootstrapBackoffMs(attempt);
    if (elapsed + delay > maxMs) break;
    delays.push(delay);
    elapsed += delay;
  }
  return delays;
}

export interface CoreBootstrapDeps {
  startCore: () => Promise<unknown>;
  health: () => Promise<unknown>;
  getStatus?: () => Promise<CoreStatus | null>;
  /** Dipanggil sebelum tiap percobaan health agar base/token plugin dibaca ulang. */
  resetConfig?: () => void;
  sleep?: (ms: number) => Promise<void>;
  maxMs?: number;
}

export interface CoreBootstrapResult {
  ok: boolean;
  /** loadError nyata dari plugin getStatus bila ada. */
  loadError: string | null;
  /** Pesan error terakhir (loadError, error startCore, atau error health). */
  error: string | null;
  attempts: number;
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const defaultSleep = (ms: number) => new Promise<void>((resolve) => { setTimeout(resolve, ms); });

/**
 * Bootstrap core: startCore() lalu polling status/health dengan backoff
 * sampai core sehat atau jadwal habis. Bila plugin melaporkan
 * startupState "error", berhenti lebih awal dengan loadError apa adanya.
 */
export async function runCoreBootstrap(deps: CoreBootstrapDeps): Promise<CoreBootstrapResult> {
  const sleep = deps.sleep ?? defaultSleep;
  const delays = bootstrapSchedule(deps.maxMs ?? BOOTSTRAP_MAX_MS);
  let loadError: string | null = null;
  let lastError: string | null = null;

  try {
    await deps.startCore();
  } catch (e) {
    // Core mungkin sudah berjalan; tetap polling sebelum menyerah.
    lastError = errText(e);
  }

  let attempts = 0;
  for (let i = 0; i <= delays.length; i += 1) {
    attempts += 1;
    deps.resetConfig?.();

    if (deps.getStatus) {
      try {
        const status = await deps.getStatus();
        if (status?.loadError) loadError = status.loadError;
        if (status?.startupState === 'error') {
          return { ok: false, loadError, error: loadError ?? lastError, attempts };
        }
      } catch {
        // Status plugin belum terbaca; health tetap dicoba.
      }
    }

    try {
      await deps.health();
      return { ok: true, loadError: null, error: null, attempts };
    } catch (e) {
      lastError = errText(e);
    }

    if (i < delays.length) await sleep(delays[i]);
  }

  return { ok: false, loadError, error: loadError ?? lastError, attempts };
}
