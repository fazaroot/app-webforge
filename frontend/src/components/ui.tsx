import type { ReactNode } from 'react';

export function Card({ title, children, right }: { title?: string; children: ReactNode; right?: ReactNode }) {
  return (
    <section className="wf-glass-card rounded-xl p-4">
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
    <div className="rounded-lg bg-red-50/90 dark:bg-red-950/70 backdrop-blur border border-danger/50 text-danger px-3 py-2 text-sm" role="alert">
      {message}
    </div>
  );
}

export function EmptyBox({ message }: { message: string }) {
  return <p className="text-sm text-slate-700 dark:text-slate-300 py-4 text-center">{message}</p>;
}

export function LoadingBox({ message = 'Memuat…' }: { message?: string }) {
  return <p className="text-sm text-slate-700 dark:text-slate-300 py-4 text-center">{message}</p>;
}

/** Blok skeleton tunggal dengan animasi shimmer (dimatikan bila prefers-reduced-motion). */
export function Skeleton({ className = '' }: { className?: string }) {
  return (
    <div
      data-testid="skeleton"
      aria-hidden="true"
      className={`skeleton-shimmer rounded-md bg-sky-200/70 dark:bg-cyan-900/50 ${className}`}
    />
  );
}

/** Skeleton menyerupai kartu: judul + beberapa baris teks. */
export function SkeletonCard({ lines = 3 }: { lines?: number }) {
  return (
    <div role="status" aria-label="Memuat" className="wf-glass-card rounded-xl p-4 space-y-2">
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
        <div key={i} className="wf-glass-panel rounded-lg px-3 py-2 space-y-2">
          <Skeleton className="h-3.5 w-1/2" />
          <Skeleton className="h-3 w-3/4" />
        </div>
      ))}
    </div>
  );
}

export const inputCls =
  'w-full rounded-lg border border-sky-300/80 dark:border-cyan-400/35 bg-white/85 dark:bg-slate-950/45 backdrop-blur px-3 py-2 text-sm text-ink dark:text-darkink outline-none focus:border-primary focus:ring-2 focus:ring-cyan-400/35 disabled:bg-slate-200 disabled:text-slate-600 disabled:border-slate-400 dark:disabled:bg-slate-800 dark:disabled:text-slate-300 dark:disabled:border-slate-600 disabled:cursor-not-allowed';
export const btnPrimary =
  'inline-flex items-center gap-1.5 rounded-lg border border-cyan-300/50 bg-primary text-white px-3 py-2 text-sm font-medium shadow shadow-cyan-600/25 disabled:bg-slate-300 disabled:text-slate-800 disabled:border-slate-500 disabled:shadow-none disabled:cursor-not-allowed dark:disabled:bg-slate-700 dark:disabled:text-slate-100 dark:disabled:border-slate-400';
export const btnGhost =
  'inline-flex items-center gap-1.5 rounded-lg border border-sky-300/80 dark:border-cyan-400/35 bg-white/75 dark:bg-slate-950/40 backdrop-blur px-3 py-2 text-sm text-ink dark:text-darkink disabled:bg-slate-200 disabled:text-slate-700 disabled:border-slate-400 disabled:cursor-not-allowed dark:disabled:bg-slate-800 dark:disabled:text-slate-200 dark:disabled:border-slate-500';
export const btnDanger =
  'inline-flex items-center gap-1.5 rounded-lg border border-red-300/60 bg-danger text-white px-3 py-2 text-sm font-medium shadow shadow-red-600/20 disabled:bg-slate-300 disabled:text-slate-800 disabled:border-slate-500 disabled:shadow-none disabled:cursor-not-allowed dark:disabled:bg-slate-700 dark:disabled:text-slate-100 dark:disabled:border-slate-400';

export function StatusPill({ status }: { status: string }) {
  const map: Record<string, string> = {
    running: 'bg-success/15 text-success border border-success/40',
    degraded: 'bg-warning/15 text-warning border border-warning/40',
    stopped: 'bg-slate-200/80 dark:bg-slate-700/80 text-slate-700 dark:text-slate-200 border border-slate-300 dark:border-slate-600',
    starting: 'bg-warning/15 text-warning border border-warning/40',
    stopping: 'bg-warning/15 text-warning border border-warning/40',
    error: 'bg-danger/15 text-danger border border-danger/40',
    ok: 'bg-success/15 text-success border border-success/40',
    warn: 'bg-warning/15 text-warning border border-warning/40',
    ready: 'bg-success/15 text-success border border-success/40',
    connected: 'bg-success/15 text-success border border-success/40',
    disconnected: 'bg-slate-200/80 dark:bg-slate-700/80 text-slate-700 dark:text-slate-200 border border-slate-300 dark:border-slate-600',
    available: 'bg-success/15 text-success border border-success/40',
    unavailable: 'bg-warning/15 text-warning border border-warning/40',
  };
  const label: Record<string, string> = {
    running: 'Berjalan', degraded: 'Degraded', stopped: 'Berhenti', starting: 'Memulai…', stopping: 'Menghentikan…', error: 'Error',
    ready: 'Siap', connected: 'Terhubung', disconnected: 'Terputus', available: 'Tersedia', unavailable: 'Tidak tersedia',
  };
  return (
    <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${map[status] ?? 'bg-slate-200/80 dark:bg-slate-700/80 text-slate-700 dark:text-slate-200 border border-slate-300 dark:border-slate-600'}`}>
      {label[status] ?? status}
    </span>
  );
}
