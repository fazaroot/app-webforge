import { WifiOff } from 'lucide-react';

export function OfflineBanner({ onRetry, retrying }: { onRetry: () => void; retrying?: boolean }) {
  return (
    <div className="rounded-xl bg-danger text-white px-4 py-3 flex items-center justify-between gap-3" role="alert">
      <div className="flex items-center gap-2">
        <WifiOff size={18} />
        <div>
          <p className="font-semibold text-sm">Core offline</p>
          <p className="text-xs opacity-90">Tidak dapat terhubung ke WebForge Core. Data tidak ditampilkan sampai koneksi pulih.</p>
        </div>
      </div>
      <button type="button" onClick={onRetry} disabled={retrying} className="shrink-0 rounded-lg bg-white text-danger px-3 py-1.5 text-sm font-medium disabled:opacity-60">
        {retrying ? 'Mencoba…' : 'Coba lagi'}
      </button>
    </div>
  );
}
