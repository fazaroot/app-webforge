import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Download } from 'lucide-react';
import { api } from '../lib/api';
import { formatDateTime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, LoadingBox, btnGhost, inputCls } from '../components/ui';

export function Logs() {
  const [severity, setSeverity] = useState('');
  const [search, setSearch] = useState('');
  const [applied, setApplied] = useState({ severity: '', search: '' });
  const [exportError, setExportError] = useState('');

  const logs = useQuery({
    queryKey: ['logs', applied.severity, applied.search],
    queryFn: () => api.logs(applied.severity, applied.search, 200),
  });

  async function exportLogs() {
    try {
      const text = await api.logsExport();
      const blob = new Blob([text], { type: 'text/plain' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'webforge-logs.txt';
      a.click();
      URL.revokeObjectURL(url);
      setExportError('');
    } catch (e) {
      setExportError(e instanceof Error ? e.message : 'Ekspor gagal');
    }
  }

  return (
    <div className="space-y-4">
      {exportError && <ErrorBox message={exportError} />}
      <Card title="Log aplikasi" right={
        <button type="button" className={btnGhost} onClick={() => void exportLogs()}><Download size={14} /> Ekspor</button>
      }>
        <div className="flex flex-wrap gap-2 mb-3">
          <select className={`${inputCls} w-auto`} value={severity} onChange={(e) => setSeverity(e.target.value)}>
            <option value="">Semua severity</option>
            <option value="debug">debug</option>
            <option value="info">info</option>
            <option value="warn">warn</option>
            <option value="error">error</option>
          </select>
          <input className={`${inputCls} flex-1 min-w-[140px]`} placeholder="Cari pesan…" value={search} onChange={(e) => setSearch(e.target.value)} />
          <button type="button" className={btnGhost} onClick={() => setApplied({ severity, search })}>Terapkan</button>
        </div>
        {logs.isLoading ? <LoadingBox /> : logs.isError ? <ErrorBox message="Gagal memuat log." /> : (logs.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Tidak ada log yang cocok dengan filter." />
        ) : (
          <div className="space-y-1 max-h-[60vh] overflow-auto">
            {logs.data!.map((l) => (
              <p key={l.id} className="text-xs font-mono">
                <span className="text-slate-600 dark:text-slate-400">{formatDateTime(l.created_at)}</span>{' '}
                <span className={l.severity === 'error' ? 'text-danger' : l.severity === 'warn' ? 'text-warning' : 'text-slate-600 dark:text-slate-400'}>[{l.severity}]</span>{' '}
                <span className="text-slate-600 dark:text-slate-400">({l.category})</span> {l.message}
              </p>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
