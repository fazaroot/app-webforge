import { useQuery } from '@tanstack/react-query';
import { api } from '../lib/api';
import { useAppStore } from '../store';
import { inputCls } from './ui';

export function ProjectPicker() {
  const activeProjectId = useAppStore((s) => s.activeProjectId);
  const setActiveProjectId = useAppStore((s) => s.setActiveProjectId);
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.projects });
  return (
    <label className="block">
      <span className="text-xs font-medium text-slate-600 dark:text-slate-400">Proyek aktif</span>
      <select
        className={inputCls}
        value={activeProjectId ?? ''}
        onChange={(e) => setActiveProjectId(e.target.value || null)}
      >
        <option value="">— Pilih proyek —</option>
        {(projects.data ?? []).map((p) => (
          <option key={p.id} value={p.id}>{p.name}</option>
        ))}
      </select>
    </label>
  );
}
