import type { ReactNode } from 'react';

export function Card({ title, children, right }: { title?: string; children: ReactNode; right?: ReactNode }) {
  return (
    <section className="rounded-xl bg-surface dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-4 shadow-sm">
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
  return <p className="text-sm text-slate-500 dark:text-slate-400 py-4 text-center">{message}</p>;
}

export function LoadingBox({ message = 'Memuat…' }: { message?: string }) {
  return <p className="text-sm text-slate-500 dark:text-slate-400 py-4 text-center">{message}</p>;
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
    stopped: 'bg-slate-200 dark:bg-slate-700 text-slate-600 dark:text-slate-300',
    starting: 'bg-warning/15 text-warning',
    stopping: 'bg-warning/15 text-warning',
    error: 'bg-danger/15 text-danger',
    ok: 'bg-success/15 text-success',
    warn: 'bg-warning/15 text-warning',
  };
  const label: Record<string, string> = {
    running: 'Berjalan', stopped: 'Berhenti', starting: 'Memulai…', stopping: 'Menghentikan…', error: 'Error',
  };
  return (
    <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${map[status] ?? 'bg-slate-200 text-slate-600'}`}>
      {label[status] ?? status}
    </span>
  );
}
