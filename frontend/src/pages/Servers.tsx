import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Play, RotateCw, Square, Trash2 } from 'lucide-react';
import { api } from '../lib/api';
import { formatDateTime, formatUptime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, LoadingBox, StatusPill, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { RuntimeBadge } from '../components/RuntimeBadge';

export function Servers() {
  const qc = useQueryClient();
  const [projectId, setProjectId] = useState('');
  const [port, setPort] = useState('8080');
  const [bindLan, setBindLan] = useState(false);
  const [docRoot, setDocRoot] = useState('');
  const [formError, setFormError] = useState('');
  const [logsFor, setLogsFor] = useState<string | null>(null);

  const servers = useQuery({ queryKey: ['servers'], queryFn: api.servers, refetchInterval: 3000 });
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.projects });
  const runtimes = useQuery({ queryKey: ['runtimes'], queryFn: api.runtimes });
  const logs = useQuery({
    queryKey: ['server-logs', logsFor],
    queryFn: () => api.serverLogs(logsFor as string),
    enabled: logsFor !== null,
    refetchInterval: 3000,
  });

  const invalidate = () => { void qc.invalidateQueries({ queryKey: ['servers'] }); };
  const onErr = (e: unknown) => setFormError(e instanceof Error ? e.message : 'Operasi gagal');

  const create = useMutation({
    mutationFn: () => api.createServer({
      project_id: projectId, engine: 'static', port: Number(port), bind_lan: bindLan,
      document_root: docRoot.trim() || undefined,
    }),
    onSuccess: () => { setFormError(''); invalidate(); },
    onError: onErr,
  });
  const act = useMutation({
    mutationFn: ({ id, action }: { id: string; action: 'start' | 'stop' | 'restart' }) =>
      action === 'start' ? api.startServer(id) : action === 'stop' ? api.stopServer(id) : api.restartServer(id),
    onSuccess: () => { setFormError(''); invalidate(); },
    onError: onErr,
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteServer(id),
    onSuccess: () => { setFormError(''); invalidate(); },
    onError: onErr,
  });

  const portNum = Number(port);
  const portValid = Number.isInteger(portNum) && portNum >= 1024 && portNum <= 65535;

  return (
    <div className="space-y-4">
      {formError && <ErrorBox message={formError} />}

      <Card title="Buat server static">
        <div className="space-y-2">
          <select className={inputCls} value={projectId} onChange={(e) => setProjectId(e.target.value)}>
            <option value="">— Pilih proyek —</option>
            {(projects.data ?? []).map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
          <input className={inputCls} inputMode="numeric" placeholder="Port (1024–65535)" value={port} onChange={(e) => setPort(e.target.value)} />
          {!portValid && port !== '' && <p className="text-xs text-danger">Port harus bilangan 1024–65535.</p>}
          <input className={inputCls} placeholder="Document root relatif (opsional, mis. public)" value={docRoot} onChange={(e) => setDocRoot(e.target.value)} />
          <label className="flex items-start gap-2 text-sm">
            <input type="checkbox" className="mt-0.5" checked={bindLan} onChange={(e) => setBindLan(e.target.checked)} />
            <span>Buka akses LAN (bind 0.0.0.0)</span>
          </label>
          {bindLan && (
            <div className="rounded-lg bg-warning/10 border border-warning/40 text-sm px-3 py-2" role="alert">
              <strong>Peringatan eksposur:</strong> server dapat diakses semua perangkat di jaringan Wi-Fi yang sama.
              Aktifkan hanya di jaringan tepercaya, dan matikan bila tidak diperlukan.
            </div>
          )}
          <button type="button" className={btnPrimary} disabled={!projectId || !portValid || create.isPending} onClick={() => create.mutate()}>
            {create.isPending ? 'Membuat…' : 'Buat server'}
          </button>
          <p className="text-xs text-slate-500">Engine Phase 1: WebForge Static Server. Runtime lain belum tersedia (lihat daftar di bawah).</p>
        </div>
      </Card>

      <Card title="Server">
        {servers.isLoading ? <LoadingBox /> : servers.isError ? <ErrorBox message="Gagal memuat server." /> : (servers.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Belum ada server. Buat server static dari proyek di atas." />
        ) : (
          <div className="space-y-3">
            {servers.data!.map((s) => (
              <div key={s.id} className="rounded-lg border border-slate-200 dark:border-slate-700 p-3">
                <div className="flex items-center justify-between gap-2">
                  <p className="font-medium text-sm">{s.project_name} <span className="text-slate-400">· {s.engine} · :{s.port}</span></p>
                  <StatusPill status={s.status} />
                </div>
                <p className="text-xs text-slate-500 mt-1">Lokal: <span className="break-all">{s.url_local}</span></p>
                <p className="text-xs text-slate-500">LAN: {s.url_lan ?? (s.bind_lan ? 'IP LAN tidak terdeteksi' : 'tidak dibuka')}</p>
                <p className="text-xs text-slate-500">
                  {s.status === 'running' ? `Uptime ${formatUptime(s.uptime_seconds)} · mulai ${formatDateTime(s.started_at)}` : 'Tidak berjalan'}
                  {s.pid !== null ? ` · PID ${s.pid}` : ''}{s.in_process ? ' · in-process' : ''}
                </p>
                {s.error_message && <p className="text-xs text-danger mt-1">{s.error_message}</p>}
                <div className="flex flex-wrap gap-2 mt-2">
                  {s.status !== 'running' && (
                    <button type="button" className={btnPrimary} disabled={act.isPending} onClick={() => act.mutate({ id: s.id, action: 'start' })}>
                      <Play size={14} /> Start
                    </button>
                  )}
                  {s.status === 'running' && (
                    <>
                      <button type="button" className={btnGhost} disabled={act.isPending} onClick={() => act.mutate({ id: s.id, action: 'stop' })}>
                        <Square size={14} /> Stop
                      </button>
                      <button type="button" className={btnGhost} disabled={act.isPending} onClick={() => act.mutate({ id: s.id, action: 'restart' })}>
                        <RotateCw size={14} /> Restart
                      </button>
                    </>
                  )}
                  <button type="button" className={btnGhost} onClick={() => setLogsFor(logsFor === s.id ? null : s.id)}>
                    {logsFor === s.id ? 'Tutup log' : 'Log'}
                  </button>
                  <button type="button" className={btnGhost} disabled={s.status === 'running' || remove.isPending}
                    title={s.status === 'running' ? 'Stop server dulu sebelum menghapus' : 'Hapus server'}
                    onClick={() => { if (window.confirm('Hapus server ini?')) remove.mutate(s.id); }}>
                    <Trash2 size={14} /> Hapus
                  </button>
                </div>
                {logsFor === s.id && (
                  <div className="mt-2 rounded-lg bg-slate-50 dark:bg-darkbg p-2 max-h-48 overflow-auto">
                    {logs.isLoading ? <LoadingBox /> : logs.isError ? <ErrorBox message="Gagal memuat log server." /> : (logs.data?.length ?? 0) === 0 ? (
                      <EmptyBox message="Belum ada log untuk server ini." />
                    ) : logs.data!.map((l) => (
                      <p key={l.id} className="text-xs font-mono"><span className="text-slate-400">{formatDateTime(l.created_at)}</span> [{l.level}] {l.message}</p>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </Card>

      <Card title="Runtime">
        {runtimes.isLoading ? <LoadingBox /> : runtimes.isError ? <ErrorBox message="Gagal memuat runtime." /> : (
          <div className="space-y-2">{(runtimes.data ?? []).map((r) => <RuntimeBadge key={r.id} runtime={r} />)}</div>
        )}
        <p className="text-xs text-slate-500 mt-2">Runtime yang belum tersedia tidak dapat dijalankan di Phase 1 dan tidak ditampilkan tombol start.</p>
      </Card>
    </div>
  );
}
