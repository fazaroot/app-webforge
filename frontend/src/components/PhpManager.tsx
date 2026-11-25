import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { api } from '../lib/api';
import { evaluatePhpRequirements, isPhpProject } from '../lib/php';
import type { PhpCliResult, PhpSettings, Project } from '../lib/types';
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

export function PhpManager() {
  const qc = useQueryClient();
  const projectId = useAppStore((s) => s.activeProjectId);
  const [form, setForm] = useState<PhpSettings>(EMPTY_SETTINGS);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [cliArgs, setCliArgs] = useState('-v');
  const [cliResult, setCliResult] = useState<PhpCliResult | null>(null);

  const info = useQuery({ queryKey: ['php-info'], queryFn: api.phpInfo });
  const settings = useQuery({ queryKey: ['php-settings'], queryFn: api.phpSettings });

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
    onError: (e) => { setNotice(''); setError(e instanceof Error ? e.message : 'Gagal menyimpan pengaturan PHP'); },
  });

  const runCli = useMutation({
    mutationFn: () => api.phpCli(projectId as string, tokenizeArgs(cliArgs)),
    onSuccess: (r) => { setCliResult(r); setError(''); },
    onError: (e) => { setCliResult(null); setError(e instanceof Error ? e.message : 'PHP CLI gagal dijalankan'); },
  });

  const textField = (key: 'memory_limit' | 'upload_max_filesize' | 'post_max_size' | 'date_timezone', label: string, placeholder: string) => (
    <label className="block text-sm">
      <span className="text-xs font-medium text-slate-500 dark:text-slate-400">{label}</span>
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
          <ErrorBox message={info.error instanceof Error ? info.error.message : 'Gagal memuat info PHP.'} />
        ) : !info.data?.available ? (
          <div className="space-y-1">
            <p className="text-sm"><span className="rounded-full bg-warning/15 text-warning px-2 py-0.5 text-xs font-medium">PHP belum tersedia</span></p>
            <p className="text-sm text-slate-500 dark:text-slate-400">
              {info.data?.error ?? 'Runtime PHP belum tersedia di perangkat ini. Server PHP akan ditolak Core sampai runtime tersedia.'}
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            <p className="text-sm">
              <span className="rounded-full bg-success/15 text-success px-2 py-0.5 text-xs font-medium">PHP tersedia</span>{' '}
              {info.data.version ? `Versi ${info.data.version}` : 'Versi tidak diketahui'}
            </p>
            {info.data.binary_path && <p className="text-xs text-slate-500 dark:text-slate-400 break-all">Binary: {info.data.binary_path}</p>}
            <p className="text-xs font-medium text-slate-500 dark:text-slate-400">Ekstensi terpasang ({info.data.extensions.length})</p>
            {info.data.extensions.length === 0 ? (
              <p className="text-sm text-slate-500 dark:text-slate-400">Tidak ada ekstensi yang dilaporkan Core.</p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {info.data.extensions.map((e) => (
                  <span key={e} className="rounded-full bg-slate-100 dark:bg-darkbg border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-xs font-mono">{e}</span>
                ))}
              </div>
            )}
          </div>
        )}
        <button type="button" className={`${btnGhost} mt-3`} disabled={info.isFetching} onClick={() => void info.refetch()}>
          <RefreshCw size={14} /> {info.isFetching ? 'Memuat ulang…' : 'Muat ulang info PHP'}
        </button>
      </Card>

      <Card title="Pengaturan php.ini">
        {settings.isLoading ? <LoadingBox message="Memuat pengaturan PHP…" /> : settings.isError ? (
          <ErrorBox message={settings.error instanceof Error ? settings.error.message : 'Gagal memuat pengaturan PHP.'} />
        ) : (
          <div className="space-y-2">
            {textField('memory_limit', 'memory_limit', 'mis. 256M')}
            {textField('upload_max_filesize', 'upload_max_filesize', 'mis. 32M')}
            {textField('post_max_size', 'post_max_size', 'mis. 32M')}
            <label className="block text-sm">
              <span className="text-xs font-medium text-slate-500 dark:text-slate-400">max_execution_time (detik)</span>
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
            <p className="text-xs text-slate-500 dark:text-slate-400">Pengaturan disimpan oleh Core dan dipakai server PHP berikutnya. Server PHP yang sedang berjalan mungkin perlu di-restart.</p>
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
          {!projectId && <p className="text-xs text-slate-500 dark:text-slate-400">Pilih proyek aktif untuk menjalankan PHP CLI di folder proyek itu.</p>}
          {cliResult && (
            <div className="rounded-lg bg-slate-50 dark:bg-darkbg p-2 space-y-1">
              <p className="text-xs text-slate-500 dark:text-slate-400">
                Exit code: {cliResult.exit_code ?? 'tidak ada'}{cliResult.timed_out ? ' · timeout' : ''}
              </p>
              {cliResult.stdout && <pre className="text-xs font-mono whitespace-pre-wrap break-all">{cliResult.stdout}</pre>}
              {cliResult.stderr && <pre className="text-xs font-mono whitespace-pre-wrap break-all text-danger">{cliResult.stderr}</pre>}
              {!cliResult.stdout && !cliResult.stderr && <p className="text-xs text-slate-500 dark:text-slate-400">Tidak ada output.</p>}
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
  const info = useQuery({ queryKey: ['php-info'], queryFn: api.phpInfo, enabled: php });
  if (!php || !project.framework) return null;

  const framework = project.framework.framework;
  return (
    <div className="mt-3 rounded-lg bg-slate-50 dark:bg-darkbg p-3 text-sm space-y-1">
      <p className="font-medium">Kesiapan ekstensi PHP ({framework})</p>
      {info.isLoading ? <p className="text-slate-500 dark:text-slate-400">Memuat info PHP…</p> : info.isError ? (
        <p className="text-danger">{info.error instanceof Error ? info.error.message : 'Gagal memuat info PHP.'}</p>
      ) : !info.data?.available ? (
        <p className="text-warning">PHP belum tersedia{info.data?.error ? `: ${info.data.error}` : ''}. Kebutuhan ekstensi belum dapat diverifikasi.</p>
      ) : (() => {
        const reqs = evaluatePhpRequirements(framework, info.data.extensions);
        if (reqs.length === 0) {
          return <p className="text-slate-500 dark:text-slate-400">Framework ini tidak memiliki daftar ekstensi wajib khusus. PHP {info.data.version ?? ''} tersedia dengan {info.data.extensions.length} ekstensi.</p>;
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
