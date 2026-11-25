import { useMutation, useQuery } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { api, getCoreStatus, getStartupLog, isNativeWebForge } from '../lib/api';
import type { SelfTestEngineResult } from '../lib/types';
import { Card, EmptyBox, ErrorBox, SkeletonRows, btnGhost, btnPrimary } from './ui';

/** Hasil Uji Semua Engine per engine: lolos/gagal + detail/error persis dari core. */
export function SelfTestResults({ results }: { results: SelfTestEngineResult[] }) {
  if (results.length === 0) return <EmptyBox message="Core tidak mengembalikan hasil engine apa pun." />;
  return (
    <div className="space-y-2">
      {results.map((r) => (
        <div key={r.engine} className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2">
          <div className="flex items-start justify-between gap-2">
            <p className="text-sm font-medium">{r.engine}</p>
            <span className={`shrink-0 rounded-full px-2 py-0.5 text-xs font-medium ${r.ok ? 'bg-success/15 text-success' : 'bg-danger/15 text-danger'}`}>
              {r.ok ? 'Lolos' : 'Gagal'}
            </span>
          </div>
          <p className="text-xs text-slate-600 dark:text-slate-400 mt-0.5">Durasi: {r.duration_ms} ms</p>
          {r.detail && <p className="text-xs mt-1 break-all">{r.detail}</p>}
          {r.error && <p className="text-xs text-danger mt-1 break-all">Error: {r.error}</p>}
        </div>
      ))}
    </div>
  );
}

/** Tombol "Uji Semua Engine": self-test nyata di core (start/HTTP/stop per engine). */
export function SelfTestPanel() {
  const selfTest = useMutation({ mutationFn: () => api.selfTest() });
  return (
    <Card title="Uji engine">
      <p className="text-xs text-slate-600 dark:text-slate-400 mb-3">
        Menjalankan self-test nyata di core untuk semua engine. Hasil per engine ditampilkan apa adanya.
      </p>
      <button type="button" className={btnPrimary} disabled={selfTest.isPending} onClick={() => selfTest.mutate()}>
        {selfTest.isPending ? 'Menguji…' : 'Uji Semua Engine'}
      </button>
      <div className="mt-3">
        {selfTest.isPending ? (
          <div>
            <p className="text-sm text-slate-600 dark:text-slate-400 mb-2">Menguji semua engine…</p>
            <SkeletonRows rows={3} />
          </div>
        ) : selfTest.isError ? (
          <ErrorBox message={selfTest.error instanceof Error ? selfTest.error.message : 'Self-test gagal dijalankan.'} />
        ) : selfTest.data ? (
          <SelfTestResults results={selfTest.data.results} />
        ) : null}
      </div>
    </Card>
  );
}

/**
 * Log startup core dari plugin native, beserta startupState/assetState.
 * Di web non-native ditampilkan catatan jujur bahwa log ini tidak tersedia.
 */
export function StartupLogPanel() {
  const native = isNativeWebForge();
  const status = useQuery({ queryKey: ['core-status'], queryFn: getCoreStatus, enabled: native, retry: false });
  const log = useQuery({ queryKey: ['startup-log'], queryFn: getStartupLog, enabled: native, retry: false });

  if (!native) {
    return (
      <Card title="Log Startup Core">
        <p className="text-sm text-slate-600 dark:text-slate-400">
          Log startup core hanya tersedia di aplikasi Android. Di browser, core tidak berjalan
          di perangkat ini sehingga log startup native tidak tersedia.
        </p>
      </Card>
    );
  }

  return (
    <Card title="Log Startup Core" right={
      <button type="button" className={btnGhost} onClick={() => { void status.refetch(); void log.refetch(); }}>
        <RefreshCw size={14} /> Muat ulang
      </button>
    }>
      <div className="text-sm space-y-1 mb-3">
        <p>Status startup: <span className="font-medium">{status.data?.startupState ?? (status.isLoading ? 'memuat…' : 'tidak diketahui')}</span></p>
        <p>Status aset: <span className="font-medium break-all">{status.data?.assetState ?? (status.isLoading ? 'memuat…' : 'tidak diketahui')}</span></p>
        {status.data?.loadError && <p className="text-danger break-all">Error pemuatan: {status.data.loadError}</p>}
        {status.isError && <ErrorBox message={status.error instanceof Error ? status.error.message : 'Gagal memuat status startup core.'} />}
      </div>
      {log.isLoading ? <SkeletonRows rows={3} /> : log.isError ? (
        <ErrorBox message={log.error instanceof Error ? log.error.message : 'Gagal memuat log startup core.'} />
      ) : (log.data?.length ?? 0) === 0 ? (
        <EmptyBox message="Belum ada entri log startup." />
      ) : (
        <div className="space-y-1 max-h-64 overflow-auto">
          {log.data!.map((e, i) => (
            <p key={`${e.ts}-${i}`} className="text-xs font-mono break-all">
              <span className="text-slate-600 dark:text-slate-400">{e.ts}</span> {e.message}
            </p>
          ))}
        </div>
      )}
    </Card>
  );
}
