import type { RuntimeInfo } from '../lib/types';

export function RuntimeBadge({ runtime }: { runtime: RuntimeInfo }) {
  if (!runtime.available) {
    return (
      <div className="flex items-start justify-between gap-3 rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2">
        <div>
          <p className="text-sm font-medium text-ink dark:text-darkink">{runtime.name}</p>
          {runtime.reason && <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">{runtime.reason}</p>}
        </div>
        <span className="shrink-0 rounded-full bg-warning/15 text-warning px-2 py-0.5 text-xs font-medium">Belum tersedia</span>
      </div>
    );
  }
  return (
    <div className="flex items-start justify-between gap-3 rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2">
      <div>
        <p className="text-sm font-medium text-ink dark:text-darkink">{runtime.name}</p>
        <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
          {runtime.version ? `Versi ${runtime.version}` : 'Terdeteksi'}
          {runtime.source ? ` · sumber: ${runtime.source}` : ''}
        </p>
      </div>
      <span className="shrink-0 rounded-full bg-success/15 text-success px-2 py-0.5 text-xs font-medium">Tersedia</span>
    </div>
  );
}
