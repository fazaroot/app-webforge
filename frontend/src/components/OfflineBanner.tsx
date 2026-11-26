import { WifiOff } from 'lucide-react';

export function OfflineBanner({ onRetry, retrying, loadError }: {
  onRetry: () => void;
  retrying?: boolean;
  /** Error nyata dari plugin/core (mis. loadError getStatus), ditampilkan apa adanya. */
  loadError?: string | null;
}) {
  return (
    <div className="rounded-xl bg-danger text-white px-4 py-3 flex items-center justify-between gap-3" role="alert">
      <div className="flex items-center gap-2">
        <WifiOff size={18} />
        <div>
          <p className="font-semibold text-sm">Core offline</p>
          <p className="text-xs opacity-90">Tidak dapat terhubung ke WebForge Core. Data tidak ditampilkan sampai koneksi pulih.</p>
          {loadError && <p className="text-xs mt-1 break-all">Penyebab: {loadError}</p>}
        </div>
      </div>
      <button type="button" onClick={onRetry} disabled={retrying} className="shrink-0 rounded-lg border border-white/70 bg-white text-danger px-3 py-1.5 text-sm font-medium disabled:bg-slate-300 disabled:text-slate-800 disabled:border-slate-100 disabled:cursor-not-allowed">
        {retrying ? 'Mencoba…' : 'Coba lagi'}
      </button>
    </div>
  );
}
