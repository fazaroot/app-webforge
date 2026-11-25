import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Database, Play, Trash2 } from 'lucide-react';
import { api } from '../lib/api';
import type { QueryResult } from '../lib/types';
import { formatBytes, formatDateTime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, LoadingBox, btnGhost, btnPrimary, inputCls } from '../components/ui';

export function Databases() {
  const qc = useQueryClient();
  const [name, setName] = useState('');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sql, setSql] = useState('SELECT name FROM sqlite_master WHERE type = \'table\';');
  const [result, setResult] = useState<QueryResult | null>(null);
  const [queryError, setQueryError] = useState('');
  const [backupPath, setBackupPath] = useState('');
  const [notice, setNotice] = useState('');

  const dbs = useQuery({ queryKey: ['databases'], queryFn: api.databases });
  const tables = useQuery({
    queryKey: ['db-tables', selectedId],
    queryFn: () => api.dbTables(selectedId as string),
    enabled: selectedId !== null,
  });

  const onErr = (e: unknown) => setQueryError(e instanceof Error ? e.message : 'Operasi gagal');

  const create = useMutation({
    mutationFn: () => api.createDatabase(name.trim()),
    onSuccess: (d) => { setName(''); setSelectedId(d.id); setQueryError(''); void qc.invalidateQueries({ queryKey: ['databases'] }); },
    onError: onErr,
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteDatabase(id),
    onSuccess: () => { setSelectedId(null); setResult(null); void qc.invalidateQueries({ queryKey: ['databases'] }); },
    onError: onErr,
  });
  const runQuery = useMutation({
    mutationFn: () => api.dbQuery(selectedId as string, sql),
    onSuccess: (r) => { setResult(r); setQueryError(''); void qc.invalidateQueries({ queryKey: ['db-tables', selectedId] }); void qc.invalidateQueries({ queryKey: ['databases'] }); },
    onError: onErr,
  });
  const backup = useMutation({
    mutationFn: () => api.dbBackup(selectedId as string),
    onSuccess: (b) => { setBackupPath(b.backup_path); setNotice(`Backup dibuat ${formatDateTime(b.created_at)}: ${b.backup_path}`); setQueryError(''); },
    onError: onErr,
  });
  const restore = useMutation({
    mutationFn: () => api.dbRestore(selectedId as string, backupPath.trim()),
    onSuccess: () => { setNotice('Restore selesai.'); setQueryError(''); void qc.invalidateQueries({ queryKey: ['db-tables', selectedId] }); void qc.invalidateQueries({ queryKey: ['databases'] }); },
    onError: onErr,
  });

  const selected = (dbs.data ?? []).find((d) => d.id === selectedId);

  return (
    <div className="space-y-4">
      {queryError && <ErrorBox message={queryError} />}
      {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{notice}</div>}

      <Card title="Database SQLite">
        <div className="flex gap-2 mb-3">
          <input className={inputCls} placeholder="Nama database baru" value={name} onChange={(e) => setName(e.target.value)} />
          <button type="button" className={btnPrimary} disabled={!name.trim() || create.isPending} onClick={() => create.mutate()}>
            <Database size={15} /> Buat
          </button>
        </div>
        {dbs.isLoading ? <LoadingBox /> : dbs.isError ? <ErrorBox message="Gagal memuat database." /> : (dbs.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Belum ada database. Buat database SQLite pertama di atas." />
        ) : (
          <div className="space-y-2">
            {dbs.data!.map((d) => (
              <div key={d.id} className={`rounded-lg border px-3 py-2 ${selectedId === d.id ? 'border-primary' : 'border-slate-200 dark:border-slate-700'}`}>
                <div className="flex items-center justify-between gap-2">
                  <button type="button" className="text-left" onClick={() => setSelectedId(d.id)}>
                    <p className="font-medium text-sm">{d.name}</p>
                    <p className="text-xs text-slate-500">{formatBytes(d.size_bytes)} · {formatDateTime(d.created_at)}</p>
                  </button>
                  <button type="button" className={btnGhost} disabled={remove.isPending}
                    onClick={() => { if (window.confirm(`Hapus database "${d.name}" beserta filenya?`)) remove.mutate(d.id); }}>
                    <Trash2 size={14} /> Hapus
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      {selected && (
        <>
          <Card title={`Tabel: ${selected.name}`}>
            {tables.isLoading ? <LoadingBox /> : tables.isError ? <ErrorBox message="Gagal memuat tabel." /> : (tables.data?.length ?? 0) === 0 ? (
              <EmptyBox message="Belum ada tabel. Buat lewat SQL di bawah, mis. CREATE TABLE contoh (id INTEGER PRIMARY KEY, nama TEXT);" />
            ) : (
              <div className="space-y-3">
                {tables.data!.map((t) => (
                  <div key={t.name}>
                    <p className="font-mono text-sm font-medium">{t.name}</p>
                    <p className="text-xs text-slate-500">{t.columns.map((c) => `${c.name} ${c.type}${c.pk ? ' (PK)' : ''}`).join(' · ')}</p>
                  </div>
                ))}
              </div>
            )}
          </Card>

          <Card title="SQL editor">
            <textarea className={`${inputCls} font-mono`} rows={5} value={sql} onChange={(e) => setSql(e.target.value)} />
            <button type="button" className={`${btnPrimary} mt-2`} disabled={!sql.trim() || runQuery.isPending} onClick={() => runQuery.mutate()}>
              <Play size={15} /> {runQuery.isPending ? 'Menjalankan…' : 'Jalankan SQL'}
            </button>
            {result && (
              <div className="mt-3">
                <p className="text-xs text-slate-500 mb-1">Baris terpengaruh: {result.rows_affected} · Hasil: {result.rows.length} baris</p>
                {result.columns.length > 0 && (
                  <div className="overflow-auto rounded-lg border border-slate-200 dark:border-slate-700">
                    <table className="text-xs w-full">
                      <thead><tr className="bg-slate-50 dark:bg-darkbg">{result.columns.map((c) => <th key={c} className="px-2 py-1 text-left font-medium">{c}</th>)}</tr></thead>
                      <tbody>
                        {result.rows.map((row, i) => (
                          <tr key={i} className="border-t border-slate-100 dark:border-slate-800">
                            {row.map((cell, j) => <td key={j} className="px-2 py-1 font-mono">{cell === null ? 'NULL' : String(cell)}</td>)}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            )}
          </Card>

          <Card title="Backup & restore">
            <div className="space-y-2">
              <button type="button" className={btnGhost} disabled={backup.isPending} onClick={() => backup.mutate()}>
                {backup.isPending ? 'Membuat backup…' : 'Buat backup'}
              </button>
              <input className={inputCls} placeholder="Path backup (di folder backups)" value={backupPath} onChange={(e) => setBackupPath(e.target.value)} />
              <button type="button" className={btnGhost} disabled={!backupPath.trim() || restore.isPending}
                onClick={() => { if (window.confirm('Restore akan menimpa isi database saat ini. Lanjutkan?')) restore.mutate(); }}>
                {restore.isPending ? 'Me-restore…' : 'Restore dari backup'}
              </button>
            </div>
          </Card>
        </>
      )}
    </div>
  );
}
