import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ShieldAlert, ShieldCheck } from 'lucide-react';
import { api, resolveApiConfig } from '../lib/api';
import { formatDateTime, maskToken } from '../lib/format';
import { Card, ErrorBox, LoadingBox, StatusPill, btnGhost, btnPrimary, inputCls } from '../components/ui';

function errText(e: unknown, fallback: string): string {
  return e instanceof Error ? e.message : fallback;
}

function SftpPanel() {
  const qc = useQueryClient();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [port, setPort] = useState('2222');
  const [bindLan, setBindLan] = useState(false);
  const [newPassword, setNewPassword] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');

  const status = useQuery({ queryKey: ['sftp-status'], queryFn: api.sftpStatus, refetchInterval: 5000 });

  useEffect(() => {
    if (status.data) {
      setUsername(status.data.username);
      setPort(String(status.data.port));
      setBindLan(status.data.bind_lan);
    }
  }, [status.data]);

  const invalidate = () => { void qc.invalidateQueries({ queryKey: ['sftp-status'] }); };
  const onErr = (fallback: string) => (e: unknown) => { setNotice(''); setError(errText(e, fallback)); };

  const save = useMutation({
    mutationFn: () => api.putSftpSettings({
      username: username.trim(),
      port: Number(port),
      bind_lan: bindLan,
      ...(password ? { password } : {}),
    }),
    onSuccess: () => { setNotice('Pengaturan SFTP tersimpan.'); setError(''); setPassword(''); invalidate(); },
    onError: onErr('Gagal menyimpan pengaturan SFTP'),
  });
  const start = useMutation({
    mutationFn: api.startSftp,
    onSuccess: () => { setNotice('Server SFTP dinyalakan.'); setError(''); invalidate(); },
    onError: onErr('Gagal menyalakan SFTP'),
  });
  const stop = useMutation({
    mutationFn: api.stopSftp,
    onSuccess: () => { setNotice('Server SFTP dimatikan.'); setError(''); invalidate(); },
    onError: onErr('Gagal mematikan SFTP'),
  });
  const reset = useMutation({
    mutationFn: api.resetSftpPassword,
    onSuccess: (r) => { setNewPassword(r.password); setError(''); invalidate(); },
    onError: onErr('Gagal mereset password SFTP'),
  });

  const portNum = Number(port);
  const portValid = Number.isInteger(portNum) && portNum >= 1024 && portNum <= 65535;
  const busy = save.isPending || start.isPending || stop.isPending || reset.isPending;
  const saveReason = busy
    ? 'Operasi SFTP sedang berjalan…'
    : !username.trim()
      ? 'Isi username SFTP dulu.'
      : !portValid
        ? 'Port SFTP tidak valid: isi bilangan 1024–65535.'
        : '';
  const running = status.data?.running ?? false;

  return (
    <Card title="SFTP">
      <div className="space-y-2">
        {error && <ErrorBox message={error} />}
        {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{notice}</div>}
        {status.isLoading ? <LoadingBox message="Memuat status SFTP…" /> : status.isError ? (
          <ErrorBox message={errText(status.error, 'Gagal memuat status SFTP.')} />
        ) : status.data ? (
          <>
            <p className="flex items-center gap-2 text-sm">
              Status server: <StatusPill status={running ? 'running' : 'stopped'} />
              <span className="text-slate-700 dark:text-slate-300">Port {status.data.port} · {status.data.bind_lan ? 'akses LAN aktif' : 'hanya perangkat ini (127.0.0.1)'}</span>
            </p>
            {status.data.started_at && <p className="text-xs text-slate-700 dark:text-slate-300">Mulai: {formatDateTime(status.data.started_at)}</p>}
            {status.data.last_error && <p className="text-xs text-danger break-all">Error terakhir: {status.data.last_error}</p>}
            <p className="text-xs text-slate-700 dark:text-slate-300 break-all">Root workspace: <span className="font-mono">{status.data.root}</span></p>
            <p className="text-xs text-slate-700 dark:text-slate-300 break-all">
              Fingerprint host key: <span className="font-mono">{status.data.host_key_fingerprint ?? 'belum tersedia (server belum pernah dinyalakan)'}</span>
            </p>
            <p className="text-xs text-slate-700 dark:text-slate-300">Bandingkan fingerprint ini dengan yang ditampilkan klien SFTP saat pertama terhubung.</p>

            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Username SFTP</span>
              <input className={inputCls} placeholder="mis. webforge" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" />
            </label>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Password SFTP (opsional)</span>
              <input className={inputCls} type="password" autoComplete="off" placeholder="Kosongkan untuk mempertahankan password tersimpan" value={password} onChange={(e) => setPassword(e.target.value)} />
            </label>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Port SFTP</span>
              <input className={inputCls} inputMode="numeric" placeholder="mis. 2222" value={port} onChange={(e) => setPort(e.target.value)} />
            </label>
            <label className="flex items-start gap-2 text-sm">
              <input type="checkbox" className="mt-0.5" checked={bindLan} onChange={(e) => setBindLan(e.target.checked)} />
              <span>Buka akses LAN untuk SFTP (bind 0.0.0.0)</span>
            </label>
            {bindLan && (
              <div className="rounded-lg bg-warning/10 border border-warning/40 text-sm px-3 py-2" role="alert">
                <strong>Peringatan keamanan:</strong> SFTP dapat diakses semua perangkat di jaringan yang sama dan memberi akses file workspace kepada siapa pun yang tahu username dan password. Gunakan password kuat, aktifkan hanya di jaringan tepercaya, dan matikan bila tidak diperlukan.
              </div>
            )}
            <div className="flex flex-wrap gap-2">
              <button type="button" className={btnPrimary} disabled={!!saveReason} onClick={() => save.mutate()}>
                {save.isPending ? 'Menyimpan…' : 'Simpan Pengaturan SFTP'}
              </button>
              {!running ? (
                <button type="button" className={btnGhost} disabled={busy} onClick={() => start.mutate()}>
                  {start.isPending ? 'Menyalakan…' : 'Start SFTP'}
                </button>
              ) : (
                <button type="button" className={btnGhost} disabled={busy} onClick={() => stop.mutate()}>
                  {stop.isPending ? 'Mematikan…' : 'Stop SFTP'}
                </button>
              )}
              <button type="button" className={btnGhost} disabled={busy} onClick={() => reset.mutate()}>
                {reset.isPending ? 'Mereset…' : 'Reset Password SFTP'}
              </button>
            </div>
            {saveReason && <p className="text-xs text-slate-800 dark:text-slate-200" role="status">Tombol nonaktif: {saveReason}</p>}
            {newPassword !== null && (
              <div className="rounded-lg bg-warning/10 border border-warning/40 text-sm px-3 py-2 space-y-1" role="alert">
                <p><strong>Password SFTP baru (hanya ditampilkan sekali):</strong></p>
                <p className="font-mono break-all">{newPassword}</p>
                <p className="text-xs">Catat sekarang. Password ini tidak akan ditampilkan lagi; bila hilang, reset lagi untuk membuat password baru.</p>
                <button type="button" className={btnGhost} onClick={() => setNewPassword(null)}>Sembunyikan password</button>
              </div>
            )}

            <p className="text-xs font-medium text-slate-700 dark:text-slate-300">Koneksi aktif ({status.data.connections.length})</p>
            {status.data.connections.length === 0 ? (
              <p className="text-sm text-slate-700 dark:text-slate-300">Tidak ada koneksi SFTP aktif.</p>
            ) : (
              <div className="space-y-1">
                {status.data.connections.map((c) => (
                  <p key={c.id} className="text-xs text-slate-700 dark:text-slate-300 break-all">
                    {c.username} dari {c.remote} · terhubung {formatDateTime(c.connected_at)} · aktivitas terakhir {formatDateTime(c.last_activity)}
                  </p>
                ))}
              </div>
            )}
          </>
        ) : null}
      </div>
    </Card>
  );
}

export function Security() {
  const status = useQuery({ queryKey: ['security-status'], queryFn: api.securityStatus });
  const network = useQuery({ queryKey: ['network'], queryFn: api.network });
  const [tokenMask, setTokenMask] = useState('Memuat…');

  useEffect(() => {
    void resolveApiConfig().then((c) => setTokenMask(maskToken(c.token))).catch(() => setTokenMask('Tidak diketahui'));
  }, []);

  return (
    <div className="space-y-4">
      <Card title="Status keamanan">
        {status.isLoading ? <LoadingBox /> : status.isError ? <ErrorBox message="Gagal memuat status keamanan." /> : (
          <div className="text-sm space-y-2">
            <p className="flex items-center gap-2">
              {status.data?.api_token_set ? <ShieldCheck size={18} className="text-success" /> : <ShieldAlert size={18} className="text-danger" />}
              Token API: {status.data?.api_token_set ? 'terpasang' : 'belum terpasang'}
            </p>
            <p>Bind manajemen: <span className="font-mono">{status.data?.management_bind}</span> (hanya perangkat ini)</p>
            <p>Server terbuka ke LAN: <strong>{status.data?.lan_servers ?? 0}</strong></p>
            {(status.data?.lan_servers ?? 0) > 0 && (
              <div className="rounded-lg bg-warning/10 border border-warning/40 px-3 py-2" role="alert">
                Ada server yang dapat diakses dari jaringan lokal. Pastikan hanya di jaringan tepercaya; matikan akses LAN bila tidak diperlukan.
              </div>
            )}
            {(status.data?.warnings ?? []).map((w) => (
              <p key={w} className="text-warning text-xs flex items-start gap-1"><ShieldAlert size={13} className="mt-0.5 shrink-0" /> {w}</p>
            ))}
          </div>
        )}
      </Card>

      <SftpPanel />

      <Card title="Token API perangkat ini">
        <p className="text-sm">Token aktif (disamarkan): <span className="font-mono">{tokenMask}</span></p>
        <p className="text-xs text-slate-700 dark:text-slate-300 mt-1">
          Token penuh tidak pernah ditampilkan di aplikasi. Token dibuat dan disimpan oleh Core di perangkat ini;
          jangan membagikannya ke siapa pun.
        </p>
      </Card>

      <Card title="Jaringan">
        {network.isLoading ? <LoadingBox /> : network.isError ? <ErrorBox message="Gagal memuat info jaringan." /> : (
          <div className="text-sm space-y-1">
            <p>IP LAN: <span className="font-mono">{network.data?.lan_ip ?? 'Tidak terdeteksi'}</span></p>
            <p>URL manajemen: <span className="font-mono break-all">{network.data?.management_url}</span></p>
            {(network.data?.interfaces ?? []).map((i) => (
              <p key={i.name} className="font-mono text-xs">{i.name}: {i.ip}</p>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
