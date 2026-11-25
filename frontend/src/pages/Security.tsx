import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ShieldAlert, ShieldCheck } from 'lucide-react';
import { api, resolveApiConfig } from '../lib/api';
import { maskToken } from '../lib/format';
import { Card, ErrorBox, LoadingBox } from '../components/ui';

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

      <Card title="Token API perangkat ini">
        <p className="text-sm">Token aktif (disamarkan): <span className="font-mono">{tokenMask}</span></p>
        <p className="text-xs text-slate-500 mt-1">
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
