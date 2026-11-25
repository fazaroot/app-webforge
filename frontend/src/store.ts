import { create } from 'zustand';

export type ThemeMode = 'light' | 'dark' | 'system';
export type Section =
  | 'dashboard' | 'projects' | 'servers' | 'terminal' | 'databases' | 'files'
  | 'editor' | 'monitoring' | 'logs' | 'tools' | 'security' | 'settings';

interface AppState {
  theme: ThemeMode;
  section: Section;
  activeProjectId: string | null;
  activeServerId: string | null;
  setTheme: (t: ThemeMode) => void;
  setSection: (s: Section) => void;
  setActiveProjectId: (id: string | null) => void;
  setActiveServerId: (id: string | null) => void;
}

function initialTheme(): ThemeMode {
  const saved = localStorage.getItem('webforge_theme');
  return saved === 'light' || saved === 'dark' || saved === 'system' ? saved : 'system';
}

export function applyTheme(theme: ThemeMode): void {
  const dark = theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.classList.toggle('dark', dark);
}

export const useAppStore = create<AppState>((set) => ({
  theme: initialTheme(),
  section: 'dashboard',
  activeProjectId: localStorage.getItem('webforge_active_project'),
  activeServerId: null,
  setTheme: (theme) => {
    localStorage.setItem('webforge_theme', theme);
    applyTheme(theme);
    set({ theme });
  },
  setSection: (section) => set({ section }),
  setActiveProjectId: (id) => {
    if (id) localStorage.setItem('webforge_active_project', id);
    else localStorage.removeItem('webforge_active_project');
    set({ activeProjectId: id });
  },
  setActiveServerId: (id) => set({ activeServerId: id }),
}));
