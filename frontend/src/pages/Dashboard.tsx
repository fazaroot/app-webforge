import { useQuery } from '@tanstack/react-query';
import { Activity, FolderGit2, Server } from 'lucide-react';
import { api } from '../lib/api';
import { formatBytes, formatPercent, formatUptime } from '../lib/format';
import { Card, ErrorBox, Skeleton, SkeletonCard, SkeletonRows } from '../components/ui';
import { RuntimeBadge } from '../components/RuntimeBadge';
import { useAppStore } from '../store';

export function Dashboard() {
  const setSection = useAppStore((s) => s.setSection);
  const health = useQuery({ queryKey: ['health'], queryFn: api.health, refetchInterval: 10000 });
  const info = useQuery({ queryKey: ['system-info'], queryFn: api.systemInfo, enabled: health.isSuccess });
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.projects, enabled: health.isSuccess });
  const servers = useQuery({ queryKey: ['servers'], queryFn: api.servers, enabled: health.isSuccess, refetchInterval: 5000 });
  const monitoring = useQuery({ queryKey: ['monitoring-current'], queryFn: api.monitoringCurrent, enabled: health.isSuccess, refetchInterval: 5000 });
  const network = useQuery({ queryKey: ['network'], queryFn: api.network, enabled: health.isSuccess });

  if (health.isLoading) {
    return (
      <div className="space-y-4">
        <SkeletonCard lines={4} />
        <div className="grid grid-cols-2 gap-3">
          <div className="rounded-xl bg-surface dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-4 shadow-sm space-y-2">
            <Skeleton className="h-5 w-6" />
            <Skeleton className="h-6 w-1/2" />
            <Skeleton className="h-3 w-2/3" />
          </div>
          <div className="rounded-xl bg-surface dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-4 shadow-sm space-y-2">
            <Skeleton className="h-5 w-6" />
            <Skeleton className="h-6 w-1/2" />
            <Skeleton className="h-3 w-2/3" />
          </div>
        </div>
        <SkeletonCard lines={2} />
        <SkeletonCard lines={3} />
      </div>
    );
  }
  if (health.isError) return <ErrorBox message="Core offline — ringkasan tidak dapat dimuat. Periksa koneksi lalu coba lagi." />;

  const running = (servers.data ?? []).filter((s) => s.status === 'running').length;
  const mon = monitoring.data;

  return (
    <div className="space-y-4">
      <Card title="Status Core">
        <div className="grid grid-cols-2 gap-3 text-sm">
          <div><p className="text-slate-600 dark:text-slate-400 text-xs">Status</p><p className="font-semibold text-success">{health.data?.status === 'ok' ? 'Online' : health.data?.status}</p></div>
          <div><p className="text-slate-600 dark:text-slate-400 text-xs">Versi</p><p className="font-semibold">{health.data?.version}</p></div>
          <div><p className="text-slate-600 dark:text-slate-400 text-xs">Uptime</p><p className="font-semibold">{formatUptime(health.data?.uptime_seconds)}</p></div>
          <div><p className="text-slate-600 dark:text-slate-400 text-xs">Port manajemen</p><p className="font-semibold">{info.data?.management_port ?? '-'}</p></div>
          <div><p className="text-slate-600 dark:text-slate-400 text-xs">Sistem</p><p className="font-semibold">{info.data ? `${info.data.os} / ${info.data.arch}` : '-'}</p></div>
          <div><p className="text-slate-600 dark:text-slate-400 text-xs">IP LAN</p><p className="font-semibold">{network.data?.lan_ip ?? 'Tidak terdeteksi'}</p></div>
        </div>
        {info.data && <p className="text-xs text-slate-600 dark:text-slate-400 mt-3 break-all">Workspace: {info.data.workspace_dir}</p>}
      </Card>

      <div className="grid grid-cols-2 gap-3">
        <button type="button" onClick={() => setSection('projects')} className="rounded-xl bg-surface dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-4 text-left shadow-sm">
          <FolderGit2 className="text-primary" size={22} />
          <p className="text-2xl font-bold mt-2">{projects.data?.length ?? '-'}</p>
          <p className="text-xs text-slate-600 dark:text-slate-400">Proyek terdaftar</p>
        </button>
        <button type="button" onClick={() => setSection('servers')} className="rounded-xl bg-surface dark:bg-darksurface border border-slate-200 dark:border-slate-700 p-4 text-left shadow-sm">
          <Server className="text-primary" size={22} />
          <p className="text-2xl font-bold mt-2">{running}<span className="text-sm text-slate-600 dark:text-slate-400"> / {servers.data?.length ?? '-'}</span></p>
          <p className="text-xs text-slate-600 dark:text-slate-400">Server berjalan</p>
        </button>
      </div>

      <Card title="Monitoring ringkas" right={<button type="button" className="text-sm text-primary" onClick={() => setSection('monitoring')}>Detail</button>}>
        {monitoring.isLoading ? <SkeletonRows rows={1} /> : monitoring.isError ? <ErrorBox message="Gagal memuat monitoring." /> : (
          <div className="grid grid-cols-3 gap-3 text-sm">
            <div><p className="text-slate-600 dark:text-slate-400 text-xs flex items-center gap-1"><Activity size={12} /> CPU</p><p className="font-semibold">{formatPercent(mon?.cpu_percent)}</p></div>
            <div><p className="text-slate-600 dark:text-slate-400 text-xs">Memori</p><p className="font-semibold">{formatBytes(mon?.mem_used_bytes)}</p></div>
            <div><p className="text-slate-600 dark:text-slate-400 text-xs">Penyimpanan</p><p className="font-semibold">{formatBytes(mon?.storage_used_bytes)}</p></div>
          </div>
        )}
        {mon?.unavailable_reason && <p className="text-xs text-warning mt-2">{mon.unavailable_reason}</p>}
      </Card>

      <Card title="Runtime">
        <div className="space-y-2">
          {(info.data?.runtimes ?? []).map((r) => <RuntimeBadge key={r.id} runtime={r} />)}
          {info.isLoading && <SkeletonRows rows={3} />}
        </div>
      </Card>
    </div>
  );
}
