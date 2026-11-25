import { useQuery } from '@tanstack/react-query';
import { CartesianGrid, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';
import { api } from '../lib/api';
import { formatBytes, formatDateTime, formatPercent, formatUptime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, Skeleton, SkeletonRows } from '../components/ui';

export function Monitoring() {
  const current = useQuery({ queryKey: ['monitoring-current'], queryFn: api.monitoringCurrent, refetchInterval: 5000 });
  const history = useQuery({ queryKey: ['monitoring-history'], queryFn: () => api.monitoringHistory(60), refetchInterval: 15000 });

  const chartData = (history.data ?? []).map((p) => ({
    time: new Date(p.recorded_at).toLocaleTimeString('id-ID', { hour: '2-digit', minute: '2-digit' }),
    cpu: p.cpu_percent,
    memMB: p.mem_used_bytes !== null ? Math.round(p.mem_used_bytes / (1024 * 1024)) : null,
  }));

  const cur = current.data;

  return (
    <div className="space-y-4">
      <Card title="Kondisi saat ini">
        {current.isLoading ? <SkeletonRows rows={2} /> : current.isError ? <ErrorBox message="Gagal memuat monitoring." /> : (
          <>
            <div className="grid grid-cols-2 gap-3 text-sm">
              <div><p className="text-xs text-slate-600 dark:text-slate-400">CPU</p><p className="font-semibold">{formatPercent(cur?.cpu_percent)}</p></div>
              <div><p className="text-xs text-slate-600 dark:text-slate-400">Memori terpakai</p><p className="font-semibold">{formatBytes(cur?.mem_used_bytes)} / {formatBytes(cur?.mem_total_bytes)}</p></div>
              <div><p className="text-xs text-slate-600 dark:text-slate-400">Penyimpanan terpakai</p><p className="font-semibold">{formatBytes(cur?.storage_used_bytes)} / {formatBytes(cur?.storage_total_bytes)}</p></div>
              <div><p className="text-xs text-slate-600 dark:text-slate-400">Port LISTEN</p><p className="font-semibold">{cur?.ports.length ?? '-'}</p></div>
            </div>
            {cur?.unavailable_reason && <p className="text-xs text-warning mt-2">{cur.unavailable_reason}</p>}
          </>
        )}
      </Card>

      <Card title="Riwayat (CPU % dan memori MB)">
        {history.isLoading ? <Skeleton className="h-64 w-full" /> : history.isError ? <ErrorBox message="Gagal memuat riwayat monitoring." /> : chartData.length === 0 ? (
          <EmptyBox message="Belum ada sampel riwayat. Sampler mencatat tiap 30 detik." />
        ) : (
          <div className="h-64">
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" />
                <XAxis dataKey="time" tick={{ fontSize: 11 }} />
                <YAxis tick={{ fontSize: 11 }} />
                <Tooltip />
                <Legend />
                <Line type="monotone" dataKey="cpu" name="CPU %" stroke="#5576E8" dot={false} connectNulls />
                <Line type="monotone" dataKey="memMB" name="Memori MB" stroke="#2EAD79" dot={false} connectNulls />
              </LineChart>
            </ResponsiveContainer>
          </div>
        )}
        {(history.data?.length ?? 0) > 0 && (
          <p className="text-xs text-slate-600 dark:text-slate-400 mt-1">Sampel terakhir: {formatDateTime(history.data![history.data!.length - 1].recorded_at)}</p>
        )}
      </Card>

      <Card title="Proses server">
        {current.isLoading ? <SkeletonRows rows={2} /> : (cur?.processes.length ?? 0) === 0 ? (
          <EmptyBox message="Tidak ada proses server yang tercatat." />
        ) : (
          <div className="space-y-1 text-sm">
            {cur!.processes.map((p) => (
              <p key={p.pid} className="font-mono text-xs">PID {p.pid} · {p.name} · uptime {formatUptime(p.uptime_seconds)}{p.server_id ? ` · server ${p.server_id.slice(0, 8)}…` : ''}</p>
            ))}
          </div>
        )}
      </Card>

      <Card title="Port yang mendengarkan (LISTEN)">
        {current.isLoading ? <SkeletonRows rows={1} /> : (cur?.ports.length ?? 0) === 0 ? (
          <EmptyBox message="Tidak ada port LISTEN yang terdeteksi." />
        ) : (
          <div className="flex flex-wrap gap-1">
            {cur!.ports.map((p) => (
              <span key={p.port} className="rounded-full bg-slate-100 dark:bg-darkbg px-2 py-0.5 text-xs font-mono">{p.port} · {p.state}</span>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
