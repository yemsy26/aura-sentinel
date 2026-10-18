import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { api, errorText } from '../lib/api';
import { confirmar } from '../lib/confirmar';
import { monaco } from '../lib/monaco';
import { baseName, diffCounts, languageOf, relativePath, samePath, selectionRef } from '../lib/text';
import { Icon } from './Icon';

export interface EditorHandle {
  reload: (path: string) => void;
  reloadAll: () => void;
  reveal: (path: string, line: number) => void;
}

interface Entry { model: monaco.editor.ITextModel; saved: number; view: monaco.editor.ICodeEditorViewState | null; stale: boolean }

interface Props {
  workspace: string | null;
  tabs: string[];
  active: string | null;
  agentChanged: string[];
  dirty: string[];
  onActivate: (path: string) => void;
  onClose: (path: string) => void;
  onDirty: (path: string, dirty: boolean) => void;
  onToast: (text: string, error?: boolean) => void;
  onAskAgent?: (ref: string) => void;
  welcome: React.ReactNode;
}

export const EditorArea = forwardRef<EditorHandle, Props>(function EditorArea(p, ref) {
  const host = useRef<HTMLDivElement>(null);
  const diffHost = useRef<HTMLDivElement>(null);
  const editor = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const diff = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const entries = useRef(new Map<string, Entry>());
  const current = useRef<string | null>(null);
  const pendingLine = useRef<{ path: string; line: number } | null>(null);
  const [baseline, setBaseline] = useState<string | null | undefined>(undefined); // undefined = sin cambios del agente
  const [counts, setCounts] = useState({ added: 0, removed: 0 });
  const [showDiff, setShowDiff] = useState(false);
  const [stale, setStale] = useState(false);
  const props = useRef(p);
  props.current = p;

  const key = (path: string) => path.toLowerCase();

  const save = useCallback(async (path: string) => {
    const entry = entries.current.get(key(path));
    if (!entry) return;
    try {
      await api.saveFile(path, entry.model.getValue());
      entry.saved = entry.model.getAlternativeVersionId();
      entry.stale = false;
      setStale(false);
      props.current.onDirty(path, false);
    } catch (e) {
      props.current.onToast(`No se pudo guardar: ${errorText(e)}`, true);
    }
  }, []);

  useEffect(() => {
    if (!host.current) return;
    const ed = monaco.editor.create(host.current, {
      theme: 'aura', automaticLayout: true, fontFamily: "'Cascadia Code', 'Cascadia Mono', Consolas, monospace",
      fontSize: 13, lineHeight: 21, minimap: { enabled: true, scale: 1 }, scrollBeyondLastLine: false,
      smoothScrolling: true, renderWhitespace: 'selection', tabSize: 2, bracketPairColorization: { enabled: true },
      padding: { top: 8 }, model: null,
    });
    ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => { if (current.current) void save(current.current); });
    ed.addAction({ // la selección (o la línea del cursor) pasa al chat como «@archivo (líneas a-b)»
      id: 'aura.pedirAlAgente', label: 'Pedir al agente sobre esto', contextMenuGroupId: 'navigation', contextMenuOrder: 0,
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.KeyA],
      run: (e) => {
        const path = current.current, sel = e.getSelection(), ws = props.current.workspace;
        if (!path || !sel) return;
        props.current.onAskAgent?.(selectionRef(ws ? relativePath(ws, path) : path, sel.startLineNumber, sel.endLineNumber, sel.endColumn));
      },
    });
    editor.current = ed;
    return () => {
      ed.dispose();
      diff.current?.dispose();
      for (const e of entries.current.values()) e.model.dispose();
      entries.current.clear();
    };
  }, [save]);

  const open = useCallback(async (path: string): Promise<Entry | null> => {
    const existing = entries.current.get(key(path));
    if (existing) return existing;
    try {
      const text = await api.readFile(path);
      const model = monaco.editor.createModel(text, languageOf(path), monaco.Uri.file(path));
      const entry: Entry = { model, saved: model.getAlternativeVersionId(), view: null, stale: false };
      model.onDidChangeContent(() => props.current.onDirty(path, model.getAlternativeVersionId() !== entry.saved));
      entries.current.set(key(path), entry);
      return entry;
    } catch (e) {
      props.current.onToast(`No se pudo abrir ${baseName(path)}: ${errorText(e)}`, true);
      props.current.onClose(path);
      return null;
    }
  }, []);

  // Pestaña activa
  useEffect(() => {
    const ed = editor.current;
    if (!ed) return;
    if (current.current) {
      const prev = entries.current.get(key(current.current));
      if (prev) prev.view = ed.saveViewState();
    }
    current.current = p.active;
    setShowDiff(false);
    if (!p.active) { ed.setModel(null); return; }
    const path = p.active;
    void open(path).then((entry) => {
      if (!entry || current.current !== path) return;
      ed.setModel(entry.model);
      if (entry.view) ed.restoreViewState(entry.view);
      setStale(entry.stale);
      const line = pendingLine.current;
      if (line && samePath(line.path, path)) {
        ed.revealLineInCenter(line.line);
        ed.setPosition({ lineNumber: line.line, column: 1 });
        pendingLine.current = null;
      }
      ed.focus();
    });
  }, [p.active, open]);

  // Pestañas cerradas: liberar sus modelos
  useEffect(() => {
    const keep = new Set(p.tabs.map(key));
    for (const [k, e] of entries.current) if (!keep.has(k)) { e.model.dispose(); entries.current.delete(k); }
  }, [p.tabs]);

  // Base de la misión para el aviso "El agente cambió este archivo"
  const changed = !!p.active && p.agentChanged.some((c) => samePath(c, p.active!));
  useEffect(() => {
    setBaseline(undefined);
    if (!p.active || !changed) return;
    let live = true;
    void api.baseline(p.active).then((b) => live && setBaseline(b));
    return () => { live = false; };
  }, [p.active, changed]);

  useEffect(() => {
    const entry = p.active ? entries.current.get(key(p.active)) : undefined;
    if (!entry || typeof baseline !== 'string') return;
    const update = () => setCounts(diffCounts(baseline, entry.model.getValue()));
    update();
    const sub = entry.model.onDidChangeContent(update);
    return () => sub.dispose();
  }, [baseline, p.active]);

  // Vista de cambios (editor de diferencias)
  useEffect(() => {
    if (!showDiff || !diffHost.current || !p.active || typeof baseline !== 'string') return;
    const entry = entries.current.get(key(p.active));
    if (!entry) return;
    const original = monaco.editor.createModel(baseline, languageOf(p.active));
    const de = monaco.editor.createDiffEditor(diffHost.current, {
      theme: 'aura', automaticLayout: true, readOnly: false, originalEditable: false, renderSideBySide: true,
      fontFamily: "'Cascadia Code', Consolas, monospace", fontSize: 13, scrollBeyondLastLine: false,
    });
    de.setModel({ original, modified: entry.model });
    diff.current = de;
    return () => { de.dispose(); original.dispose(); diff.current = null; };
  }, [showDiff, p.active, baseline]);

  const replace = (entry: Entry, text: string) => {
    entry.model.pushEditOperations([], [{ range: entry.model.getFullModelRange(), text }], () => null);
  };

  const reload = useCallback(async (path: string) => {
    const entry = entries.current.get(key(path));
    if (!entry) return;
    const dirty = entry.model.getAlternativeVersionId() !== entry.saved;
    if (dirty) { // no pisar lo que el usuario está escribiendo
      entry.stale = true;
      if (current.current && samePath(current.current, path)) setStale(true);
      return;
    }
    try {
      const text = await api.readFile(path);
      if (text !== entry.model.getValue()) {
        replace(entry, text);
        entry.saved = entry.model.getAlternativeVersionId();
        props.current.onDirty(path, false);
      }
    } catch { /* borrado: la pestaña se queda con lo último */ }
  }, []);

  useImperativeHandle(ref, () => ({
    reload: (path) => void reload(path),
    reloadAll: () => { for (const path of props.current.tabs) void reload(path); },
    reveal: (path, line) => {
      pendingLine.current = { path, line };
      const ed = editor.current;
      if (ed && current.current && samePath(current.current, path)) {
        ed.revealLineInCenter(line);
        ed.setPosition({ lineNumber: line, column: 1 });
        pendingLine.current = null;
      }
    },
  }), [reload]);

  const undoAgent = async () => {
    if (!p.active || typeof baseline !== 'string') return;
    if (!(await confirmar(`¿Devolver ${baseName(p.active)} a como estaba antes de la misión?`))) return;
    const entry = entries.current.get(key(p.active));
    if (!entry) return;
    replace(entry, baseline);
    await save(p.active);
    setShowDiff(false);
  };

  const reloadFromDisk = async () => {
    if (!p.active) return;
    const entry = entries.current.get(key(p.active));
    if (!entry) return;
    replace(entry, await api.readFile(p.active));
    entry.saved = entry.model.getAlternativeVersionId();
    entry.stale = false;
    setStale(false);
    p.onDirty(p.active, false);
  };

  const closeTab = async (path: string) => {
    if (p.dirty.some((d) => samePath(d, path)) && !(await confirmar(`${baseName(path)} tiene cambios sin guardar. ¿Cerrar igualmente?`))) return;
    p.onClose(path);
  };

  return (
    <div className="editor-zone">
      {p.tabs.length > 0 && (
        <div className="tabs" role="tablist" aria-label="Archivos abiertos">
          {p.tabs.map((path) => {
            const selected = !!p.active && samePath(path, p.active);
            const isDirty = p.dirty.some((d) => samePath(d, path));
            const byAgent = p.agentChanged.some((c) => samePath(c, path));
            return (
              <div key={path} role="tab" aria-selected={selected} className="tab" tabIndex={0} title={path}
                onClick={() => p.onActivate(path)} onAuxClick={(e) => { if (e.button === 1) closeTab(path); }}
                onKeyDown={(e) => { if (e.key === 'Enter') p.onActivate(path); }}>
                {baseName(path)}
                {byAgent && <span className="changed-dot" aria-label="cambiado por el agente" />}
                <span className="tab-close" role="button" aria-label={`Cerrar ${baseName(path)}`}
                  onClick={(e) => { e.stopPropagation(); closeTab(path); }}>
                  {isDirty ? <span className="dirty-dot" /> : <Icon name="close" size={12} />}
                </span>
              </div>
            );
          })}
        </div>
      )}
      {p.active && stale && (
        <div className="banner" style={{ background: 'var(--warn-bg)', borderColor: 'var(--warn-line)' }}>
          <Icon name="warning" className="warn" />
          <span className="grow">El agente cambió este archivo en disco y tienes cambios sin guardar.</span>
          <button className="btn small" onClick={reloadFromDisk}>Cargar la versión del disco</button>
          <button className="btn small" onClick={() => p.active && save(p.active)}>Guardar la mía</button>
        </div>
      )}
      {p.active && baseline !== undefined && (
        <div className="banner">
          <Icon name="diff" />
          {baseline === null
            ? <span className="grow">El agente creó este archivo en la última misión.</span>
            : <span className="grow">El agente cambió este archivo: <strong><span className="ok">+{counts.added}</span> <span className="err">−{counts.removed}</span></strong> líneas
              {p.workspace && <span className="dim"> · {relativePath(p.workspace, p.active)}</span>}</span>}
          {baseline !== null && <>
            <button className="btn small" aria-pressed={showDiff} onClick={() => setShowDiff(!showDiff)}>{showDiff ? 'Volver al editor' : 'Ver cambios'}</button>
            <button className="btn small" onClick={undoAgent}><Icon name="undo" size={14} />Deshacer</button>
          </>}
        </div>
      )}
      <div className="editor-host" style={{ display: p.active ? undefined : 'none' }}>
        <div ref={host} style={{ visibility: showDiff ? 'hidden' : 'visible' }} />
        {showDiff && <div ref={diffHost} />}
      </div>
      {!p.active && p.welcome}
    </div>
  );
});
