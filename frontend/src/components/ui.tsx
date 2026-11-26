import type { ReactNode } from 'react';

export function Card({ title, children, right }: { title?: string; children: ReactNode; right?: ReactNode }) {
  return (
    <section className="rounded-xl bg-surface dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-4 shadow-sm shadow-slate-900/5 dark:shadow-black/20">
      {(title || right) && (
        <div className="flex items-center justify-between gap-2 mb-3">
          {title ? <h2 className="font-semibold text-ink dark:text-darkink">{title}</h2> : <span />}
          {right}
        </div>
      )}
      {children}
    </section>
  );
}

export function ErrorBox({ message }: { message: string }) {
  return (
    <div className="rounded-lg bg-red-50 dark:bg-red-950 border border-danger/40 text-danger px-3 py-2 text-sm" role="alert">
      {message}
    </div>
  );
}

export function EmptyBox({ message }: { message: string }) {
  return <p className="text-sm text-slate-600 dark:text-slate-400 py-4 text-center">{message}</p>;
}

export function LoadingBox({ message = 'Memuat…' }: { message?: string }) {
  return <p className="text-sm text-slate-600 dark:text-slate-400 py-4 text-center">{message}</p>;
}

/** Blok skeleton tunggal dengan animasi shimmer (dimatikan bila prefers-reduced-motion). */
export function Skeleton({ className = '' }: { className?: string }) {
  return (
    <div
      data-testid="skeleton"
      aria-hidden="true"
      className={`skeleton-shimmer rounded-md bg-slate-200 dark:bg-slate-700/60 ${className}`}
    />
  );
}

/** Skeleton menyerupai kartu: judul + beberapa baris teks. */
export function SkeletonCard({ lines = 3 }: { lines?: number }) {
  return (
    <div role="status" aria-label="Memuat" className="rounded-xl bg-surface dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-4 shadow-sm shadow-slate-900/5 dark:shadow-black/20 space-y-2">
      <Skeleton className="h-4 w-1/3" />
      {Array.from({ length: lines }, (_, i) => (
        <Skeleton key={i} className={`h-3 ${i === lines - 1 ? 'w-2/3' : 'w-full'}`} />
      ))}
    </div>
  );
}

/** Skeleton menyerupai daftar baris (mis. daftar proyek/server/database). */
export function SkeletonRows({ rows = 3 }: { rows?: number }) {
  return (
    <div role="status" aria-label="Memuat" className="space-y-2">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2 space-y-2">
          <Skeleton className="h-3.5 w-1/2" />
          <Skeleton className="h-3 w-3/4" />
        </div>
      ))}
    </div>
  );
}

export const inputCls =
  'w-full rounded-lg border border-slate-300 dark:border-slate-600 bg-white dark:bg-darkbg px-3 py-2 text-sm text-ink dark:text-darkink outline-none focus:border-primary';
export const btnPrimary =
  'inline-flex items-center gap-1.5 rounded-lg bg-primary text-white px-3 py-2 text-sm font-medium disabled:opacity-50';
export const btnGhost =
  'inline-flex items-center gap-1.5 rounded-lg border border-slate-300 dark:border-slate-600 px-3 py-2 text-sm text-ink dark:text-darkink disabled:opacity-50';
export const btnDanger =
  'inline-flex items-center gap-1.5 rounded-lg bg-danger text-white px-3 py-2 text-sm font-medium disabled:opacity-50';

export function StatusPill({ status }: { status: string }) {
  const map: Record<string, string> = {
    running: 'bg-success/15 text-success',
    degraded: 'bg-warning/15 text-warning',
    stopped: 'bg-slate-200 dark:bg-slate-700 text-slate-600 dark:text-slate-300',
    starting: 'bg-warning/15 text-warning',
    stopping: 'bg-warning/15 text-warning',
    error: 'bg-danger/15 text-danger',
    ok: 'bg-success/15 text-success',
    warn: 'bg-warning/15 text-warning',
    ready: 'bg-success/15 text-success',
    connected: 'bg-success/15 text-success',
    disconnected: 'bg-slate-200 dark:bg-slate-700 text-slate-600 dark:text-slate-300',
    available: 'bg-success/15 text-success',
    unavailable: 'bg-warning/15 text-warning',
  };
  const label: Record<string, string> = {
    running: 'Berjalan', degraded: 'Degraded', stopped: 'Berhenti', starting: 'Memulai…', stopping: 'Menghentikan…', error: 'Error',
    ready: 'Siap', connected: 'Terhubung', disconnected: 'Terputus', available: 'Tersedia', unavailable: 'Tidak tersedia',
  };
  return (
    <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${map[status] ?? 'bg-slate-200 dark:bg-slate-700 text-slate-600 dark:text-slate-300'}`}>
      {label[status] ?? status}
    </span>
  );
}
