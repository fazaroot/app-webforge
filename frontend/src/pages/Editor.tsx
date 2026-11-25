import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, File, Folder, Save, Search, X } from 'lucide-react';
import { EditorState, type Extension } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, highlightActiveLine } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching, indentOnInput } from '@codemirror/language';
import { oneDark } from '@codemirror/theme-one-dark';
import { javascript } from '@codemirror/lang-javascript';
import { html } from '@codemirror/lang-html';
import { css } from '@codemirror/lang-css';
import { json } from '@codemirror/lang-json';
import { python } from '@codemirror/lang-python';
import { php } from '@codemirror/lang-php';
import { sql } from '@codemirror/lang-sql';
import { markdown } from '@codemirror/lang-markdown';
import { yaml } from '@codemirror/lang-yaml';
import { api } from '../lib/api';
import { Card, EmptyBox, ErrorBox, LoadingBox, btnGhost, btnPrimary, inputCls } from '../components/ui';
import { ProjectPicker } from '../components/ProjectPicker';
import { useAppStore } from '../store';

function languageFor(path: string): Extension | null {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  switch (ext) {
    case 'js': case 'jsx': case 'mjs': case 'cjs': return javascript();
    case 'ts': case 'tsx': return javascript({ typescript: true, jsx: ext === 'tsx' });
    case 'html': case 'htm': return html();
    case 'css': case 'scss': return css();
    case 'json': return json();
    case 'py': return python();
    case 'php': return php();
    case 'sql': return sql();
    case 'md': return markdown();
    case 'yml': case 'yaml': return yaml();
    default: return null;
  }
}

interface Tab { path: string; content: string; dirty: boolean }

function TreeNode({ projectId, path, name, depth, onOpen }: { projectId: string; path: string; name: string; depth: number; onOpen: (p: string) => void }) {
  const [expanded, setExpanded] = useState(false);
  const children = useQuery({
    queryKey: ['files', projectId, path],
    queryFn: () => api.filesList(projectId, path),
    enabled: expanded,
  });
  return (
    <div>
      <button type="button" className="flex items-center gap-1 text-sm py-0.5 w-full text-left" style={{ paddingLeft: depth * 12 }} onClick={() => setExpanded(!expanded)}>
        {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        <Folder size={14} className="text-primary" /> {name}
      </button>
      {expanded && (
        <div>
          {children.isLoading && <p className="text-xs text-slate-600 dark:text-slate-400" style={{ paddingLeft: (depth + 1) * 12 }}>Memuat…</p>}
          {(children.data ?? []).map((e) => e.is_dir ? (
            <TreeNode key={e.path} projectId={projectId} path={e.path} name={e.name} depth={depth + 1} onOpen={onOpen} />
          ) : (
            <button key={e.path} type="button" className="flex items-center gap-1 text-sm py-0.5 w-full text-left" style={{ paddingLeft: (depth + 1) * 12 }} onClick={() => onOpen(e.path)}>
              <File size={14} /> {e.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function Editor() {
  const projectId = useAppStore((s) => s.activeProjectId);
  const theme = useAppStore((s) => s.theme);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [findText, setFindText] = useState('');
  const [findInfo, setFindInfo] = useState('');
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);

  const root = useQuery({
    queryKey: ['files', projectId, ''],
    queryFn: () => api.filesList(projectId as string, ''),
    enabled: projectId !== null,
  });

  const activeTab = useMemo(() => tabs.find((t) => t.path === activePath) ?? null, [tabs, activePath]);

  const openFile = useCallback(async (path: string) => {
    if (!projectId) return;
    const existing = tabs.find((t) => t.path === path);
    if (existing) { setActivePath(path); return; }
    try {
      const r = await api.filesRead(projectId, path);
      setTabs((prev) => [...prev, { path: r.path, content: r.content, dirty: false }]);
      setActivePath(r.path);
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Gagal membuka file');
    }
  }, [projectId, tabs]);

  const save = useMutation({
    mutationFn: () => {
      const view = viewRef.current;
      const tab = activeTab as Tab;
      const content = view ? view.state.doc.toString() : tab.content;
      return api.filesWrite(projectId as string, tab.path, content).then((r) => ({ r, content }));
    },
    onSuccess: ({ r, content }) => {
      setTabs((prev) => prev.map((t) => (t.path === activePath ? { ...t, content, dirty: false } : t)));
      setNotice(`Tersimpan (${r.size} byte).`);
      setError('');
    },
    onError: (e) => setError(e instanceof Error ? e.message : 'Gagal menyimpan'),
  });

  useEffect(() => {
    if (!hostRef.current || !activeTab) return;
    const lang = languageFor(activeTab.path);
    const dark = document.documentElement.classList.contains('dark');
    const extensions: Extension[] = [
      lineNumbers(), highlightActiveLine(), history(), bracketMatching(), indentOnInput(),
      syntaxHighlighting(defaultHighlightStyle),
      keymap.of([...defaultKeymap, ...historyKeymap]),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) {
          const doc = u.state.doc.toString();
          setTabs((prev) => prev.map((t) => (t.path === activeTab.path ? { ...t, content: doc, dirty: true } : t)));
        }
      }),
    ];
    if (lang) extensions.push(lang);
    if (dark) extensions.push(oneDark);
    const view = new EditorView({
      state: EditorState.create({ doc: activeTab.content, extensions }),
      parent: hostRef.current,
    });
    viewRef.current = view;
    return () => { view.destroy(); viewRef.current = null; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activePath, theme]);

  function findNext() {
    const view = viewRef.current;
    if (!view || !findText) return;
    const doc = view.state.doc.toString();
    const from = view.state.selection.main.to;
    let idx = doc.toLowerCase().indexOf(findText.toLowerCase(), from);
    if (idx < 0) idx = doc.toLowerCase().indexOf(findText.toLowerCase());
    if (idx < 0) { setFindInfo('Tidak ditemukan.'); return; }
    const total = doc.toLowerCase().split(findText.toLowerCase()).length - 1;
    view.dispatch({ selection: { anchor: idx, head: idx + findText.length }, scrollIntoView: true });
    view.focus();
    setFindInfo(`${total} kecocokan dalam file.`);
  }

  function closeTab(path: string) {
    const tab = tabs.find((t) => t.path === path);
    if (tab?.dirty && !window.confirm('File belum disimpan. Tutup tab dan buang perubahan?')) return;
    setTabs((prev) => prev.filter((t) => t.path !== path));
    if (activePath === path) setActivePath(null);
  }

  if (!projectId) {
    return (
      <div className="space-y-4">
        <ProjectPicker />
        <EmptyBox message="Pilih proyek dulu untuk membuka code editor." />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <ProjectPicker />
      {error && <ErrorBox message={error} />}
      {notice && <div className="rounded-lg bg-success/10 border border-success/40 text-sm px-3 py-2">{notice}</div>}

      <Card title="File proyek">
        {root.isLoading ? <LoadingBox /> : root.isError ? <ErrorBox message="Gagal memuat file proyek." /> : (root.data?.length ?? 0) === 0 ? (
          <EmptyBox message="Proyek ini belum memiliki file." />
        ) : (
          <div className="max-h-56 overflow-auto">
            {root.data!.map((e) => e.is_dir ? (
              <TreeNode key={e.path} projectId={projectId} path={e.path} name={e.name} depth={0} onOpen={(p) => void openFile(p)} />
            ) : (
              <button key={e.path} type="button" className="flex items-center gap-1 text-sm py-0.5 w-full text-left" onClick={() => void openFile(e.path)}>
                <File size={14} /> {e.name}
              </button>
            ))}
          </div>
        )}
      </Card>

      <Card title="Editor">
        {tabs.length === 0 ? (
          <EmptyBox message="Pilih file dari daftar di atas untuk mulai mengedit." />
        ) : (
          <>
            <div className="flex flex-wrap gap-1 mb-2">
              {tabs.map((t) => (
                <span key={t.path} className={`inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs ${t.path === activePath ? 'bg-primary text-white' : 'bg-slate-100 dark:bg-darkbg'}`}>
                  <button type="button" onClick={() => setActivePath(t.path)}>{t.path}{t.dirty ? ' •' : ''}</button>
                  <button type="button" aria-label={`Tutup ${t.path}`} onClick={() => closeTab(t.path)}><X size={12} /></button>
                </span>
              ))}
            </div>
            <div className="flex flex-wrap gap-2 mb-2">
              <input className={`${inputCls} flex-1 min-w-[140px]`} placeholder="Cari dalam file…" value={findText} onChange={(e) => setFindText(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') findNext(); }} />
              <button type="button" className={btnGhost} disabled={!findText} onClick={findNext}><Search size={14} /> Cari</button>
              <button type="button" className={btnPrimary} disabled={!activeTab?.dirty || save.isPending} onClick={() => save.mutate()}>
                <Save size={14} /> {save.isPending ? 'Menyimpan…' : 'Simpan'}
              </button>
            </div>
            {findInfo && <p className="text-xs text-slate-600 dark:text-slate-400 mb-1">{findInfo}</p>}
            <div ref={hostRef} className="rounded-lg border border-slate-200 dark:border-slate-700 h-[420px] overflow-hidden bg-white dark:bg-darkbg" />
            <p className="text-xs text-slate-600 dark:text-slate-400 mt-1">Titik (•) berarti ada perubahan belum disimpan. Simpan menulis file nyata lewat API.</p>
          </>
        )}
      </Card>
    </div>
  );
}
