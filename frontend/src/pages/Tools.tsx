import { useQuery } from '@tanstack/react-query';
import { api } from '../lib/api';
import { formatDateTime } from '../lib/format';
import { Card, EmptyBox, ErrorBox, LoadingBox, StatusPill } from '../components/ui';
import { RuntimeBadge } from '../components/RuntimeBadge';
import { ProjectPicker } from '../components/ProjectPicker';
import { useAppStore } from '../store';

export function Tools() {
  const projectId = useAppStore((s) => s.activeProjectId);
  const git = useQuery({
    queryKey: ['git-status', projectId],
    queryFn: () => api.gitStatus(projectId as string),
    enabled: projectId !== null,
  });
  const diag = useQuery({ queryKey: ['diagnostics'], queryFn: api.diagnostics });
  const runtimes = useQuery({ queryKey: ['runtimes'], queryFn: api.runtimes });

  return (
    <div className="space-y-4">
      <Card title="Status Git">
        <ProjectPicker />
        {!projectId ? (
          <EmptyBox message="Pilih proyek untuk melihat status Git-nya." />
        ) : git.isLoading ? <LoadingBox /> : git.isError ? <ErrorBox message="Gagal memuat status Git." /> : !git.data?.available ? (
          <p className="text-sm text-warning mt-2">{git.data?.reason ?? 'Git tidak tersedia di perangkat ini.'}</p>
        ) : (
          <div className="text-sm mt-2 space-y-1">
            <p>Branch: <span className="font-mono">{git.data.branch ?? '-'}</span></p>
            <p>{git.data.clean ? 'Working tree bersih.' : `${git.data.entries.length} perubahan:`}</p>
            {git.data.entries.map((e) => <p key={e} className="font-mono text-xs">{e}</p>)}
          </div>
        )}
      </Card>

      <Card title="Diagnostik sistem">
        {diag.isLoading ? <LoadingBox /> : diag.isError ? <ErrorBox message="Gagal memuat diagnostik." /> : (
          <>
            <p className="text-xs text-slate-500 mb-2">Dihasilkan: {formatDateTime(diag.data?.generated_at)}</p>
            <div className="space-y-2">
              {(diag.data?.checks ?? []).map((c) => (
                <div key={c.name} className="flex items-start justify-between gap-2 rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-2">
                  <div>
                    <p className="text-sm font-medium">{c.name}</p>
                    <p className="text-xs text-slate-500">{c.detail}</p>
                  </div>
                  <StatusPill status={c.status} />
                </div>
              ))}
            </div>
            <p className="text-sm font-medium mt-3 mb-1">Penyimpanan</p>
            <pre className="text-xs font-mono whitespace-pre-wrap rounded-lg bg-slate-50 dark:bg-darkbg p-2">{JSON.stringify(diag.data?.storage ?? {}, null, 2)}</pre>
            <p className="text-sm font-medium mt-3 mb-1">Port</p>
            <pre className="text-xs font-mono whitespace-pre-wrap rounded-lg bg-slate-50 dark:bg-darkbg p-2">{JSON.stringify(diag.data?.ports ?? [], null, 2)}</pre>
          </>
        )}
      </Card>

      <Card title="Runtime terdaftar">
        {runtimes.isLoading ? <LoadingBox /> : runtimes.isError ? <ErrorBox message="Gagal memuat runtime." /> : (
          <div className="space-y-2">{(runtimes.data ?? []).map((r) => <RuntimeBadge key={r.id} runtime={r} />)}</div>
        )}
      </Card>
    </div>
  );
}
