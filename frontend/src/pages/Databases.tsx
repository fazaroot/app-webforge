import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Database, Play, Square, Trash2 } from 'lucide-react';
import { api } from '../lib/api';
import type { CreateDatabaseBody, DatabaseKind, QueryResult, RuntimeInfo } from '../lib/types';
import { formatBytes, formatDateTime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, SkeletonRows, StatusPill, btnGhost, btnPrimary, inputCls } from '../components/ui';

const KIND_LABELS: Record<DatabaseKind, string> = {
  sqlite: 'SQLite',
  postgresql: 'PostgreSQL lokal',
  mariadb: 'MariaDB lokal',
  'external-mysql': 'MySQL eksternal',
  'external-postgresql': 'PostgreSQL eksternal',
};

const LOCAL_KINDS: DatabaseKind[] = ['postgresql', 'mariadb'];
const EXTERNAL_KINDS: DatabaseKind[] = ['external-mysql', 'external-postgresql'];

function runtimeAvailable(runtimes: RuntimeInfo[], pattern: RegExp): boolean {
  return runtimes.some((r) => r.available && pattern.test(`${r.id} ${r.name}`));
}

function defaultSql(kind: DatabaseKind): string {
  if (kind === 'sqlite') return 'SELECT name FROM sqlite_master WHERE type = \'table\';';
  if (kind === 'external-mysql' || kind === 'mariadb') return 'SHOW TABLES;';
  return 'SELECT table_name FROM information_schema.tables WHERE table_schema = \'public\';';
}

function defaultPort(kind: DatabaseKind): string {
  if (kind === 'external-mysql') return '3306';
  if (kind === 'external-postgresql') return '5432';
  return '';
}

export function Databases() {
  const qc = useQueryClient();
  const [name, setName] = useState('');
  const [kind, setKind] = useState<DatabaseKind>('sqlite');
  const [host, setHost] = useState('');
  const [port, setPort] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [databaseName, setDatabaseName] = useState('');
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sql, setSql] = useState(defaultSql('sqlite'));
  const [result, setResult] = useState<QueryResult | null>(null);
  const [queryError, setQueryError] = useState('');
  const [backupPath, setBackupPath] = useState('');
  const [notice, setNotice] = useState('');

  const dbs = useQuery({ queryKey: ['databases'], queryFn: api.databases });
  const runtimes = useQuery({ queryKey: ['runtimes'], queryFn: () => api.runtimes() });
  const tables = useQuery({
    queryKey: ['db-tables', selectedId],
    queryFn: () => api.dbTables(selectedId as string),
    enabled: selectedId !== null,
  });

  const runtimeList = runtimes.data ?? [];
  const pgLocalAvailable = runtimeAvailable(runtimeList, /postgres|pgsql/i);
  const mariaLocalAvailable = runtimeAvailable(runtimeList, /maria|mysql/i);
  const kindOptions: DatabaseKind[] = [
    'sqlite',
    ...(pgLocalAvailable ? ['postgresql' as DatabaseKind] : []),
    ...(mariaLocalAvailable ? ['mariadb' as DatabaseKind] : []),
    ...EXTERNAL_KINDS,
  ];
  const isExternal = EXTERNAL_KINDS.includes(kind);
  const portNum = Number(port);
  const portValid = port.trim() === '' || (Number.isInteger(portNum) && portNum >= 1 && portNum <= 65535);

  const onErr = (e: unknown) => setQueryError(e instanceof Error ? e.message : 'Operasi gagal');

  function buildBody(): CreateDatabaseBody {
    const body: CreateDatabaseBody = { name: name.trim(), kind };
    if (isExternal) {
      body.host = host.trim();
      if (port.trim()) body.port = portNum;
      body.username = username.trim();
      body.password = password;
      body.database_name = databaseName.trim();
    }
    return body;
  }

  function onKindChange(next: DatabaseKind) {
    setKind(next);
    setPort(defaultPort(next));
    setTestResult(null);
  }

  const create = useMutation({
    mutationFn: () => api.createDatabase(buildBody()),
    onSuccess: (d) => {
      setName('');
      setPassword('');
      setTestResult(null);
      setSelectedId(d.id);
      setSql(defaultSql(d.kind ?? 'sqlite'));
      setQueryError('');
      void qc.invalidateQueries({ queryKey: ['databases'] });
    },
    onError: onErr,
  });
  const testConnection = useMutation({
    mutationFn: () => api.testDatabaseConnection(buildBody()),
    onSuccess: (r) => { setTestResult(r); setQueryError(''); },
    onError: onErr,
  });
  const startStop = useMutation({
    mutationFn: ({ id, action }: { id: string; action: 'start' | 'stop' }) =>
      action === 'start' ? api.startDatabase(id) : api.stopDatabase(id),
    onSuccess: () => { setQueryError(''); void qc.invalidateQueries({ queryKey: ['databases'] }); },
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
  const selectedKind: DatabaseKind = selected?.kind ?? 'sqlite';

  function selectDatabase(id: string) {
    const db = (dbs.data ?? []).find((d) => d.id === id);
    setSelectedId(id);
    setResult(null);
    if (db) setSql(defaultSql(db.kind ?? 'sqlite'));
  }

  return (
    <div className="space-y-4">
      {queryError && <ErrorBox message={queryError} />}
      {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{notice}</div>}

      <Card title="Database">
        <div className="space-y-2 mb-3">
          <label className="block text-sm">
            <span className="text-xs font-medium text-slate-600 dark:text-slate-400">Jenis database</span>
            <select className={inputCls} value={kind} onChange={(e) => onKindChange(e.target.value as DatabaseKind)}>
              {kindOptions.map((k) => <option key={k} value={k}>{KIND_LABELS[k]}</option>)}
            </select>
          </label>
          {runtimes.isLoading && <p className="text-xs text-slate-600 dark:text-slate-400">Memuat runtime… pilihan PostgreSQL/MariaDB lokal muncul bila runtime-nya tersedia.</p>}
          {!runtimes.isLoading && (!pgLocalAvailable || !mariaLocalAvailable) && (
            <p className="text-xs text-slate-600 dark:text-slate-400">
              {!pgLocalAvailable && 'PostgreSQL lokal tidak ditampilkan karena runtime-nya belum tersedia. '}
              {!mariaLocalAvailable && 'MariaDB lokal tidak ditampilkan karena runtime-nya belum tersedia. '}
              Database eksternal tetap dapat dicoba karena koneksinya nyata ke server tujuan.
            </p>
          )}
          <input className={inputCls} placeholder="Nama database baru" value={name} onChange={(e) => setName(e.target.value)} />
          {isExternal && (
            <>
              <input className={inputCls} placeholder="Host, mis. 192.168.1.10 atau db.contoh.com" value={host} onChange={(e) => setHost(e.target.value)} />
              <input className={inputCls} inputMode="numeric" placeholder="Port" value={port} onChange={(e) => setPort(e.target.value)} />
              {!portValid && <p className="text-xs text-danger">Port harus bilangan 1–65535.</p>}
              <input className={inputCls} placeholder="Username" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" />
              <input className={inputCls} type="password" placeholder="Password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="off" />
              <input className={inputCls} placeholder="Nama database di server (database_name)" value={databaseName} onChange={(e) => setDatabaseName(e.target.value)} />
              <button type="button" className={btnGhost} disabled={!host.trim() || !portValid || testConnection.isPending} onClick={() => testConnection.mutate()}>
                {testConnection.isPending ? 'Menguji koneksi…' : 'Tes Koneksi'}
              </button>
              {testResult && (
                <div className={`rounded-lg border px-3 py-2 text-sm ${testResult.ok ? 'bg-success/10 border-success/40 text-success' : 'bg-red-50 dark:bg-red-950 border-danger/40 text-danger'}`} role="status">
                  {testResult.message}
                </div>
              )}
            </>
          )}
          <button type="button" className={btnPrimary}
            disabled={!name.trim() || (isExternal && (!host.trim() || !portValid)) || create.isPending}
            onClick={() => create.mutate()}>
            <Database size={15} /> {create.isPending ? 'Membuat…' : 'Buat'}
          </button>
        </div>
        {dbs.isLoading ? <SkeletonRows rows={3} /> : dbs.isError ? <ErrorBox message="Gagal memuat database." /> : (dbs.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Belum ada database. Buat database pertama di atas." />
        ) : (
          <div className="space-y-2">
            {dbs.data!.map((d) => {
              const dKind: DatabaseKind = d.kind ?? 'sqlite';
              const isLocalServer = LOCAL_KINDS.includes(dKind);
              return (
                <div key={d.id} className={`rounded-lg border px-3 py-2 ${selectedId === d.id ? 'border-primary' : 'border-slate-200 dark:border-slate-700'}`}>
                  <div className="flex items-center justify-between gap-2">
                    <button type="button" className="text-left min-w-0" onClick={() => selectDatabase(d.id)}>
                      <p className="font-medium text-sm">{d.name} <span className="text-slate-600 dark:text-slate-400">· {KIND_LABELS[dKind]}</span></p>
                      <p className="text-xs text-slate-600 dark:text-slate-400">
                        {dKind === 'sqlite' ? `${formatBytes(d.size_bytes)} · ` : ''}
                        {d.host ? `${d.host}${d.port ? `:${d.port}` : ''} · ` : ''}
                        {d.database_name ? `${d.database_name} · ` : ''}
                        {formatDateTime(d.created_at)}
                      </p>
                    </button>
                    <div className="flex items-center gap-2 shrink-0">
                      {d.status && <StatusPill status={d.status} />}
                      <button type="button" className={btnGhost} disabled={remove.isPending}
                        onClick={() => { if (window.confirm(dKind === 'sqlite' ? `Hapus database "${d.name}" beserta filenya?` : `Hapus database "${d.name}"?`)) remove.mutate(d.id); }}>
                        <Trash2 size={14} /> Hapus
                      </button>
                    </div>
                  </div>
                  {d.error_message && <p className="text-xs text-danger mt-1">{d.error_message}</p>}
                  {isLocalServer && (
                    <div className="flex flex-wrap gap-2 mt-2">
                      {d.status !== 'running' ? (
                        <button type="button" className={btnPrimary} disabled={startStop.isPending} onClick={() => startStop.mutate({ id: d.id, action: 'start' })}>
                          <Play size={14} /> Start
                        </button>
                      ) : (
                        <button type="button" className={btnGhost} disabled={startStop.isPending} onClick={() => startStop.mutate({ id: d.id, action: 'stop' })}>
                          <Square size={14} /> Stop
                        </button>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </Card>

      {selected && (
        <>
          <Card title={`Tabel: ${selected.name}`}>
            {tables.isLoading ? <SkeletonRows rows={2} /> : tables.isError ? (
              <ErrorBox message={tables.error instanceof Error ? tables.error.message : 'Gagal memuat tabel.'} />
            ) : (tables.data?.length ?? 0) === 0 ? (
              <EmptyBox message={selectedKind === 'sqlite'
                ? 'Belum ada tabel. Buat lewat SQL di bawah, mis. CREATE TABLE contoh (id INTEGER PRIMARY KEY, nama TEXT);'
                : 'Belum ada tabel yang dilaporkan Core untuk database ini.'} />
            ) : (
              <div className="space-y-3">
                {tables.data!.map((t) => (
                  <div key={t.name}>
                    <p className="font-mono text-sm font-medium">{t.name}</p>
                    <p className="text-xs text-slate-600 dark:text-slate-400">{t.columns.map((c) => `${c.name} ${c.type}${c.pk ? ' (PK)' : ''}`).join(' · ')}</p>
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
                <p className="text-xs text-slate-600 dark:text-slate-400 mb-1">Baris terpengaruh: {result.rows_affected} · Hasil: {result.rows.length} baris</p>
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
              {selectedKind !== 'sqlite' && (
                <p className="text-xs text-slate-600 dark:text-slate-400">Backup/restore untuk {KIND_LABELS[selectedKind]} bergantung dukungan Core; bila tidak didukung, pesan error backend ditampilkan apa adanya di atas.</p>
              )}
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
