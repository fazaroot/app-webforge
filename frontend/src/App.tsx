import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  Activity, Code2, Database, FolderGit2, FolderOpen, LayoutDashboard, Menu, Moon, ScrollText,
  Server, Settings as SettingsIcon, ShieldCheck, Sun, TerminalSquare, Wrench, X,
} from 'lucide-react';
import { api } from './lib/api';
import { useCoreBootstrap } from './lib/useCoreBootstrap';
import { OfflineBanner } from './components/OfflineBanner';
import { SkeletonCard } from './components/ui';
import { useAppStore, type Section } from './store';
import { Dashboard } from './pages/Dashboard';
import { Projects } from './pages/Projects';
import { Servers } from './pages/Servers';
import { Terminal } from './pages/Terminal';
import { Databases } from './pages/Databases';
import { Files } from './pages/Files';
import { Editor } from './pages/Editor';
import { Monitoring } from './pages/Monitoring';
import { Logs } from './pages/Logs';
import { Tools } from './pages/Tools';
import { Security } from './pages/Security';
import { Settings } from './pages/Settings';

const SECTIONS: { id: Section; label: string; icon: typeof LayoutDashboard }[] = [
  { id: 'dashboard', label: 'Dashboard', icon: LayoutDashboard },
  { id: 'projects', label: 'Proyek', icon: FolderGit2 },
  { id: 'servers', label: 'Server', icon: Server },
  { id: 'terminal', label: 'Terminal', icon: TerminalSquare },
  { id: 'databases', label: 'Database', icon: Database },
  { id: 'files', label: 'File Manager', icon: FolderOpen },
  { id: 'editor', label: 'Code Editor', icon: Code2 },
  { id: 'monitoring', label: 'Monitoring', icon: Activity },
  { id: 'logs', label: 'Log', icon: ScrollText },
  { id: 'tools', label: 'Tools', icon: Wrench },
  { id: 'security', label: 'Keamanan', icon: ShieldCheck },
  { id: 'settings', label: 'Pengaturan', icon: SettingsIcon },
];

const BOTTOM: Section[] = ['dashboard', 'projects', 'servers', 'terminal'];

export default function App() {
  const section = useAppStore((s) => s.section);
  const setSection = useAppStore((s) => s.setSection);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia('(prefers-color-scheme: dark)').matches,
  );

  const health = useQuery({ queryKey: ['health'], queryFn: api.health, refetchInterval: 15000, retry: 1 });
  const boot = useCoreBootstrap();
  // Native: selama bootstrap berjalan tampilkan skeleton "Menyalakan core…",
  // banner offline hanya setelah polling backoff habis. Web: perilaku lama.
  const coreStarting = boot.phase === 'starting';
  const coreFailed = boot.phase === 'failed' && !health.isSuccess;
  const showOfflineBanner = coreFailed || (boot.phase === 'web' && health.isError);

  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => {
      setSystemDark(mq.matches);
      const t = useAppStore.getState().theme;
      if (t === 'system') document.documentElement.classList.toggle('dark', mq.matches);
    };
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);

  const isDark = theme === 'dark' || (theme === 'system' && systemDark);

  function toggleTheme() {
    // Tema system: tentukan mode efektif saat ini, lalu set eksplisit lawannya.
    setTheme(isDark ? 'light' : 'dark');
  }

  const active = SECTIONS.find((s) => s.id === section);

  function go(id: Section) {
    setSection(id);
    setDrawerOpen(false);
  }

  return (
    <div className="wf-app min-h-full text-ink dark:text-darkink flex flex-col">
      <header className="app-header wf-header sticky top-0 z-30 text-white px-4 pb-3 flex items-center gap-3">
        <button type="button" aria-label="Buka menu" onClick={() => setDrawerOpen(true)} className="p-1"><Menu size={22} /></button>
        <div className="min-w-0">
          <p className="font-bold leading-tight">WebForge Mobile</p>
          <p className="text-xs opacity-95 leading-tight">Faza X TCP · {active?.label}</p>
        </div>
        <button
          type="button"
          aria-label={isDark ? 'Ganti ke tema terang' : 'Ganti ke tema gelap'}
          title={isDark ? 'Ganti ke tema terang' : 'Ganti ke tema gelap'}
          onClick={toggleTheme}
          className="ml-auto p-2 rounded-lg hover:bg-white/15 focus:outline-none focus:ring-2 focus:ring-white/60"
        >
          {isDark ? <Sun size={20} /> : <Moon size={20} />}
        </button>
        <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${health.isSuccess ? 'bg-success' : coreStarting ? 'bg-warning' : 'bg-danger'}`}>
          {health.isSuccess ? 'Core online' : coreStarting ? 'Menyalakan core…' : health.isLoading ? 'Menghubungi…' : 'Core offline'}
        </span>
      </header>

      {drawerOpen && (
        <div className="fixed inset-0 z-40">
          <button type="button" aria-label="Tutup menu" className="absolute inset-0 bg-black/50" onClick={() => setDrawerOpen(false)} />
          <nav className="drawer-top wf-drawer absolute left-0 top-0 bottom-0 w-64 shadow-xl px-3 pb-4 overflow-auto">
            <div className="flex items-center justify-between mb-3">
              <div>
                <p className="font-bold">WebForge Mobile</p>
                <p className="text-xs text-slate-700 dark:text-slate-300">Faza X TCP</p>
              </div>
              <button type="button" aria-label="Tutup" onClick={() => setDrawerOpen(false)} className="p-1"><X size={20} /></button>
            </div>
            <div className="space-y-1">
              {SECTIONS.map((s) => (
                <button key={s.id} type="button" onClick={() => go(s.id)}
                  className={`w-full flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm text-left ${section === s.id ? 'bg-primary text-white font-medium' : 'hover:bg-slate-100 dark:hover:bg-darkbg'}`}>
                  <s.icon size={18} /> {s.label}
                </button>
              ))}
            </div>
          </nav>
        </div>
      )}

      <main className="flex-1 w-full max-w-3xl mx-auto px-4 py-4 pb-24 space-y-4">
        {showOfflineBanner && (
          <OfflineBanner
            onRetry={() => { if (boot.phase === 'web') void health.refetch(); else boot.retry(); }}
            retrying={boot.phase === 'web' ? health.isFetching : coreStarting}
            loadError={boot.phase === 'web' ? null : boot.loadError}
          />
        )}
        {coreStarting ? (
          <div className="space-y-4">
            <p className="text-sm text-slate-600 dark:text-slate-400" role="status">Menyalakan core…</p>
            <SkeletonCard lines={4} />
            <SkeletonCard lines={3} />
          </div>
        ) : (<>
        {section === 'dashboard' && <Dashboard />}
        {section === 'projects' && <Projects />}
        {section === 'servers' && <Servers />}
        {section === 'terminal' && <Terminal />}
        {section === 'databases' && <Databases />}
        {section === 'files' && <Files />}
        {section === 'editor' && <Editor />}
        {section === 'monitoring' && <Monitoring />}
        {section === 'logs' && <Logs />}
        {section === 'tools' && <Tools />}
        {section === 'security' && <Security />}
        {section === 'settings' && <Settings />}
        </>)}
      </main>

      <nav className="bottom-nav wf-bottom-nav fixed bottom-0 left-0 right-0 z-30">
        <div className="max-w-3xl mx-auto grid grid-cols-5">
          {BOTTOM.map((id) => {
            const s = SECTIONS.find((x) => x.id === id) as { id: Section; label: string; icon: typeof LayoutDashboard };
            return (
              <button key={id} type="button" onClick={() => go(id)}
                className={`flex flex-col items-center gap-0.5 py-2 text-[11px] ${section === id ? 'text-primary font-semibold' : 'text-slate-600 dark:text-slate-400'}`}>
                <s.icon size={20} /> {s.label}
              </button>
            );
          })}
          <button type="button" onClick={() => setDrawerOpen(true)} className="flex flex-col items-center gap-0.5 py-2 text-[11px] text-slate-600 dark:text-slate-400">
            <Menu size={20} /> Lainnya
          </button>
        </div>
      </nav>
    </div>
  );
}
