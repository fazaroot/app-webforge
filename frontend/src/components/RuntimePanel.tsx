import { useState, type ReactNode } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { api } from '../lib/api';
import type { RuntimeInfo, RuntimeTestResult } from '../lib/types';
import { RuntimeBadge } from './RuntimeBadge';
import { ErrorBox, btnGhost } from './ui';

/** Panel hasil Tes Runtime: stdout/stderr/exit code/durasi apa adanya dari core. */
export function RuntimeTestResultPanel({ result }: { result: RuntimeTestResult }) {
  return (
    <div className="rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-darkbg px-3 py-2 space-y-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
        <span className={`rounded-full px-2 py-0.5 font-medium ${result.ok ? 'bg-success/15 text-success' : 'bg-danger/15 text-danger'}`}>
          {result.ok ? 'Tes lolos' : 'Tes gagal'}
        </span>
        {result.version && <span>Versi: {result.version}</span>}
        <span>Exit code: {result.exit_code ?? '—'}</span>
        <span>Durasi: {result.duration_ms} ms</span>
      </div>
      {result.error && <p className="text-xs text-danger break-all">Error: {result.error}</p>}
      <div>
        <p className="text-xs font-medium text-slate-600 dark:text-slate-400">Stdout</p>
        <pre className="text-xs font-mono whitespace-pre-wrap break-all rounded bg-white dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-2 max-h-40 overflow-auto">{result.stdout || '(kosong)'}</pre>
      </div>
      <div>
        <p className="text-xs font-medium text-slate-600 dark:text-slate-400">Stderr</p>
        <pre className="text-xs font-mono whitespace-pre-wrap break-all rounded bg-white dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-2 max-h-40 overflow-auto">{result.stderr || '(kosong)'}</pre>
      </div>
    </div>
  );
}

/** Status kartu runtime diperbarui dari hasil tes segar. */
function applyTestResult(runtime: RuntimeInfo, result: RuntimeTestResult): RuntimeInfo {
  return {
    ...runtime,
    available: result.ok,
    version: result.version ?? runtime.version,
    reason: result.ok ? null : result.error ?? runtime.reason,
  };
}

/**
 * Daftar runtime dengan Tes Runtime per runtime dan probe ulang (?fresh=1).
 * renderAction menambahkan aksi tambahan per runtime (mis. "Pakai engine ini").
 */
export function RuntimePanel({ runtimes, renderAction }: {
  runtimes: RuntimeInfo[];
  renderAction?: (runtime: RuntimeInfo) => ReactNode;
}) {
  const qc = useQueryClient();
  const [results, setResults] = useState<Record<string, RuntimeTestResult>>({});
  const [testErrors, setTestErrors] = useState<Record<string, string>>({});

  const test = useMutation({
    mutationFn: (id: string) => api.testRuntime(id),
    onSuccess: (result) => {
      setResults((prev) => ({ ...prev, [result.id]: result }));
      setTestErrors((prev) => {
        const next = { ...prev };
        delete next[result.id];
        return next;
      });
      qc.setQueryData<RuntimeInfo[]>(['runtimes'], (prev) =>
        prev?.map((r) => (r.id === result.id ? applyTestResult(r, result) : r)));
    },
    onError: (e, id) => {
      setTestErrors((prev) => ({ ...prev, [id]: e instanceof Error ? e.message : 'Tes runtime gagal dijalankan' }));
    },
  });

  const refreshProbe = useMutation({
    mutationFn: () => api.runtimes(true),
    onSuccess: (data) => {
      qc.setQueryData<RuntimeInfo[]>(['runtimes'], data);
      setResults({});
      setTestErrors({});
    },
  });

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs text-slate-600 dark:text-slate-400">
          Status dari probe core (cache). Tes Runtime menjalankan probe segar untuk satu runtime.
        </p>
        <button type="button" className={btnGhost} disabled={refreshProbe.isPending} onClick={() => refreshProbe.mutate()}>
          <RefreshCw size={14} /> {refreshProbe.isPending ? 'Memprobe…' : 'Probe ulang semua'}
        </button>
      </div>
      {refreshProbe.isError && (
        <ErrorBox message={refreshProbe.error instanceof Error ? refreshProbe.error.message : 'Probe ulang gagal.'} />
      )}
      {runtimes.map((r) => {
        const result = results[r.id];
        const effective = result ? applyTestResult(r, result) : r;
        const testing = test.isPending && test.variables === r.id;
        return (
          <div key={r.id} className="space-y-1">
            <RuntimeBadge runtime={effective} />
            <div className="flex flex-wrap justify-end gap-2">
              {renderAction?.(effective)}
              <button type="button" className={btnGhost} disabled={testing} onClick={() => test.mutate(r.id)}>
                {testing ? 'Menguji…' : 'Tes Runtime'}
              </button>
            </div>
            {testErrors[r.id] && <ErrorBox message={testErrors[r.id]} />}
            {result && <RuntimeTestResultPanel result={result} />}
          </div>
        );
      })}
    </div>
  );
}
