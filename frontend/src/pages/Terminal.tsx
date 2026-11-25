import { useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Send, Trash2 } from 'lucide-react';
import { api, terminalWsUrl } from '../lib/api';
import { formatDateTime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, LoadingBox, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { ProjectPicker } from '../components/ProjectPicker';
import { useAppStore } from '../store';

interface Line { kind: 'cmd' | 'out' | 'err' | 'info'; text: string }

export function Terminal() {
  const qc = useQueryClient();
  const activeProjectId = useAppStore((s) => s.activeProjectId);
  const [command, setCommand] = useState('');
  const [lines, setLines] = useState<Line[]>([]);
  const [running, setRunning] = useState(false);
  const [transport, setTransport] = useState<'ws' | 'http' | null>(null);
  const outputRef = useRef<HTMLDivElement>(null);

  const history = useQuery({ queryKey: ['terminal-history'], queryFn: () => api.terminalHistory(50) });

  const push = (line: Line) => {
    setLines((prev) => [...prev, line]);
    requestAnimationFrame(() => { outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight }); });
  };

  const runHttp = useMutation({
    mutationFn: (cmd: string) => api.terminalExecute(cmd, activeProjectId ?? undefined),
    onSuccess: (res) => {
      if (res.stdout) push({ kind: 'out', text: res.stdout });
      if (res.stderr) push({ kind: 'err', text: res.stderr });
      push({ kind: 'info', text: `exit ${res.exit_code} · ${res.duration_ms} ms (HTTP)` });
      setRunning(false);
      void qc.invalidateQueries({ queryKey: ['terminal-history'] });
    },
    onError: (e) => {
      push({ kind: 'err', text: e instanceof Error ? e.message : 'Perintah gagal dijalankan' });
      setRunning(false);
    },
  });

  async function runWs(cmd: string): Promise<boolean> {
    const url = await terminalWsUrl();
    return new Promise<boolean>((resolve) => {
      let settled = false;
      const ws = new WebSocket(url);
      const timer = window.setTimeout(() => { if (!settled) { settled = true; ws.close(); resolve(false); } }, 5000);
      ws.onopen = () => {
        window.clearTimeout(timer);
        setTransport('ws');
        ws.send(JSON.stringify({ type: 'run', command: cmd, project_id: activeProjectId ?? null }));
      };
      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(String(ev.data)) as { type: string; data?: string; code?: number };
          if (msg.type === 'output' && msg.data) push({ kind: 'out', text: msg.data });
          if (msg.type === 'exit') {
            push({ kind: 'info', text: `exit ${msg.code ?? '?'} (WebSocket)` });
            settled = true;
            ws.close();
            setRunning(false);
            void qc.invalidateQueries({ queryKey: ['terminal-history'] });
            resolve(true);
          }
        } catch { /* abaikan pesan tak dikenal */ }
      };
      ws.onerror = () => { if (!settled) { settled = true; window.clearTimeout(timer); resolve(false); } };
      ws.onclose = () => { if (!settled) { settled = true; window.clearTimeout(timer); resolve(false); } };
    });
  }

  async function run() {
    const cmd = command.trim();
    if (!cmd || running) return;
    setCommand('');
    push({ kind: 'cmd', text: `$ ${cmd}` });
    setRunning(true);
    const ok = await runWs(cmd);
    if (!ok) {
      setTransport('http');
      push({ kind: 'info', text: 'WebSocket tidak tersedia — memakai HTTP.' });
      runHttp.mutate(cmd);
    }
  }

  return (
    <div className="space-y-4">
      <Card title="Terminal">
        <div className="space-y-2">
          <ProjectPicker />
          <p className="text-xs text-slate-500">Perintah dijalankan dengan sh -c di folder proyek aktif (atau workspace bila belum dipilih), timeout 30 detik.
            {transport && <> Transport terakhir: {transport === 'ws' ? 'WebSocket' : 'HTTP'}.</>}</p>
          <div ref={outputRef} className="rounded-lg bg-[#0D1117] text-slate-100 p-3 h-64 overflow-auto font-mono text-xs whitespace-pre-wrap">
            {lines.length === 0 ? <span className="text-slate-500">Belum ada keluaran. Ketik perintah di bawah.</span> :
              lines.map((l, i) => (
                <div key={i} className={l.kind === 'cmd' ? 'text-emerald-300' : l.kind === 'err' ? 'text-red-300' : l.kind === 'info' ? 'text-slate-400' : ''}>{l.text}</div>
              ))}
          </div>
          <div className="flex gap-2">
            <input className={inputCls} placeholder="mis. ls -la" value={command}
              onChange={(e) => setCommand(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') void run(); }} />
            <button type="button" className={btnPrimary} disabled={!command.trim() || running} onClick={() => void run()}>
              <Send size={15} /> {running ? 'Berjalan…' : 'Jalankan'}
            </button>
            <button type="button" className={btnGhost} onClick={() => setLines([])} title="Bersihkan keluaran"><Trash2 size={15} /></button>
          </div>
        </div>
      </Card>

      <Card title="Riwayat perintah">
        {history.isLoading ? <LoadingBox /> : history.isError ? <ErrorBox message="Gagal memuat riwayat." /> : (history.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Belum ada riwayat perintah." />
        ) : (
          <div className="space-y-1">
            {history.data!.map((h) => (
              <button key={h.id} type="button" className="w-full text-left rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2" onClick={() => setCommand(h.command)}>
                <p className="font-mono text-xs break-all">{h.command}</p>
                <p className="text-xs text-slate-500">exit {h.exit_code} · {formatDateTime(h.created_at)}</p>
              </button>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
