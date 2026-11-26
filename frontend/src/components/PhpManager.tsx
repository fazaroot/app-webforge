import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { api } from '../lib/api';
import { evaluatePhpRequirements, isPhpProject } from '../lib/php';
import type { ComposerRunResult, PhpCliResult, PhpSettings, PhpVersionInfo, Project } from '../lib/types';
import { Card, ErrorBox, LoadingBox, btnGhost, btnPrimary, inputCls } from './ui';
import { ProjectPicker } from './ProjectPicker';
import { useAppStore } from '../store';

const EMPTY_SETTINGS: PhpSettings = {
  memory_limit: '', upload_max_filesize: '', post_max_size: '',
  max_execution_time: 0, date_timezone: '', display_errors: false,
};

function tokenizeArgs(input: string): string[] {
  const args: string[] = [];
  let current = '';
  let quote: string | null = null;
  for (const ch of input) {
    if (quote) {
      if (ch === quote) quote = null;
      else current += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (/\s/.test(ch)) {
      if (current) { args.push(current); current = ''; }
    } else {
      current += ch;
    }
  }
  if (current) args.push(current);
  return args;
}

function errText(e: unknown, fallback: string): string {
  return e instanceof Error ? e.message : fallback;
}

function ComposerPanel({ projectId, phpVersion }: { projectId: string | null; phpVersion: string }) {
  const qc = useQueryClient();
  const [pkg, setPkg] = useState('');
  const [result, setResult] = useState<ComposerRunResult | null>(null);
  const [error, setError] = useState('');
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.projects });
  const project = (projects.data ?? []).find((p) => p.id === projectId) ?? null;
  const isPhp = isPhpProject(project);
  const status = useQuery({
    queryKey: ['project-composer', projectId],
    queryFn: () => api.projectComposer(projectId as string),
    enabled: !!projectId && isPhp,
  });

  const run = useMutation({
    mutationFn: (body: { action: 'install' | 'update' | 'require'; package?: string }) =>
      api.runComposer(projectId as string, { ...body, php_version: phpVersion || undefined }),
    onSuccess: (r) => {
      setResult(r);
      setError('');
      void qc.invalidateQueries({ queryKey: ['project-composer', projectId] });
    },
    onError: (e) => { setResult(null); setError(errText(e, 'Composer gagal dijalankan')); },
  });

  const composerAvailable = status.data?.available ?? false;
  const requireReason = !pkg.trim() ? 'Isi nama paket dulu untuk Require.' : '';
  const baseReason = !projectId
    ? 'Pilih proyek aktif dulu.'
    : project && !isPhp
      ? 'Composer hanya untuk proyek PHP.'
      : status.isLoading
        ? 'Memuat status Composer…'
        : status.data && !composerAvailable
          ? `Composer belum tersedia${status.data.error ? `: ${status.data.error}` : ''}.`
          : run.isPending
            ? 'Composer sedang berjalan…'
            : '';

  return (
    <Card title="Composer">
      <div className="space-y-2">
        <ProjectPicker />
        {error && <ErrorBox message={error} />}
        {!projectId ? (
          <p className="text-sm text-slate-700 dark:text-slate-300">Pilih proyek aktif untuk melihat status Composer proyek itu.</p>
        ) : project && !isPhp ? (
          <p className="text-sm text-slate-700 dark:text-slate-300">Proyek terpilih bukan proyek PHP, jadi Composer tidak berlaku.</p>
        ) : status.isLoading ? (
          <LoadingBox message="Memuat status Composer…" />
        ) : status.isError ? (
          <ErrorBox message={errText(status.error, 'Gagal memuat status Composer.')} />
        ) : status.data ? (
          <>
            <p className="text-sm">
              Composer: {status.data.available
                ? `tersedia${status.data.version ? ` (${status.data.version})` : ''}`
                : 'belum tersedia'}
              {status.data.error ? <span className="text-danger"> · {status.data.error}</span> : null}
            </p>
            <p className="text-sm">
              composer.json: {status.data.has_composer_json ? 'ada' : 'tidak ada'}
              {' · composer.lock: '}{status.data.has_lock ? 'ada' : 'tidak ada'}
              {' · vendor/: '}{status.data.vendor_present ? 'sudah terinstal' : 'belum terinstal'}
            </p>
            <p className="text-xs text-slate-700 dark:text-slate-300">
              Versi PHP proyek: {status.data.php_version ?? 'mengikuti default Core'}
              {status.data.recommended_php_version ? ` · rekomendasi framework: ${status.data.recommended_php_version}` : ''}
              {phpVersion ? ` · Composer dijalankan dengan PHP ${phpVersion}` : ''}
            </p>
            <div className="rounded-lg bg-warning/10 border border-warning/40 text-sm px-3 py-2" role="alert">
              Composer membutuhkan jaringan internet untuk mengunduh paket. Tanpa jaringan, perintah akan gagal dan errornya ditampilkan apa adanya.
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" className={btnPrimary} disabled={!!baseReason} onClick={() => run.mutate({ action: 'install' })}>
                {run.isPending ? 'Menjalankan…' : 'Composer Install'}
              </button>
              <button type="button" className={btnGhost} disabled={!!baseReason} onClick={() => run.mutate({ action: 'update' })}>
                {run.isPending ? 'Menjalankan…' : 'Composer Update'}
              </button>
            </div>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Paket untuk Require</span>
              <input className={inputCls} placeholder="mis. monolog/monolog" value={pkg} onChange={(e) => setPkg(e.target.value)} />
            </label>
            <button type="button" className={btnGhost} disabled={!!baseReason || !!requireReason}
              onClick={() => run.mutate({ action: 'require', package: pkg.trim() })}>
              {run.isPending ? 'Menjalankan…' : 'Require Paket'}
            </button>
            {(baseReason || requireReason) && (
              <p className="text-xs text-slate-800 dark:text-slate-200" role="status">Tombol nonaktif: {baseReason || requireReason}</p>
            )}
            {result && (
              <div className="rounded-lg bg-slate-50/80 dark:bg-darkbg/70 p-2 space-y-1">
                <p className="text-xs text-slate-700 dark:text-slate-300">
                  Exit code: {result.exit_code ?? 'tidak ada'} · {result.duration_ms} ms{result.timed_out ? ' · timeout' : ''}
                </p>
                {result.stdout && <pre className="text-xs font-mono whitespace-pre-wrap break-all max-h-56 overflow-auto">{result.stdout}</pre>}
                {result.stderr && <pre className="text-xs font-mono whitespace-pre-wrap break-all text-danger max-h-56 overflow-auto">{result.stderr}</pre>}
                {!result.stdout && !result.stderr && <p className="text-xs text-slate-700 dark:text-slate-300">Tidak ada output.</p>}
              </div>
            )}
          </>
        ) : null}
      </div>
    </Card>
  );
}

export function PhpManager() {
  const qc = useQueryClient();
  const projectId = useAppStore((s) => s.activeProjectId);
  const [form, setForm] = useState<PhpSettings>(EMPTY_SETTINGS);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [cliArgs, setCliArgs] = useState('-v');
  const [cliResult, setCliResult] = useState<PhpCliResult | null>(null);
  const [versionOverride, setVersionOverride] = useState<string | null>(null);

  const info = useQuery({ queryKey: ['php-info'], queryFn: () => api.phpInfo() });
  const settings = useQuery({ queryKey: ['php-settings'], queryFn: api.phpSettings });

  const versions: PhpVersionInfo[] = info.data?.versions && info.data.versions.length > 0
    ? info.data.versions
    : info.data?.available
      ? [{
        version: info.data.selected_version ?? (info.data.version ? info.data.version.split('.').slice(0, 2).join('.') : '8.3'),
        patch_version: info.data.version,
        available: true,
        fpm_available: false,
        extensions: info.data.extensions,
        binary_path: info.data.binary_path,
        fpm_binary_path: null,
        error: null,
      }]
      : [];
  const selectedVersion = versionOverride
    ?? info.data?.selected_version
    ?? versions.find((v) => v.available)?.version
    ?? versions[0]?.version
    ?? '';
  const versionEntry = versions.find((v) => v.version === selectedVersion) ?? null;
  const detail = useQuery({
    queryKey: ['php-info', selectedVersion],
    queryFn: () => api.phpInfo(selectedVersion || undefined),
    enabled: !!info.data && !!selectedVersion && !!info.data.versions,
  });
  const extensions = detail.data?.extensions ?? versionEntry?.extensions ?? info.data?.extensions ?? [];
  const displayVersion = detail.data?.version ?? versionEntry?.patch_version ?? info.data?.version ?? null;
  const binaryPath = detail.data?.binary_path ?? versionEntry?.binary_path ?? info.data?.binary_path ?? null;
  const composer = detail.data?.composer ?? info.data?.composer;

  useEffect(() => {
    if (settings.data) setForm(settings.data);
  }, [settings.data]);

  const save = useMutation({
    mutationFn: () => api.putPhpSettings(form),
    onSuccess: (saved) => {
      setForm(saved);
      setNotice('Pengaturan PHP tersimpan.');
      setError('');
      void qc.invalidateQueries({ queryKey: ['php-settings'] });
    },
    onError: (e) => { setNotice(''); setError(errText(e, 'Gagal menyimpan pengaturan PHP')); },
  });

  const runCli = useMutation({
    mutationFn: () => api.phpCli(projectId as string, tokenizeArgs(cliArgs)),
    onSuccess: (r) => { setCliResult(r); setError(''); },
    onError: (e) => { setCliResult(null); setError(errText(e, 'PHP CLI gagal dijalankan')); },
  });

  const textField = (key: 'memory_limit' | 'upload_max_filesize' | 'post_max_size' | 'date_timezone', label: string, placeholder: string) => (
    <label className="block text-sm">
      <span className="text-xs font-medium text-slate-700 dark:text-slate-300">{label}</span>
      <input className={inputCls} placeholder={placeholder} value={form[key]}
        onChange={(e) => setForm({ ...form, [key]: e.target.value })} />
    </label>
  );

  return (
    <>
      <Card title="PHP Manager">
        {error && <ErrorBox message={error} />}
        {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2 mb-2">{notice}</div>}
        {info.isLoading ? <LoadingBox message="Memuat info PHP…" /> : info.isError ? (
          <ErrorBox message={errText(info.error, 'Gagal memuat info PHP.')} />
        ) : !info.data?.available && versions.length === 0 ? (
          <div className="space-y-1">
            <p className="text-sm"><span className="rounded-full bg-warning/15 text-warning px-2 py-0.5 text-xs font-medium">PHP belum tersedia</span></p>
            <p className="text-sm text-slate-700 dark:text-slate-300">
              {info.data?.error ?? 'Runtime PHP belum tersedia di perangkat ini. Server PHP akan ditolak Core sampai runtime tersedia.'}
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            <p className="text-sm">
              <span className="rounded-full bg-success/15 text-success px-2 py-0.5 text-xs font-medium">PHP tersedia</span>{' '}
              {displayVersion ? `Versi ${displayVersion}` : 'Versi tidak diketahui'}
            </p>
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">Versi PHP</span>
              <select className={inputCls} value={selectedVersion} onChange={(e) => setVersionOverride(e.target.value)}>
                {versions.map((v) => (
                  <option key={v.version} value={v.version} disabled={!v.available}>
                    PHP {v.version}{v.patch_version ? ` (${v.patch_version})` : ''}{v.available ? '' : ' — tidak tersedia'}
                  </option>
                ))}
              </select>
            </label>
            {binaryPath && <p className="text-xs text-slate-700 dark:text-slate-300 break-all">Binary: {binaryPath}</p>}
            {versions.length > 0 && (
              <div className="space-y-1">
                <p className="text-xs font-medium text-slate-700 dark:text-slate-300">Status PHP-FPM per versi</p>
                {versions.map((v) => (
                  <p key={v.version} className="text-xs text-slate-700 dark:text-slate-300">
                    PHP {v.version}: {v.available ? 'tersedia' : `tidak tersedia${v.error ? ` (${v.error})` : ''}`}
                    {' · PHP-FPM: '}{v.fpm_available ? 'tersedia' : 'tidak tersedia'}
                    {v.fpm_binary_path ? <span className="break-all"> · {v.fpm_binary_path}</span> : null}
                  </p>
                ))}
              </div>
            )}
            <p className="text-xs font-medium text-slate-700 dark:text-slate-300">Ekstensi terpasang ({extensions.length})</p>
            {extensions.length === 0 ? (
              <p className="text-sm text-slate-700 dark:text-slate-300">Tidak ada ekstensi yang dilaporkan Core.</p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {extensions.map((e) => (
                  <span key={e} className="rounded-full bg-slate-100/80 dark:bg-darkbg/70 border border-sky-200 dark:border-cyan-400/30 px-2 py-0.5 text-xs font-mono">{e}</span>
                ))}
              </div>
            )}
            <p className="text-sm">
              Composer: {composer
                ? composer.available
                  ? `tersedia${composer.version ? ` (${composer.version})` : ''}${composer.path ? ` · ${composer.path}` : ''}`
                  : `belum tersedia${composer.error ? `: ${composer.error}` : ''}`
                : 'status Composer belum dilaporkan Core'}
            </p>
          </div>
        )}
        <button type="button" className={`${btnGhost} mt-3`} disabled={info.isFetching} onClick={() => void info.refetch()}>
          <RefreshCw size={14} /> {info.isFetching ? 'Memuat ulang…' : 'Muat ulang info PHP'}
        </button>
      </Card>

      <ComposerPanel projectId={projectId} phpVersion={selectedVersion} />

      <Card title="Pengaturan php.ini">
        {settings.isLoading ? <LoadingBox message="Memuat pengaturan PHP…" /> : settings.isError ? (
          <ErrorBox message={errText(settings.error, 'Gagal memuat pengaturan PHP.')} />
        ) : (
          <div className="space-y-2">
            {textField('memory_limit', 'memory_limit', 'mis. 256M')}
            {textField('upload_max_filesize', 'upload_max_filesize', 'mis. 32M')}
            {textField('post_max_size', 'post_max_size', 'mis. 32M')}
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-700 dark:text-slate-300">max_execution_time (detik)</span>
              <input className={inputCls} inputMode="numeric" value={String(form.max_execution_time)}
                onChange={(e) => setForm({ ...form, max_execution_time: Number(e.target.value) || 0 })} />
            </label>
            {textField('date_timezone', 'date_timezone', 'mis. Asia/Jakarta')}
            <label className="flex items-start gap-2 text-sm">
              <input type="checkbox" className="mt-0.5" checked={form.display_errors}
                onChange={(e) => setForm({ ...form, display_errors: e.target.checked })} />
              <span>display_errors (tampilkan error PHP di browser)</span>
            </label>
            <button type="button" className={btnPrimary} disabled={save.isPending} onClick={() => save.mutate()}>
              {save.isPending ? 'Menyimpan…' : 'Simpan php.ini'}
            </button>
            <p className="text-xs text-slate-700 dark:text-slate-300">Pengaturan disimpan oleh Core dan dipakai server PHP berikutnya. Server PHP yang sedang berjalan mungkin perlu di-restart.</p>
          </div>
        )}
      </Card>

      <Card title="PHP CLI">
        <div className="space-y-2">
          <ProjectPicker />
          <input className={inputCls} placeholder="Argumen php, mis. -v atau artisan --version" value={cliArgs} onChange={(e) => setCliArgs(e.target.value)} />
          <button type="button" className={btnGhost} disabled={!projectId || runCli.isPending} onClick={() => runCli.mutate()}>
            {runCli.isPending ? 'Menjalankan…' : 'Jalankan PHP CLI'}
          </button>
          {!projectId && <p className="text-xs text-slate-700 dark:text-slate-300">Pilih proyek aktif untuk menjalankan PHP CLI di folder proyek itu.</p>}
          {cliResult && (
            <div className="rounded-lg bg-slate-50/80 dark:bg-darkbg/70 p-2 space-y-1">
              <p className="text-xs text-slate-700 dark:text-slate-300">
                Exit code: {cliResult.exit_code ?? 'tidak ada'}{cliResult.timed_out ? ' · timeout' : ''}
              </p>
              {cliResult.stdout && <pre className="text-xs font-mono whitespace-pre-wrap break-all">{cliResult.stdout}</pre>}
              {cliResult.stderr && <pre className="text-xs font-mono whitespace-pre-wrap break-all text-danger">{cliResult.stderr}</pre>}
              {!cliResult.stdout && !cliResult.stderr && <p className="text-xs text-slate-700 dark:text-slate-300">Tidak ada output.</p>}
            </div>
          )}
        </div>
      </Card>
    </>
  );
}

/** Kesiapan ekstensi PHP untuk proyek PHP, dibandingkan daftar nyata dari Core. */
export function PhpReadiness({ project }: { project: Project }) {
  const php = isPhpProject(project);
  const version = project.php_version ?? project.framework?.recommended_php_version ?? undefined;
  const info = useQuery({
    queryKey: ['php-info', version ?? 'default'],
    queryFn: () => api.phpInfo(version),
    enabled: php,
  });
  if (!php || !project.framework) return null;

  const framework = project.framework.framework;
  const entry = info.data?.versions?.find((v) => v.version === (version ?? info.data?.selected_version));
  const extensions = entry?.extensions ?? info.data?.extensions ?? [];
  return (
    <div className="wf-glass-panel mt-3 rounded-lg p-3 text-sm space-y-1">
      <p className="font-medium">Kesiapan ekstensi PHP ({framework})</p>
      {project.framework.php_version_constraint && (
        <p className="text-xs text-slate-700 dark:text-slate-300">Constraint versi PHP framework: <span className="font-mono">{project.framework.php_version_constraint}</span></p>
      )}
      {project.framework.recommended_php_version && (
        <p className="text-xs text-slate-700 dark:text-slate-300">Rekomendasi versi PHP: {project.framework.recommended_php_version}</p>
      )}
      {info.isLoading ? <p className="text-slate-700 dark:text-slate-300">Memuat info PHP…</p> : info.isError ? (
        <p className="text-danger">{errText(info.error, 'Gagal memuat info PHP.')}</p>
      ) : !info.data?.available && !entry?.available ? (
        <p className="text-warning">PHP belum tersedia{info.data?.error ? `: ${info.data.error}` : ''}. Kebutuhan ekstensi belum dapat diverifikasi.</p>
      ) : (() => {
        const reqs = evaluatePhpRequirements(framework, extensions);
        if (reqs.length === 0) {
          return <p className="text-slate-700 dark:text-slate-300">Framework ini tidak memiliki daftar ekstensi wajib khusus. PHP {info.data?.version ?? ''} tersedia dengan {extensions.length} ekstensi.</p>;
        }
        return (
          <ul className="space-y-1">
            {reqs.map((r) => (
              <li key={r.name} className="flex items-center justify-between gap-2">
                <span className="font-mono text-xs">{r.name}</span>
                {r.satisfied ? (
                  <span className="rounded-full bg-success/15 text-success px-2 py-0.5 text-xs font-medium">Terpenuhi</span>
                ) : (
                  <span className="rounded-full bg-danger/15 text-danger px-2 py-0.5 text-xs font-medium">Kurang</span>
                )}
              </li>
            ))}
          </ul>
        );
      })()}
    </div>
  );
}
