import { useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowLeft, Copy, File, Folder, FolderPlus, MoveRight, Pencil, Search, Trash2, Upload } from 'lucide-react';
import { api } from '../lib/api';
import type { FileEntry } from '../lib/types';
import { formatBytes, formatDateTime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, LoadingBox, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { ProjectPicker } from '../components/ProjectPicker';
import { useAppStore } from '../store';

function parentPath(p: string): string {
  const idx = p.lastIndexOf('/');
  return idx <= 0 ? '' : p.slice(0, idx);
}

export function Files() {
  const qc = useQueryClient();
  const projectId = useAppStore((s) => s.activeProjectId);
  const [path, setPath] = useState('');
  const [error, setError] = useState('');
  const [newDir, setNewDir] = useState('');
  const [searchQ, setSearchQ] = useState('');
  const [searching, setSearching] = useState(false);
  const [dirSize, setDirSize] = useState<string | null>(null);
  const [action, setAction] = useState<{ entry: FileEntry; kind: 'rename' | 'copy' | 'move'; target: string } | null>(null);
  const [preview, setPreview] = useState<{ path: string; content: string } | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const listing = useQuery({
    queryKey: ['files', projectId, path],
    queryFn: () => api.filesList(projectId as string, path),
    enabled: projectId !== null,
  });
  const search = useQuery({
    queryKey: ['files-search', projectId, searchQ],
    queryFn: () => api.filesSearch(projectId as string, searchQ),
    enabled: projectId !== null && searching && searchQ.trim() !== '',
  });

  const invalidate = () => { void qc.invalidateQueries({ queryKey: ['files', projectId] }); };
  const onErr = (e: unknown) => setError(e instanceof Error ? e.message : 'Operasi gagal');

  const mkdir = useMutation({
    mutationFn: () => api.filesMkdir(projectId as string, path ? `${path}/${newDir.trim()}` : newDir.trim()),
    onSuccess: () => { setNewDir(''); setError(''); invalidate(); },
    onError: onErr,
  });
  const remove = useMutation({
    mutationFn: (p: string) => api.filesDelete(projectId as string, p),
    onSuccess: () => { setError(''); invalidate(); },
    onError: onErr,
  });
  const doAction = useMutation({
    mutationFn: () => {
      const a = action as { entry: FileEntry; kind: 'rename' | 'copy' | 'move'; target: string };
      if (a.kind === 'rename') return api.filesRename(projectId as string, a.entry.path, a.target.trim());
      if (a.kind === 'copy') return api.filesCopy(projectId as string, a.entry.path, a.target.trim());
      return api.filesMove(projectId as string, a.entry.path, a.target.trim());
    },
    onSuccess: () => { setAction(null); setError(''); invalidate(); },
    onError: onErr,
  });
  const size = useMutation({
    mutationFn: (p: string) => api.filesDirSize(projectId as string, p),
    onSuccess: (r) => setDirSize(`${r.path || '(root)'}: ${formatBytes(r.bytes)} · ${r.files} file`),
    onError: onErr,
  });
  const upload = useMutation({
    mutationFn: async (file: File) => {
      const content = await file.text();
      const target = path ? `${path}/${file.name}` : file.name;
      return api.filesWrite(projectId as string, target, content);
    },
    onSuccess: () => { setError(''); invalidate(); },
    onError: onErr,
  });

  async function openPreview(entry: FileEntry) {
    try {
      const r = await api.filesRead(projectId as string, entry.path);
      setPreview({ path: r.path, content: r.content });
      setError('');
    } catch (e) {
      onErr(e);
    }
  }

  if (!projectId) {
    return (
      <div className="space-y-4">
        <ProjectPicker />
        <EmptyBox message="Pilih proyek dulu untuk membuka file manager." />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <ProjectPicker />
      {error && <ErrorBox message={error} />}
      {dirSize && <div className="rounded-lg bg-primary/10 border border-primary/30 text-sm px-3 py-2">{dirSize}</div>}

      <Card title={`Folder: /${path}`}>
        <div className="flex flex-wrap gap-2 mb-3">
          <button type="button" className={btnGhost} disabled={path === ''} onClick={() => setPath(parentPath(path))}>
            <ArrowLeft size={14} /> Ke atas
          </button>
          <button type="button" className={btnGhost} onClick={() => size.mutate(path)}>Ukuran folder</button>
          <button type="button" className={btnGhost} onClick={() => fileInput.current?.click()}>
            <Upload size={14} /> Upload file teks
          </button>
          <input ref={fileInput} type="file" className="hidden" onChange={(e) => { const f = e.target.files?.[0]; if (f) upload.mutate(f); e.target.value = ''; }} />
        </div>
        <div className="flex gap-2 mb-3">
          <input className={inputCls} placeholder="Nama folder baru" value={newDir} onChange={(e) => setNewDir(e.target.value)} />
          <button type="button" className={btnPrimary} disabled={!newDir.trim() || mkdir.isPending} onClick={() => mkdir.mutate()}>
            <FolderPlus size={15} /> Buat folder
          </button>
        </div>
        <div className="flex gap-2 mb-3">
          <input className={inputCls} placeholder="Cari nama file/folder…" value={searchQ} onChange={(e) => setSearchQ(e.target.value)} />
          <button type="button" className={btnGhost} disabled={!searchQ.trim()} onClick={() => setSearching(true)}><Search size={15} /> Cari</button>
          {searching && <button type="button" className={btnGhost} onClick={() => { setSearching(false); setSearchQ(''); }}>Reset</button>}
        </div>

        {searching ? (
          search.isLoading ? <LoadingBox /> : search.isError ? <ErrorBox message="Pencarian gagal." /> : (search.data?.length ?? 0) === 0 ? (
            <EmptyBox message="Tidak ada hasil untuk pencarian ini." />
          ) : (
            <div className="space-y-1">
              {search.data!.map((h) => (
                <button key={h.path} type="button" className="w-full text-left rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2 text-sm"
                  onClick={() => { if (h.is_dir) setPath(h.path); else void openPreview({ name: h.name, path: h.path, is_dir: false, size: 0, modified_at: '' }); }}>
                  {h.is_dir ? <Folder size={14} className="inline mr-1" /> : <File size={14} className="inline mr-1" />}{h.path}
                </button>
              ))}
            </div>
          )
        ) : listing.isLoading ? <LoadingBox /> : listing.isError ? <ErrorBox message="Gagal memuat daftar file." /> : (listing.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Folder ini kosong." />
        ) : (
          <div className="space-y-1">
            {listing.data!.map((entry) => (
              <div key={entry.path} className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2">
                <div className="flex items-center justify-between gap-2">
                  <button type="button" className="text-left min-w-0" onClick={() => { if (entry.is_dir) setPath(entry.path); else void openPreview(entry); }}>
                    <p className="text-sm font-medium truncate">{entry.is_dir ? <Folder size={14} className="inline mr-1 text-primary" /> : <File size={14} className="inline mr-1" />}{entry.name}</p>
                    <p className="text-xs text-slate-500">{entry.is_dir ? 'Folder' : formatBytes(entry.size)} · {formatDateTime(entry.modified_at)}</p>
                  </button>
                  <div className="flex gap-1 shrink-0">
                    <button type="button" className="p-1.5" title="Ganti nama" onClick={() => setAction({ entry, kind: 'rename', target: entry.path })}><Pencil size={15} /></button>
                    <button type="button" className="p-1.5" title="Salin" onClick={() => setAction({ entry, kind: 'copy', target: `${entry.path}.bak` })}><Copy size={15} /></button>
                    <button type="button" className="p-1.5" title="Pindah" onClick={() => setAction({ entry, kind: 'move', target: entry.path })}><MoveRight size={15} /></button>
                    <button type="button" className="p-1.5 text-danger" title="Hapus"
                      onClick={() => { if (window.confirm(`Hapus "${entry.path}"? Tindakan ini tidak dapat dibatalkan.`)) remove.mutate(entry.path); }}>
                      <Trash2 size={15} />
                    </button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}

        {action && (
          <div className="mt-3 rounded-lg bg-slate-50 dark:bg-darkbg p-3 space-y-2">
            <p className="text-sm font-medium">{action.kind === 'rename' ? 'Ganti nama' : action.kind === 'copy' ? 'Salin' : 'Pindah'}: {action.entry.path}</p>
            <input className={inputCls} value={action.target} onChange={(e) => setAction({ ...action, target: e.target.value })} placeholder="Path tujuan (relatif proyek)" />
            <div className="flex gap-2">
              <button type="button" className={btnPrimary} disabled={!action.target.trim() || doAction.isPending} onClick={() => doAction.mutate()}>
                {doAction.isPending ? 'Memproses…' : 'Terapkan'}
              </button>
              <button type="button" className={btnGhost} onClick={() => setAction(null)}>Batal</button>
            </div>
          </div>
        )}
      </Card>

      {preview && (
        <Card title={`Pratinjau: ${preview.path}`} right={<button type="button" className="text-sm text-primary" onClick={() => setPreview(null)}>Tutup</button>}>
          <pre className="text-xs font-mono whitespace-pre-wrap max-h-80 overflow-auto rounded-lg bg-slate-50 dark:bg-darkbg p-3">{preview.content}</pre>
        </Card>
      )}
      <p className="text-xs text-slate-500">Upload menulis file teks lewat API tulis file; file biner tidak didukung untuk pratinjau/edit (API menolak file biner).</p>
    </div>
  );
}
