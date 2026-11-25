import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Play, RefreshCw, RotateCw, Square, Trash2 } from 'lucide-react';
import { api } from '../lib/api';
import { formatDateTime, formatUptime } from '../lib/format';
import { isPhpProject } from '../lib/php';
import type { CreateServerBody, Project, RuntimeInfo, ServerEngine, ServerInstance, ValidateConfigResult } from '../lib/types';
import { Card, EmptyBox, ErrorBox, SkeletonRows, StatusPill, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { RuntimePanel } from '../components/RuntimePanel';
import { PhpManager } from '../components/PhpManager';

export const ENGINE_LABELS: Record<ServerEngine, string> = {
  static: 'WebForge Static Server',
  php: 'PHP built-in server',
  caddy: 'Caddy',
  nginx: 'Nginx',
  apache: 'Apache',
  python: 'Python',
  node: 'Node.js',
};

const WEB_SERVER_ENGINES: ServerEngine[] = ['caddy', 'nginx', 'apache'];

const PYTHON_FRAMEWORKS = new Set(['Flask', 'Django', 'FastAPI', 'Python', 'Bottle', 'Pyramid']);
const NODE_FRAMEWORKS = new Set(['Express', 'React', 'React + Vite', 'Vite', 'Next.js', 'Nuxt', 'Vue', 'Angular', 'Svelte', 'Node.js', 'NestJS']);

/** True bila proyek terdeteksi Python dari data deteksi Core. */
export function isPythonProject(project: Project | null | undefined): boolean {
  const fw = project?.framework;
  if (!fw) return false;
  return fw.language === 'Python' || fw.recommended_runtime === 'python' || PYTHON_FRAMEWORKS.has(fw.framework)
    || fw.detected_files.some((f) => ['requirements.txt', 'pyproject.toml', 'manage.py'].includes(f));
}

/** True bila proyek terdeteksi Node.js dari data deteksi Core. */
export function isNodeProject(project: Project | null | undefined): boolean {
  const fw = project?.framework;
  if (!fw) return false;
  return ['JavaScript', 'TypeScript', 'Node.js', 'Node'].includes(fw.language)
    || fw.recommended_runtime === 'node' || NODE_FRAMEWORKS.has(fw.framework)
    || fw.detected_files.includes('package.json');
}

/** Engine yang relevan untuk proyek terpilih, sebelum filter ketersediaan runtime. */
export function relevantEngines(project: Project | null): ServerEngine[] {
  if (!project) return ['static'];
  const engines: ServerEngine[] = ['static'];
  if (isPhpProject(project)) engines.push('php');
  engines.push(...WEB_SERVER_ENGINES);
  if (isPythonProject(project)) engines.push('python');
  if (isNodeProject(project)) engines.push('node');
  return engines;
}

function runtimeAvailable(runtimes: RuntimeInfo[], id: string): boolean {
  return runtimes.some((r) => r.id === id && r.available);
}

/** Engine relevan yang runtime-nya benar-benar tersedia. Static selalu tersedia (in-process). */
export function availableEngines(project: Project | null, runtimes: RuntimeInfo[]): ServerEngine[] {
  return relevantEngines(project).filter((e) => e === 'static' || runtimeAvailable(runtimes, e));
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : 'Operasi gagal';
}

function ServerConfigPanel({ server, onError }: { server: ServerInstance; onError: (m: string) => void }) {
  const qc = useQueryClient();
  const cfg = useQuery({ queryKey: ['server-config', server.id], queryFn: () => api.serverConfig(server.id) });
  const [text, setText] = useState('');
  const [validation, setValidation] = useState<ValidateConfigResult | null>(null);
  const [notice, setNotice] = useState('');

  useEffect(() => {
    if (cfg.data) setText(cfg.data.config_text);
  }, [cfg.data]);

  const save = useMutation({
    mutationFn: () => api.putServerConfig(server.id, text),
    onSuccess: () => {
      setNotice('Konfigurasi tersimpan.');
      onError('');
      void qc.invalidateQueries({ queryKey: ['server-config', server.id] });
      void qc.invalidateQueries({ queryKey: ['servers'] });
    },
    onError: (e) => onError(errMsg(e)),
  });
  const validate = useMutation({
    mutationFn: () => api.validateServerConfig(server.id, text),
    onSuccess: (r) => { setValidation(r); onError(''); },
    onError: (e) => onError(errMsg(e)),
  });
  const reload = useMutation({
    mutationFn: () => api.reloadServer(server.id),
    onSuccess: () => {
      setNotice('Server dimuat ulang (reload).');
      onError('');
      void qc.invalidateQueries({ queryKey: ['servers'] });
    },
    onError: (e) => onError(errMsg(e)),
  });

  return (
    <div className="mt-3 rounded-lg border border-slate-200 dark:border-slate-700 p-3 space-y-2">
      <p className="font-medium text-sm">Konfigurasi</p>
      {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{notice}</div>}
      {cfg.isLoading ? <SkeletonRows rows={2} /> : cfg.isError ? (
        <ErrorBox message={cfg.error instanceof Error ? cfg.error.message : 'Gagal memuat konfigurasi.'} />
      ) : cfg.data ? (
        <>
          <p className="text-xs text-slate-600 dark:text-slate-400 break-all">Path konfigurasi: <span className="font-mono">{cfg.data.config_path}</span></p>
          <textarea
            aria-label="Konfigurasi server"
            className={`${inputCls} font-mono`}
            rows={8}
            value={text}
            readOnly={!cfg.data.editable}
            onChange={(e) => setText(e.target.value)}
          />
          {!cfg.data.editable && (
            <p className="text-xs text-slate-600 dark:text-slate-400">Konfigurasi ini tidak dapat diedit dari aplikasi (Core menandainya read-only). Kamu tetap dapat memvalidasinya.</p>
          )}
          <div className="flex flex-wrap gap-2">
            <button type="button" className={btnGhost} disabled={cfg.isFetching}
              onClick={() => { setValidation(null); setNotice(''); void cfg.refetch(); }}>
              <RefreshCw size={14} /> {cfg.isFetching ? 'Memuat…' : 'Muat Ulang Konfigurasi'}
            </button>
            {cfg.data.editable && (
              <button type="button" className={btnPrimary} disabled={save.isPending} onClick={() => save.mutate()}>
                {save.isPending ? 'Menyimpan…' : 'Simpan'}
              </button>
            )}
            <button type="button" className={btnGhost} disabled={validate.isPending} onClick={() => validate.mutate()}>
              {validate.isPending ? 'Memvalidasi…' : 'Validasi'}
            </button>
            {server.status === 'running' && (
              <button type="button" className={btnGhost} disabled={reload.isPending} onClick={() => reload.mutate()}>
                <RefreshCw size={14} /> {reload.isPending ? 'Memuat ulang…' : 'Reload'}
              </button>
            )}
          </div>
          {validation && (
            <div className={`rounded-lg border px-3 py-2 ${validation.valid ? 'bg-success/10 border-success/40' : 'bg-red-50 dark:bg-red-950 border-danger/40'}`}>
              <p className={`text-sm font-medium ${validation.valid ? 'text-success' : 'text-danger'}`}>
                {validation.valid ? 'Konfigurasi valid.' : 'Konfigurasi tidak valid.'}
              </p>
              {validation.output && <pre className="text-xs font-mono whitespace-pre-wrap break-all mt-1">{validation.output}</pre>}
            </div>
          )}
          {cfg.data.generated_config && (
            <details>
              <summary className="text-xs text-slate-600 dark:text-slate-400 cursor-pointer">Konfigurasi hasil generate Core</summary>
              <pre className="text-xs font-mono whitespace-pre-wrap break-all mt-1 rounded-lg bg-slate-50 dark:bg-darkbg p-2">{cfg.data.generated_config}</pre>
            </details>
          )}
        </>
      ) : null}
    </div>
  );
}

function PythonPanel({ project }: { project: Project }) {
  const qc = useQueryClient();
  const [output, setOutput] = useState('');
  const [error, setError] = useState('');
  const info = useQuery({ queryKey: ['python-info'], queryFn: api.pythonInfo });
  const proj = useQuery({ queryKey: ['project-python', project.id], queryFn: () => api.projectPython(project.id) });

  const onAction = (r: { ok: boolean; output: string }) => {
    setOutput(r.output);
    setError('');
    void qc.invalidateQueries({ queryKey: ['project-python', project.id] });
  };
  const venv = useMutation({ mutationFn: () => api.createPythonVenv(project.id), onSuccess: onAction, onError: (e) => setError(errMsg(e)) });
  const install = useMutation({ mutationFn: () => api.installPythonDeps(project.id), onSuccess: onAction, onError: (e) => setError(errMsg(e)) });

  return (
    <Card title={`Python: ${project.name}`}>
      <div className="space-y-2">
        {error && <ErrorBox message={error} />}
        {info.isLoading ? <SkeletonRows rows={2} /> : info.isError ? (
          <ErrorBox message={info.error instanceof Error ? info.error.message : 'Gagal memuat info Python.'} />
        ) : !info.data?.available ? (
          <p className="text-sm text-slate-600 dark:text-slate-400">
            Runtime Python belum tersedia{info.data?.error ? `: ${info.data.error}` : ''}. Panel dependensi Python tidak dapat dijalankan sampai runtime tersedia.
          </p>
        ) : (
          <>
            <p className="text-sm">
              Python {info.data.version ?? 'terdeteksi'}
              {info.data.executable ? <span className="text-slate-600 dark:text-slate-400"> · {info.data.executable}</span> : null}
              {' · pip: '}{info.data.pip_available ? 'tersedia' : 'tidak tersedia'}
            </p>
            {proj.isLoading ? <SkeletonRows rows={2} /> : proj.isError ? (
              <ErrorBox message={proj.error instanceof Error ? proj.error.message : 'Gagal memuat status Python proyek.'} />
            ) : proj.data ? (
              <>
                <p className="text-sm">
                  venv: {proj.data.venv_exists ? `sudah dibuat${proj.data.venv_path ? ` di ${proj.data.venv_path}` : ''}` : 'belum dibuat'}
                  {' · requirements.txt: '}{proj.data.requirements_exists ? 'ada' : 'tidak ada'}
                </p>
                {proj.data.entry && (
                  <p className="text-xs text-slate-600 dark:text-slate-400">Entry{proj.data.entry_kind ? ` (${proj.data.entry_kind})` : ''}: <span className="font-mono">{proj.data.entry}</span></p>
                )}
                <div className="flex flex-wrap gap-2">
                  {!proj.data.venv_exists && (
                    <button type="button" className={btnGhost} disabled={venv.isPending} onClick={() => venv.mutate()}>
                      {venv.isPending ? 'Membuat venv…' : 'Buat venv'}
                    </button>
                  )}
                  <button type="button" className={btnGhost} disabled={install.isPending} onClick={() => install.mutate()}>
                    {install.isPending ? 'Menginstal…' : 'Instal dependensi'}
                  </button>
                </div>
                {output && <pre className="text-xs font-mono whitespace-pre-wrap break-all rounded-lg bg-slate-50 dark:bg-darkbg p-2 max-h-48 overflow-auto">{output}</pre>}
                <p className="text-xs font-medium text-slate-600 dark:text-slate-400">Paket terpasang ({proj.data.packages.length})</p>
                {proj.data.packages.length === 0 ? (
                  <p className="text-sm text-slate-600 dark:text-slate-400">Belum ada paket yang dilaporkan Core.</p>
                ) : (
                  <div className="flex flex-wrap gap-1.5">
                    {proj.data.packages.map((p) => (
                      <span key={p.name} className="rounded-full bg-slate-100 dark:bg-darkbg border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-xs font-mono">{p.name} {p.version}</span>
                    ))}
                  </div>
                )}
              </>
            ) : null}
          </>
        )}
      </div>
    </Card>
  );
}

function NodePanel({ project }: { project: Project }) {
  const qc = useQueryClient();
  const [output, setOutput] = useState('');
  const [error, setError] = useState('');
  const proj = useQuery({ queryKey: ['project-node', project.id], queryFn: () => api.projectNode(project.id) });
  const install = useMutation({
    mutationFn: () => api.installNodeDeps(project.id),
    onSuccess: (r) => {
      setOutput(r.output);
      setError('');
      void qc.invalidateQueries({ queryKey: ['project-node', project.id] });
    },
    onError: (e) => setError(errMsg(e)),
  });

  const scriptEntries = Object.entries(proj.data?.scripts ?? {});

  return (
    <Card title={`Node.js: ${project.name}`}>
      <div className="space-y-2">
        {error && <ErrorBox message={error} />}
        {proj.isLoading ? <SkeletonRows rows={2} /> : proj.isError ? (
          <ErrorBox message={proj.error instanceof Error ? proj.error.message : 'Gagal memuat status Node proyek.'} />
        ) : proj.data ? (
          <>
            <p className="text-sm">
              package.json: {proj.data.package_json_exists ? 'ada' : 'tidak ada'}
              {' · node_modules: '}{proj.data.node_modules_exists ? 'sudah terinstal' : 'belum terinstal'}
              {' · npm: '}{proj.data.npm_available ? 'tersedia' : 'tidak tersedia'}
            </p>
            {proj.data.entry && <p className="text-xs text-slate-600 dark:text-slate-400">Entry: <span className="font-mono">{proj.data.entry}</span></p>}
            <p className="text-xs font-medium text-slate-600 dark:text-slate-400">Script terdeteksi ({scriptEntries.length})</p>
            {scriptEntries.length === 0 ? (
              <p className="text-sm text-slate-600 dark:text-slate-400">Tidak ada script di package.json proyek ini.</p>
            ) : (
              <div className="space-y-1">
                {scriptEntries.map(([name, cmd]) => (
                  <p key={name} className="text-xs font-mono break-all"><span className="font-medium">{name}</span>: {cmd}</p>
                ))}
              </div>
            )}
            {proj.data.npm_available ? (
              <button type="button" className={btnGhost} disabled={install.isPending} onClick={() => install.mutate()}>
                {install.isPending ? 'Menjalankan npm install…' : 'npm install'}
              </button>
            ) : (
              <p className="text-sm text-slate-600 dark:text-slate-400">npm belum tersedia di perangkat ini, jadi instalasi dependensi tidak dapat dijalankan.</p>
            )}
            {output && <pre className="text-xs font-mono whitespace-pre-wrap break-all rounded-lg bg-slate-50 dark:bg-darkbg p-2 max-h-48 overflow-auto">{output}</pre>}
          </>
        ) : null}
      </div>
    </Card>
  );
}

export function Servers() {
  const qc = useQueryClient();
  const [projectId, setProjectId] = useState('');
  const [engine, setEngine] = useState<ServerEngine>('static');
  const [port, setPort] = useState('8080');
  const [bindLan, setBindLan] = useState(false);
  const [docRoot, setDocRoot] = useState('');
  const [proxyPort, setProxyPort] = useState('');
  const [command, setCommand] = useState('');
  const [formError, setFormError] = useState('');
  const [logsFor, setLogsFor] = useState<string | null>(null);
  const [configFor, setConfigFor] = useState<string | null>(null);

  const servers = useQuery({ queryKey: ['servers'], queryFn: api.servers, refetchInterval: 3000 });
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.projects });
  const runtimes = useQuery({ queryKey: ['runtimes'], queryFn: () => api.runtimes() });
  const logs = useQuery({
    queryKey: ['server-logs', logsFor],
    queryFn: () => api.serverLogs(logsFor as string),
    enabled: logsFor !== null,
    refetchInterval: 3000,
  });

  const invalidate = () => { void qc.invalidateQueries({ queryKey: ['servers'] }); };
  const onErr = (e: unknown) => setFormError(errMsg(e));

  const selectedProject = (projects.data ?? []).find((p) => p.id === projectId) ?? null;
  const selectedIsPhp = isPhpProject(selectedProject);
  const selectedIsPython = isPythonProject(selectedProject);
  const selectedIsNode = isNodeProject(selectedProject);
  const runtimeList = runtimes.data ?? [];
  const engineOptions = availableEngines(selectedProject, runtimeList);
  const unavailableEngines = relevantEngines(selectedProject)
    .filter((e) => e !== 'static' && !engineOptions.includes(e))
    .map((e) => ({ engine: e, reason: runtimeList.find((r) => r.id === e)?.reason ?? null }));

  useEffect(() => {
    if (!engineOptions.includes(engine)) setEngine(engineOptions[0] ?? 'static');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engineOptions.join(','), engine]);

  function onProjectChange(id: string) {
    setProjectId(id);
    const p = (projects.data ?? []).find((x) => x.id === id) ?? null;
    const options = availableEngines(p, runtimeList);
    if (isPhpProject(p) && options.includes('php')) setEngine('php');
    else if (isPythonProject(p) && options.includes('python')) setEngine('python');
    else if (isNodeProject(p) && options.includes('node')) setEngine('node');
    else setEngine('static');
    setDocRoot(isPhpProject(p) ? p?.framework?.recommended_document_root ?? '' : '');
  }

  const portNum = Number(port);
  const portValid = Number.isInteger(portNum) && portNum >= 1024 && portNum <= 65535;
  const proxyNum = Number(proxyPort);
  const proxyValid = proxyPort.trim() === '' || (Number.isInteger(proxyNum) && proxyNum >= 1024 && proxyNum <= 65535);
  const isWebServerEngine = WEB_SERVER_ENGINES.includes(engine);
  const isCommandEngine = engine === 'python' || engine === 'node';

  const create = useMutation({
    mutationFn: () => {
      const body: CreateServerBody = {
        project_id: projectId, engine, port: portNum, bind_lan: bindLan,
        document_root: docRoot.trim() || undefined,
      };
      if (isWebServerEngine && proxyPort.trim()) body.proxy_port = proxyNum;
      if (isCommandEngine && command.trim()) body.command = command.trim();
      return api.createServer(body);
    },
    onSuccess: () => { setFormError(''); invalidate(); },
    onError: onErr,
  });
  const act = useMutation({
    mutationFn: ({ id, action }: { id: string; action: 'start' | 'stop' | 'restart' | 'reload' }) =>
      action === 'start' ? api.startServer(id)
        : action === 'stop' ? api.stopServer(id)
          : action === 'restart' ? api.restartServer(id)
            : api.reloadServer(id),
    onSuccess: () => { setFormError(''); invalidate(); },
    onError: onErr,
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteServer(id),
    onSuccess: () => { setFormError(''); invalidate(); },
    onError: onErr,
  });

  return (
    <div className="space-y-4">
      {formError && <ErrorBox message={formError} />}

      <Card title="Buat server">
        <div className="space-y-2">
          <select className={inputCls} value={projectId} onChange={(e) => onProjectChange(e.target.value)}>
            <option value="">— Pilih proyek —</option>
            {(projects.data ?? []).map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
          <label className="block text-sm">
            <span className="text-xs font-medium text-slate-600 dark:text-slate-400">Engine</span>
            <select className={inputCls} value={engine} onChange={(e) => setEngine(e.target.value as ServerEngine)}>
              {engineOptions.map((e) => <option key={e} value={e}>{ENGINE_LABELS[e]}</option>)}
            </select>
          </label>
          {!selectedProject && <p className="text-xs text-slate-600 dark:text-slate-400">Pilih proyek untuk melihat engine yang sesuai dengan proyek itu.</p>}
          {runtimes.isLoading && <p className="text-xs text-slate-600 dark:text-slate-400">Memuat daftar runtime… engine selain Static muncul setelah runtime termuat.</p>}
          {selectedProject && unavailableEngines.length > 0 && (
            <p className="text-xs text-slate-600 dark:text-slate-400">
              Engine berikut tidak ditampilkan karena runtime-nya belum tersedia:{' '}
              {unavailableEngines.map((u) => `${ENGINE_LABELS[u.engine]}${u.reason ? ` (${u.reason})` : ''}`).join('; ')}.
            </p>
          )}
          <input className={inputCls} inputMode="numeric" placeholder="Port (1024–65535)" value={port} onChange={(e) => setPort(e.target.value)} />
          {!portValid && port !== '' && <p className="text-xs text-danger">Port harus bilangan 1024–65535.</p>}
          {isWebServerEngine && (
            <>
              <label className="block text-sm">
                <span className="text-xs font-medium text-slate-600 dark:text-slate-400">Port backend reverse proxy (opsional)</span>
                <input className={inputCls} inputMode="numeric" placeholder="mis. 3000" value={proxyPort} onChange={(e) => setProxyPort(e.target.value)} />
              </label>
              {!proxyValid && <p className="text-xs text-danger">Port backend harus bilangan 1024–65535 atau dikosongkan.</p>}
            </>
          )}
          {isCommandEngine && (
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-600 dark:text-slate-400">Perintah menjalankan (opsional)</span>
              <input className={inputCls} placeholder={engine === 'python' ? 'mis. python app.py' : 'mis. npm start'} value={command} onChange={(e) => setCommand(e.target.value)} />
            </label>
          )}
          <input className={inputCls} placeholder="Document root relatif (opsional, mis. public)" value={docRoot} onChange={(e) => setDocRoot(e.target.value)} />
          {selectedIsPhp && selectedProject?.framework && (
            <p className="text-xs text-slate-600 dark:text-slate-400">Rekomendasi deteksi {selectedProject.framework.framework}: document root <span className="font-mono">{selectedProject.framework.recommended_document_root}</span>.</p>
          )}
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
          <button type="button" className={btnPrimary} disabled={!projectId || !portValid || !proxyValid || create.isPending} onClick={() => create.mutate()}>
            {create.isPending ? 'Membuat…' : 'Buat server'}
          </button>
          <p className="text-xs text-slate-600 dark:text-slate-400">Engine yang tampil hanya yang runtime-nya tersedia dan sesuai proyek terpilih. Bila Core menolak pembuatan server, pesan errornya ditampilkan apa adanya di atas.</p>
        </div>
      </Card>

      {selectedProject && selectedIsPython && <PythonPanel project={selectedProject} />}
      {selectedProject && selectedIsNode && <NodePanel project={selectedProject} />}

      <Card title="Server">
        {servers.isLoading ? <SkeletonRows rows={3} /> : servers.isError ? <ErrorBox message="Gagal memuat server." /> : (servers.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Belum ada server. Buat server dari proyek di atas." />
        ) : (
          <div className="space-y-3">
            {servers.data!.map((s) => {
              const isWebServer = WEB_SERVER_ENGINES.includes(s.engine as ServerEngine);
              return (
                <div key={s.id} className="rounded-lg border border-slate-200 dark:border-slate-700 p-3">
                  <div className="flex items-center justify-between gap-2">
                    <p className="font-medium text-sm">{s.project_name} <span className="text-slate-600 dark:text-slate-400">· {s.engine} · :{s.port}</span></p>
                    <StatusPill status={s.status} />
                  </div>
                  <p className="text-xs text-slate-600 dark:text-slate-400 mt-1">Lokal: <span className="break-all">{s.url_local}</span></p>
                  <p className="text-xs text-slate-600 dark:text-slate-400">LAN: {s.url_lan ?? (s.bind_lan ? 'IP LAN tidak terdeteksi' : 'tidak dibuka')}</p>
                  {s.proxy_port !== null && s.proxy_port !== undefined && <p className="text-xs text-slate-600 dark:text-slate-400">Reverse proxy ke port backend: {s.proxy_port}</p>}
                  {s.command && <p className="text-xs text-slate-600 dark:text-slate-400">Perintah: <span className="font-mono break-all">{s.command}</span></p>}
                  <p className="text-xs text-slate-600 dark:text-slate-400">
                    {s.status === 'running' ? `Uptime ${formatUptime(s.uptime_seconds)} · mulai ${formatDateTime(s.started_at)}` : 'Tidak berjalan'}
                    {s.pid !== null ? ` · PID ${s.pid}` : ''}{s.in_process ? ' · in-process' : ''}
                  </p>
                  {s.error_message && <p className="text-xs text-danger mt-1">{s.error_message}</p>}
                  <div className="flex flex-wrap gap-2 mt-2">
                    {(s.status === 'stopped' || s.status === 'error') && (
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
                        {isWebServer && (
                          <button type="button" className={btnGhost} disabled={act.isPending} onClick={() => act.mutate({ id: s.id, action: 'reload' })}>
                            <RefreshCw size={14} /> Reload
                          </button>
                        )}
                      </>
                    )}
                    {isWebServer && (
                      <button type="button" className={btnGhost} onClick={() => setConfigFor(configFor === s.id ? null : s.id)}>
                        {configFor === s.id ? 'Tutup konfigurasi' : 'Konfigurasi'}
                      </button>
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
                  {configFor === s.id && isWebServer && <ServerConfigPanel server={s} onError={setFormError} />}
                  {logsFor === s.id && (
                    <div className="mt-2 rounded-lg bg-slate-50 dark:bg-darkbg p-2 max-h-48 overflow-auto">
                      {logs.isLoading ? <SkeletonRows rows={2} /> : logs.isError ? <ErrorBox message="Gagal memuat log server." /> : (logs.data?.length ?? 0) === 0 ? (
                        <EmptyBox message="Belum ada log untuk server ini." />
                      ) : logs.data!.map((l) => (
                        <p key={l.id} className="text-xs font-mono"><span className="text-slate-600 dark:text-slate-400">{formatDateTime(l.created_at)}</span> [{l.level}] {l.message}</p>
                      ))}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </Card>

      <Card title="Runtime">
        {runtimes.isLoading ? <SkeletonRows rows={3} /> : runtimes.isError ? <ErrorBox message="Gagal memuat runtime." /> : (
          <RuntimePanel
            runtimes={runtimeList}
            renderAction={(r) => {
              const engineId = r.id as ServerEngine;
              return r.available && engineOptions.includes(engineId) ? (
                <button type="button" className={btnGhost} onClick={() => setEngine(engineId)}>
                  Pakai engine ini
                </button>
              ) : null;
            }}
          />
        )}
        <p className="text-xs text-slate-600 dark:text-slate-400 mt-2">Runtime yang belum tersedia tidak dapat dijalankan dan tidak ditampilkan tombol start. Tombol "Pakai engine ini" hanya muncul untuk runtime tersedia yang relevan dengan proyek terpilih. "Tes Runtime" menjalankan probe segar dan menampilkan keluaran aslinya.</p>
      </Card>

      <PhpManager />
    </div>
  );
}
