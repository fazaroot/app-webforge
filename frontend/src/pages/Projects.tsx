import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { FolderOpen, FolderPlus, RefreshCw, Trash2 } from 'lucide-react';
import { api, isNativeWebForge, pickDirectory } from '../lib/api';
import type { Project } from '../lib/types';
import { formatDateTime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, LoadingBox, btnDanger, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { PhpReadiness } from '../components/PhpManager';
import { useAppStore } from '../store';

// Harus sama persis dengan core/src/templates.rs (TEMPLATES).
const TEMPLATES = ['static', 'native-php', 'flask', 'express', 'react-vite'];

function readinessLabel(r: string): string {
  const map: Record<string, string> = {
    ready: 'Siap jalan', needs_runtime: 'Butuh runtime', needs_dependencies: 'Butuh dependensi', unknown: 'Belum diketahui',
  };
  return map[r] ?? r;
}

export function Projects() {
  const qc = useQueryClient();
  const setActiveProjectId = useAppStore((s) => s.setActiveProjectId);
  const activeProjectId = useAppStore((s) => s.activeProjectId);
  const [name, setName] = useState('');
  const [template, setTemplate] = useState('static');
  const [regName, setRegName] = useState('');
  const [regPath, setRegPath] = useState('');
  const [selected, setSelected] = useState<Project | null>(null);
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [formError, setFormError] = useState('');
  const [picking, setPicking] = useState(false);
  const [importNotice, setImportNotice] = useState('');
  const native = isNativeWebForge();

  const projects = useQuery({ queryKey: ['projects'], queryFn: api.projects });

  const create = useMutation({
    mutationFn: () => api.createProject(name.trim(), template),
    onSuccess: (p) => { setName(''); setFormError(''); setSelected(p); setActiveProjectId(p.id); void qc.invalidateQueries({ queryKey: ['projects'] }); },
    onError: (e) => setFormError(e instanceof Error ? e.message : 'Gagal membuat proyek'),
  });
  const register = useMutation({
    mutationFn: () => api.registerProject(regName.trim(), regPath.trim()),
    onSuccess: (p) => { setRegName(''); setRegPath(''); setFormError(''); setImportNotice(''); setSelected(p); setActiveProjectId(p.id); void qc.invalidateQueries({ queryKey: ['projects'] }); },
    onError: (e) => setFormError(e instanceof Error ? e.message : 'Gagal mendaftarkan proyek'),
  });

  async function handlePickDirectory() {
    setPicking(true);
    setFormError('');
    setImportNotice('');
    try {
      const result = await pickDirectory();
      setRegPath(result.path);
      if (!regName.trim()) setRegName(result.name);
      setImportNotice(`Folder terimpor: ${result.fileCount} file disalin ke workspace. Periksa nama proyek, lalu daftarkan.`);
    } catch (e) {
      setFormError(e instanceof Error ? e.message : 'Gagal memilih folder');
    } finally {
      setPicking(false);
    }
  }
  const scan = useMutation({
    mutationFn: (id: string) => api.scanProject(id),
    onSuccess: (_fw, id) => { void qc.invalidateQueries({ queryKey: ['projects'] }); void api.project(id).then(setSelected).catch(() => undefined); },
    onError: (e) => setFormError(e instanceof Error ? e.message : 'Scan gagal'),
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteProject(id, deleteFiles),
    onSuccess: () => { setSelected(null); setDeleteFiles(false); void qc.invalidateQueries({ queryKey: ['projects'] }); },
    onError: (e) => setFormError(e instanceof Error ? e.message : 'Gagal menghapus proyek'),
  });

  return (
    <div className="space-y-4">
      {formError && <ErrorBox message={formError} />}
      <Card title="Buat proyek dari template">
        <div className="space-y-2">
          <input className={inputCls} placeholder="Nama proyek" value={name} onChange={(e) => setName(e.target.value)} />
          <select className={inputCls} value={template} onChange={(e) => setTemplate(e.target.value)}>
            {TEMPLATES.map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
          <button type="button" className={btnPrimary} disabled={!name.trim() || create.isPending} onClick={() => create.mutate()}>
            <FolderPlus size={16} /> {create.isPending ? 'Membuat…' : 'Buat proyek'}
          </button>
        </div>
      </Card>

      <Card title="Daftarkan folder yang sudah ada">
        <div className="space-y-2">
          {native && (
            <button type="button" className={btnGhost} disabled={picking} onClick={() => void handlePickDirectory()}>
              <FolderOpen size={16} /> {picking ? 'Mengimpor folder…' : 'Pilih Folder (SAF)'}
            </button>
          )}
          {importNotice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{importNotice}</div>}
          <input className={inputCls} placeholder="Nama proyek" value={regName} onChange={(e) => setRegName(e.target.value)} />
          <input className={inputCls} placeholder="Path absolut di dalam workspace" value={regPath} onChange={(e) => setRegPath(e.target.value)} />
          <button type="button" className={btnGhost} disabled={!regName.trim() || !regPath.trim() || register.isPending} onClick={() => register.mutate()}>
            {register.isPending ? 'Mendaftarkan…' : 'Daftarkan & scan'}
          </button>
        </div>
      </Card>

      <Card title="Daftar proyek">
        {projects.isLoading ? <LoadingBox /> : projects.isError ? <ErrorBox message="Gagal memuat proyek." /> : (projects.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Belum ada proyek. Buat dari template atau daftarkan folder di atas." />
        ) : (
          <div className="space-y-2">
            {projects.data!.map((p) => (
              <button key={p.id} type="button" onClick={() => { setSelected(p); setActiveProjectId(p.id); }}
                className={`w-full text-left rounded-lg border px-3 py-2 ${activeProjectId === p.id ? 'border-primary' : 'border-slate-200 dark:border-slate-700'}`}>
                <p className="font-medium text-sm">{p.name}</p>
                <p className="text-xs text-slate-500 dark:text-slate-400 break-all">{p.path}</p>
                <p className="text-xs text-slate-500 dark:text-slate-400">
                  {p.framework ? `${p.framework.framework} · ${readinessLabel(p.framework.readiness)}` : 'Belum di-scan'}
                </p>
              </button>
            ))}
          </div>
        )}
      </Card>

      {selected && (
        <Card title={`Detail: ${selected.name}`}>
          <dl className="text-sm space-y-1">
            <div><dt className="text-xs text-slate-500">Slug</dt><dd>{selected.slug}</dd></div>
            <div><dt className="text-xs text-slate-500">Path</dt><dd className="break-all">{selected.path}</dd></div>
            <div><dt className="text-xs text-slate-500">Document root</dt><dd className="break-all">{selected.document_root}</dd></div>
            <div><dt className="text-xs text-slate-500">Dibuat</dt><dd>{formatDateTime(selected.created_at)}</dd></div>
          </dl>
          {selected.framework ? (
            <div className="mt-3 rounded-lg bg-slate-50 dark:bg-darkbg p-3 text-sm space-y-1">
              <p><span className="text-slate-500">Framework:</span> {selected.framework.framework} ({selected.framework.language}{selected.framework.version ? ` ${selected.framework.version}` : ''})</p>
              <p><span className="text-slate-500">Runtime disarankan:</span> {selected.framework.recommended_runtime}</p>
              <p><span className="text-slate-500">Document root disarankan:</span> {selected.framework.recommended_document_root}</p>
              <p><span className="text-slate-500">Kesiapan:</span> {readinessLabel(selected.framework.readiness)}</p>
              {selected.framework.detected_files.length > 0 && <p><span className="text-slate-500">File terdeteksi:</span> {selected.framework.detected_files.join(', ')}</p>}
              {selected.framework.notes.map((n) => <p key={n} className="text-warning text-xs">{n}</p>)}
            </div>
          ) : <p className="text-sm text-slate-500 mt-2">Framework belum terdeteksi. Jalankan scan.</p>}
          <PhpReadiness project={selected} />
          <div className="flex flex-wrap gap-2 mt-3">
            <button type="button" className={btnGhost} disabled={scan.isPending} onClick={() => scan.mutate(selected.id)}>
              <RefreshCw size={15} /> {scan.isPending ? 'Memindai…' : 'Scan ulang'}
            </button>
            <label className="inline-flex items-center gap-1.5 text-sm">
              <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
              Hapus file juga
            </label>
            <button type="button" className={btnDanger} disabled={remove.isPending}
              onClick={() => { if (window.confirm(`Hapus proyek "${selected.name}"?${deleteFiles ? ' Folder proyek juga akan dihapus.' : ' File tetap disimpan.'}`)) remove.mutate(selected.id); }}>
              <Trash2 size={15} /> Hapus
            </button>
          </div>
        </Card>
      )}
    </div>
  );
}
