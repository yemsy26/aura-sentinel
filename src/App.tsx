import { Suspense, lazy, useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { AgentPanel, type QuickKind } from './components/AgentPanel';
import { ActivityBar, Splitter, StatusBar, TopBar, type SideView } from './components/Bars';
import { BottomPanel, type BottomTab } from './components/BottomPanel';
import { CommandPalette, SettingsDialog, Shortcuts, type PaletteCommand } from './components/Dialogs';
import type { EditorHandle } from './components/EditorArea';
import { Icon } from './components/Icon';
import { SidePanel } from './components/SidePanel';
import { agentReducer, initialAgent } from './lib/agent';
import {
  api, errorText, onAgentEvent, onAsk, onAskClosed, onFileUpdated,
  type AgentProcess, type Checkpoint, type FileNode, type GitInfo, type PendingMission, type QuickOrder, type SystemStats,
} from './lib/api';
import { confirmar } from './lib/confirmar';
import { relativePath, samePath, serverUrl } from './lib/text';

// Monaco pesa ~4 MB: se carga aparte para que la ventana aparezca antes.
const EditorArea = lazy(() => import('./components/EditorArea').then((m) => ({ default: m.EditorArea })));

function stored<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch { return fallback; }
}
function store(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* sin almacenamiento: solo se pierde la preferencia */ }
}
const now = () => new Date().toISOString();

export default function App() {
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [recents, setRecents] = useState<string[]>(() => stored('aura_recent_workspaces', []));
  const [tree, setTree] = useState<FileNode[]>([]);
  const [tabs, setTabs] = useState<string[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [dirty, setDirty] = useState<string[]>([]);
  const [agent, dispatch] = useReducer(agentReducer, initialAgent);
  const [marks, setMarks] = useState<Record<string, 'M' | 'A'>>({});
  const [view, setView] = useState<SideView | null>('explorer');
  const [sideWidth, setSideWidth] = useState(() => stored('aura_side_width', 260));
  const [agentWidth, setAgentWidth] = useState(() => stored('aura_agent_width', 420));
  const [bottomOpen, setBottomOpen] = useState(() => stored('aura_bottom_open', true));
  const [bottomHeight, setBottomHeight] = useState(() => stored('aura_bottom_height', 220));
  const [bottomTab, setBottomTab] = useState<BottomTab>('problems');
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState<string>(() => stored('aura_model', ''));
  const [bionic, setBionic] = useState<'ok' | 'err' | 'unknown'>('unknown');
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [git, setGit] = useState<GitInfo | null>(null);
  const [checkpoints, setCheckpoints] = useState<Checkpoint[]>([]);
  const [rules, setRules] = useState<{ reglas: string; lecciones: string } | null>(null);
  const [processes, setProcesses] = useState<AgentProcess[]>([]);
  const [pending, setPending] = useState<PendingMission | null>(null);
  const [palette, setPalette] = useState(false);
  const [settings, setSettings] = useState<'engine' | 'profile' | 'keys' | null>(null);
  const [toast, setToast] = useState<{ text: string; error: boolean } | null>(null);
  const [previewUrl, setPreviewUrl] = useState('');
  const [reviewPlan, setReviewPlan] = useState(() => stored('aura_revisar_plan', false));
  const [draft, setDraft] = useState<{ text: string; id: number } | null>(null);
  const editor = useRef<EditorHandle>(null);
  const treeTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const workspaceRef = useRef(workspace);
  workspaceRef.current = workspace;

  const notify = useCallback((text: string, error = false) => setToast({ text, error }), []);
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), toast.error ? 7000 : 3500);
    return () => clearTimeout(t);
  }, [toast]);

  useEffect(() => { store('aura_side_width', sideWidth); }, [sideWidth]);
  useEffect(() => { store('aura_agent_width', agentWidth); }, [agentWidth]);
  useEffect(() => { store('aura_bottom_open', bottomOpen); }, [bottomOpen]);
  useEffect(() => { store('aura_bottom_height', bottomHeight); }, [bottomHeight]);
  useEffect(() => { store('aura_model', model); }, [model]);

  // ── Datos del proyecto ──────────────────────────────────────────────────
  const refreshTree = useCallback(async () => {
    const ws = workspaceRef.current;
    if (!ws) return;
    try { setTree(await api.tree(ws)); } catch (e) { notify(`No pude leer la carpeta: ${errorText(e)}`, true); }
  }, [notify]);
  const refreshTreeSoon = useCallback(() => {
    clearTimeout(treeTimer.current);
    treeTimer.current = setTimeout(() => void refreshTree(), 400);
  }, [refreshTree]);
  const refreshProject = useCallback(async () => {
    const ws = workspaceRef.current;
    if (!ws) return;
    const [g, cps, pend] = await Promise.all([
      api.git(ws).catch(() => null), api.checkpoints(ws).catch(() => []), api.pendingMission(ws).catch(() => null),
    ]);
    setGit(g); setCheckpoints(cps); setPending(pend);
  }, []);
  const refreshProcesses = useCallback(async () => {
    const ws = workspaceRef.current;
    if (!ws) return;
    try { setProcesses(((await api.order({ tipo: 'procesos' }, ws)).datos as AgentProcess[]) ?? []); } catch { /* agente ocupado */ }
  }, []);
  const refreshModels = useCallback(async () => {
    try {
      const list = await api.models();
      setModels(list);
      setBionic('ok');
      setModel((m) => (list.includes(m) ? m : list[0] ?? ''));
    } catch { setBionic('err'); }
  }, []);

  const openWorkspace = useCallback(async (path: string) => {
    setWorkspace(path);
    workspaceRef.current = path;
    store('aura_last_workspace', path);
    setRecents((r) => {
      const next = [path, ...r.filter((x) => !samePath(x, path))].slice(0, 8);
      store('aura_recent_workspaces', next);
      return next;
    });
    setTabs([]); setActive(null); setDirty([]); setMarks({}); setProcesses([]); setPreviewUrl('');
    void refreshTree();
    void refreshProject();
    try { dispatch({ type: 'history', messages: await api.chatHistory(path) }); } catch { dispatch({ type: 'reset' }); }
  }, [refreshTree, refreshProject]);

  const pickFolder = useCallback(async () => {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const dir = await open({ directory: true, defaultPath: workspaceRef.current ?? undefined });
    if (typeof dir === 'string') void openWorkspace(dir);
  }, [openWorkspace]);

  // Arranque
  useEffect(() => {
    const last = stored<string | null>('aura_last_workspace', null);
    void (last ? Promise.resolve(last) : api.currentDirectory()).then(openWorkspace).catch(() => undefined);
    void refreshModels();
    void api.rules().then(setRules).catch(() => undefined);
    // Con la ventana minimizada u oculta no se consulta nada (nvidia-smi cuesta un proceso por consulta).
    const visible = () => document.visibilityState === 'visible';
    const poll = () => { if (visible()) void api.stats().then(setStats).catch(() => undefined); };
    poll();
    const statsTimer = setInterval(poll, 4000);
    const modelsTimer = setInterval(() => { if (visible()) void refreshModels(); }, 30000);
    document.addEventListener('visibilitychange', poll);
    return () => { clearInterval(statsTimer); clearInterval(modelsTimer); document.removeEventListener('visibilitychange', poll); };
  }, [openWorkspace, refreshModels]);

  // Eventos del agente
  useEffect(() => {
    const subs = [
      onAgentEvent((event) => {
        dispatch({ type: 'event', event });
        if (event.tipo === 'archivos') {
          refreshTreeSoon();
          if (!event.rutas.length) editor.current?.reloadAll(); // git, ejecutar o restaurar: puede haber cambiado cualquiera
        }
      }),
      onFileUpdated((path) => { dispatch({ type: 'changed', path }); editor.current?.reload(path); }),
      onAsk((ask) => dispatch({ type: 'ask', ask })),
      onAskClosed((id) => dispatch({ type: 'askClosed', id })),
    ];
    return () => { for (const s of subs) void s.then((off) => off()); };
  }, [refreshTreeSoon]);

  // Marcas M/A del explorador para lo que tocó el agente en esta misión
  useEffect(() => {
    if (!agent.changed.length) { setMarks({}); return; }
    let live = true;
    void Promise.all(agent.changed.map(async (path) => [path.toLowerCase(), (await api.baseline(path).catch(() => '')) === null ? 'A' : 'M'] as const))
      .then((list) => live && setMarks(Object.fromEntries(list)));
    return () => { live = false; };
  }, [agent.changed]);

  // ── Editor ──────────────────────────────────────────────────────────────
  const openFile = useCallback((path: string, line?: number) => {
    setTabs((t) => (t.some((x) => samePath(x, path)) ? t : [...t, path]));
    setActive((a) => (a && samePath(a, path) ? a : path));
    if (line) editor.current?.reveal(path, line);
  }, []);
  const closeTab = useCallback((path: string) => {
    setTabs((t) => {
      const i = t.findIndex((x) => samePath(x, path));
      const next = t.filter((x) => !samePath(x, path));
      setActive((a) => (a && samePath(a, path) ? next[Math.min(i, next.length - 1)] ?? null : a));
      return next;
    });
    setDirty((d) => d.filter((x) => !samePath(x, path)));
  }, []);
  const markDirty = useCallback((path: string, isDirty: boolean) => {
    setDirty((d) => {
      const has = d.some((x) => samePath(x, path));
      if (isDirty === has) return d;
      return isDirty ? [...d, path] : d.filter((x) => !samePath(x, path));
    });
  }, []);

  // ── Agente ──────────────────────────────────────────────────────────────
  const files = useMemo(() => (workspace ? tree.filter((n) => !n.is_dir).map((n) => relativePath(workspace, n.path)) : []),
    [tree, workspace]);
  const lastObjective = [...agent.items].reverse().find((i) => i.kind === 'user')?.text ?? '';

  const afterRun = useCallback(() => {
    void refreshTree();
    void refreshProject();
    void refreshProcesses();
    void api.rules().then(setRules).catch(() => undefined);
  }, [refreshTree, refreshProject, refreshProcesses]);

  const runMission = useCallback(async (text: string) => {
    const ws = workspaceRef.current;
    if (!ws || agent.busy) return;
    const mentioned = [...text.matchAll(/(?:^|\s)@(\S+)/g)].map((m) => m[1]).filter((f) => files.includes(f));
    const message = mentioned.length ? `${text}\n\n(Archivos que menciono: ${mentioned.join(', ')})` : text;
    dispatch({ type: 'send', text, now: Date.now() });
    void api.saveChat(ws, { sender: 'user', text, timestamp: now() }).catch(() => undefined);
    try {
      const result = await api.mission(message, ws, model || null, reviewPlan);
      dispatch({ type: 'done', text: result.respuesta_conversacional, status: result.status });
      void api.saveChat(ws, { sender: 'agent', text: result.respuesta_conversacional, timestamp: now() }).catch(() => undefined);
    } catch (e) {
      dispatch({ type: 'done', text: `Error: ${errorText(e)}`, status: 'ERROR' });
    }
    afterRun();
  }, [agent.busy, files, model, reviewPlan, afterRun]);

  const runOrder = useCallback(async (order: QuickOrder, label: string) => {
    const ws = workspaceRef.current;
    if (!ws || agent.busy) return;
    dispatch({ type: 'send', text: label, now: Date.now() });
    try {
      const fin = await api.order(order, ws);
      dispatch({ type: 'done', text: fin.respuesta, status: fin.estado === 'error' ? 'ERROR' : 'FINISH' });
      void api.saveChat(ws, { sender: 'user', text: label, timestamp: now() }).catch(() => undefined);
      void api.saveChat(ws, { sender: 'agent', text: fin.respuesta, timestamp: now() }).catch(() => undefined);
    } catch (e) {
      dispatch({ type: 'done', text: errorText(e), status: 'ERROR' });
    }
    afterRun();
  }, [agent.busy, afterRun]);

  const quick = useCallback((kind: QuickKind) => {
    const ws = workspaceRef.current;
    if (!ws) return;
    if (kind === 'continuar') void runMission('continúa');
    if (kind === 'verificar') { setBottomOpen(true); setBottomTab('problems'); void runOrder({ tipo: 'verificar' }, 'Verificar el proyecto'); }
    if (kind === 'revisar') void runOrder({ tipo: 'revisar', texto: lastObjective }, 'Revisar el código');
    if (kind === 'nuevo') void (async () => {
      if (!(await confirmar('¿Empezar una conversación nueva? El agente olvida el historial; los archivos y el plan se conservan.', false))) return;
      void api.order({ tipo: 'nuevo' }, ws).catch(() => undefined);
      void api.clearChat(ws).catch(() => undefined);
      dispatch({ type: 'reset' });
    })();
  }, [runMission, runOrder, lastObjective]);

  const openUrl = useCallback((url: string) => {
    if (/^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?(\/|$)/.test(url)) {
      setPreviewUrl(url); setBottomOpen(true); setBottomTab('preview');
    } else {
      void navigator.clipboard?.writeText(url).catch(() => undefined);
      notify(`Enlace copiado: ${url}`);
    }
  }, [notify]);
  const stop = useCallback(() => { void api.cancel(); notify('Deteniendo el agente y sus procesos…'); }, [notify]);
  const answer = useCallback((option: string) => {
    if (!agent.ask) return;
    void api.answer(agent.ask.id, option).catch((e) => notify(errorText(e), true));
    dispatch({ type: 'ask', ask: null });
  }, [agent.ask, notify]);

  const restore = useCallback(async (numero: number) => {
    if (!(await confirmar(`¿Volver al punto de restauración #${numero}? Los archivos que guardó se reescriben.`))) return;
    void runOrder({ tipo: 'restaurar', numero }, `Volver al punto de restauración #${numero}`).then(() => editor.current?.reloadAll());
  }, [runOrder]);

  const stopProcess = useCallback(async (id: number) => {
    const ws = workspaceRef.current;
    if (!ws) return;
    try { notify((await api.order({ tipo: 'detener', id }, ws)).respuesta); } catch (e) { notify(errorText(e), true); }
    void refreshProcesses();
  }, [notify, refreshProcesses]);

  const problems = useMemo(() => Object.values(agent.problems).flat(), [agent.problems]);
  const server = useMemo(() => processes.filter((x) => x.vivo).map((x) => serverUrl(x.comando)).find(Boolean) ?? null, [processes]);
  useEffect(() => { if (server && !previewUrl) setPreviewUrl(server); }, [server, previewUrl]);

  // ── Atajos ──────────────────────────────────────────────────────────────
  const toggleView = useCallback((v: SideView) => setView((cur) => (cur === v ? null : v)), []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.altKey) return;
      const k = e.key.toLowerCase();
      const inEditor = !!(e.target as HTMLElement | null)?.closest?.('.monaco-editor');
      const act = (fn: () => void) => { e.preventDefault(); e.stopPropagation(); fn(); };
      if (k === 'p' && !e.shiftKey) act(() => setPalette(true));
      else if (k === 'k' && !inEditor) act(() => setPalette(true));
      else if (k === 'j') act(() => setBottomOpen((o) => !o));
      else if (k === 'b' && !e.shiftKey) act(() => setView((v) => (v ? null : 'explorer')));
      else if (k === 'f' && e.shiftKey) act(() => setView('search'));
      else if (k === 'e' && e.shiftKey) act(() => setView('explorer'));
      else if (k === 'l') act(() => document.getElementById('composer')?.focus());
      else if (k === 'w' && active) act(() => void (async () => {
        const sinGuardar = dirty.some((d) => samePath(d, active));
        if (sinGuardar && !(await confirmar(`${active.split(/[\\/]/).pop()} tiene cambios sin guardar. ¿Cerrar igualmente?`))) return;
        closeTab(active);
      })());
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [active, closeTab, dirty]);

  const commands: PaletteCommand[] = [
    { id: 'open', label: 'Abrir carpeta…', icon: 'folder', run: () => void pickFolder() },
    { id: 'verify', label: 'Agente: verificar el proyecto', icon: 'check', run: () => quick('verificar') },
    { id: 'review', label: 'Agente: revisar el código', icon: 'search', run: () => quick('revisar') },
    { id: 'continue', label: 'Agente: continuar la misión', icon: 'play', run: () => quick('continuar') },
    { id: 'new', label: 'Agente: nueva conversación', icon: 'plus', run: () => quick('nuevo') },
    { id: 'stop', label: 'Agente: detener', icon: 'stop', run: stop },
    { id: 'rules', label: 'Reglas y lecciones del agente', icon: 'book', run: () => setView('rules') },
    { id: 'restore', label: 'Puntos de restauración', icon: 'history', run: () => setView('restore') },
    { id: 'git', label: 'Git: cambios y commits', icon: 'git', run: () => setView('git') },
    { id: 'search', label: 'Buscar en el proyecto', icon: 'search', hint: 'Ctrl+Shift+F', run: () => setView('search') },
    { id: 'terminal', label: 'Terminal', icon: 'terminal', run: () => { setBottomOpen(true); setBottomTab('terminal'); } },
    { id: 'problems', label: 'Problemas', icon: 'warning', run: () => { setBottomOpen(true); setBottomTab('problems'); } },
    { id: 'processes', label: 'Procesos del agente', icon: 'cpu', run: () => { setBottomOpen(true); setBottomTab('processes'); void refreshProcesses(); } },
    { id: 'preview', label: 'Vista previa', icon: 'eye', run: () => { setBottomOpen(true); setBottomTab('preview'); } },
    { id: 'panel', label: 'Mostrar u ocultar el panel inferior', icon: 'terminal', hint: 'Ctrl+J', run: () => setBottomOpen((o) => !o) },
    { id: 'reload', label: 'Recargar el árbol de archivos', icon: 'refresh', run: () => void refreshTree() },
    { id: 'settings', label: 'Configuración del motor', icon: 'settings', run: () => setSettings('engine') },
    { id: 'profile', label: 'Perfil y preferencias', icon: 'user', run: () => setSettings('profile') },
    { id: 'keys', label: 'Atajos de teclado', icon: 'info', run: () => setSettings('keys') },
  ];

  const welcome = (
    <div className="welcome">
      <Icon name="logo" size={44} />
      <h1>Aura Sentinel</h1>
      <span>{workspace ? 'Abre un archivo del explorador o pide algo al agente.' : 'Abre una carpeta de proyecto para empezar.'}</span>
      <div className="field-row">
        <button className="btn" onClick={() => void pickFolder()}><Icon name="folder" />Abrir carpeta</button>
        <button className="btn" onClick={() => setPalette(true)}><Icon name="search" />Paleta (Ctrl+K)</button>
      </div>
      <Shortcuts />
    </div>
  );

  return (
    <div className="app">
      <TopBar workspace={workspace} recents={recents} onOpenFolder={() => void pickFolder()} onPickRecent={(p) => void openWorkspace(p)}
        onPalette={() => setPalette(true)} models={models} model={model} onModel={setModel} bionic={bionic}
        context={agent.context} stats={stats} onSettings={() => setSettings('engine')} />
      <div className="workbench">
        <ActivityBar view={view} onView={toggleView} changes={git?.changes.length ?? 0} onProfile={() => setSettings('profile')} />
        {view && <>
          <SidePanel view={view} width={sideWidth} workspace={workspace} tree={tree} active={active} marks={marks} git={git}
            checkpoints={checkpoints} rules={rules} busy={agent.busy} onOpen={openFile}
            onRefresh={() => { void refreshTree(); void refreshProject(); }} onOpenFolder={() => void pickFolder()}
            onSearch={(q) => api.search(workspace ?? '', q)} onRestore={restore}
            onAddRule={(t) => void runOrder({ tipo: 'regla', texto: t }, `Nueva regla: ${t}`)} />
          <Splitter axis="x" size={sideWidth} onSize={setSideWidth} min={180} max={520} />
        </>}
        <main className="main">
          <Suspense fallback={<div className="editor-zone">{welcome}</div>}>
            <EditorArea ref={editor} workspace={workspace} tabs={tabs} active={active} agentChanged={agent.changed} dirty={dirty}
              onActivate={setActive} onClose={closeTab} onDirty={markDirty} onToast={notify} welcome={welcome}
              onAskAgent={(text) => setDraft({ text, id: Date.now() })} />
          </Suspense>
          {bottomOpen && <>
            <Splitter axis="y" size={bottomHeight} onSize={setBottomHeight} min={110} max={640} invert />
            <BottomPanel tab={bottomTab} onTab={(t) => { setBottomTab(t); if (t === 'processes') void refreshProcesses(); }}
              height={bottomHeight} workspace={workspace} problems={problems} processes={processes} busy={agent.busy}
              previewUrl={previewUrl} onPreviewUrl={setPreviewUrl} onRun={(c) => api.run(workspace ?? '', c)}
              onStopProcess={(id) => void stopProcess(id)} onRefreshProcesses={() => void refreshProcesses()}
              onOpenExternal={(u) => void api.openExternal(u).catch((e) => notify(errorText(e), true))}
              onAskAgent={(t) => void runMission(t)} onHide={() => setBottomOpen(false)} />
          </>}
        </main>
        <Splitter axis="x" size={agentWidth} onSize={setAgentWidth} min={320} max={760} invert />
        <AgentPanel state={agent} width={agentWidth} workspace={workspace} files={files} pending={pending}
          onSend={(t) => void runMission(t)} onStop={stop} onQuick={quick} onAnswer={answer}
          onDismissPending={() => { if (workspace) void api.dismissPending(workspace).then(() => setPending(null)); }}
          onOpenFile={openFile} onOpenUrl={openUrl} draft={draft}
          reviewPlan={reviewPlan} onReviewPlan={(v) => { setReviewPlan(v); store('aura_revisar_plan', v); }} />
      </div>
      <StatusBar git={git} errors={problems.filter((x) => x.severity === 'error').length}
        warnings={problems.filter((x) => x.severity === 'warning').length} server={server} bionic={bionic} stats={stats}
        onProblems={() => { setBottomOpen(true); setBottomTab('problems'); }}
        onPreview={() => { setBottomOpen(true); setBottomTab('preview'); }} onGit={() => setView('git')}
        onToggleBottom={() => setBottomOpen((o) => !o)} />
      {palette && <CommandPalette commands={commands} onClose={() => setPalette(false)} onOpenFile={(p) => openFile(p)}
        files={tree.filter((n) => !n.is_dir).map((n) => ({ path: n.path, label: workspace ? relativePath(workspace, n.path) : n.name }))} />}
      {settings && <SettingsDialog initialTab={settings} workspace={workspace} onClose={() => setSettings(null)} onToast={notify}
        onEngineChanged={() => void refreshModels()} />}
      {toast && <div className={`toast${toast.error ? ' err' : ''}`} role="status">{toast.text}</div>}
    </div>
  );
}
