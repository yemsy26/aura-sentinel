import { useEffect, useMemo, useRef, useState } from 'react';
import type { Checkpoint, FileNode, GitInfo, SearchHit } from '../lib/api';
import { joinPath, relativePath, samePath } from '../lib/text';
import type { SideView } from './Bars';
import { Icon } from './Icon';

interface TreeNode { node: FileNode; children: TreeNode[] }

export function buildTree(nodes: FileNode[], root: string): TreeNode[] {
  const byParent = new Map<string, FileNode[]>();
  for (const n of nodes) {
    if (samePath(n.path, root)) continue;
    const key = (n.parent_path ?? '').toLowerCase();
    byParent.set(key, [...(byParent.get(key) ?? []), n]);
  }
  const build = (parent: string): TreeNode[] =>
    (byParent.get(parent.toLowerCase()) ?? [])
      .sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || a.name.localeCompare(b.name, 'es', { sensitivity: 'base' }))
      .map((node) => ({ node, children: node.is_dir ? build(node.path) : [] }));
  return build(root);
}

function Tree({ items, depth, open, toggle, active, marks, onOpen }: {
  items: TreeNode[]; depth: number; open: Set<string>; toggle: (p: string) => void; active: string | null;
  marks: Record<string, 'M' | 'A'>; onOpen: (p: string) => void;
}) {
  return (
    <ul className="tree" role={depth ? 'group' : 'tree'}>
      {items.map(({ node, children }) => {
        const expanded = open.has(node.path);
        const mark = marks[node.path.toLowerCase()];
        return (
          <li key={node.path} role="treeitem" aria-expanded={node.is_dir ? expanded : undefined}>
            <button className={`tree-row${node.is_dir ? ' dir' : ''}${active && samePath(active, node.path) ? ' active' : ''}`}
              style={{ paddingLeft: 8 + depth * 14 }} title={node.path}
              onClick={() => (node.is_dir ? toggle(node.path) : onOpen(node.path))}>
              {node.is_dir ? <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={14} /> : <span style={{ width: 14 }} />}
              <span className="ellipsis">{node.name}</span>
              {mark && <span className={`mark ${mark}`} title={mark === 'A' ? 'Creado por el agente' : 'Cambiado por el agente'}>{mark}</span>}
            </button>
            {node.is_dir && expanded && children.length > 0 &&
              <Tree items={children} depth={depth + 1} open={open} toggle={toggle} active={active} marks={marks} onOpen={onOpen} />}
          </li>
        );
      })}
    </ul>
  );
}

interface SidePanelProps {
  view: SideView;
  width: number;
  workspace: string | null;
  tree: FileNode[];
  active: string | null;
  marks: Record<string, 'M' | 'A'>;
  git: GitInfo | null;
  checkpoints: Checkpoint[];
  rules: { reglas: string; lecciones: string } | null;
  busy: boolean;
  onOpen: (path: string, line?: number) => void;
  onRefresh: () => void;
  onOpenFolder: () => void;
  onSearch: (query: string) => Promise<SearchHit[]>;
  onRestore: (numero: number) => void;
  onAddRule: (text: string) => void;
}

export function SidePanel(p: SidePanelProps) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  const items = useMemo(() => (p.workspace ? buildTree(p.tree, p.workspace) : []), [p.tree, p.workspace]);
  const toggle = (path: string) => setOpen((s) => {
    const next = new Set(s);
    if (next.has(path)) next.delete(path); else next.add(path);
    return next;
  });
  const changedCount = Object.keys(p.marks).length;

  if (!p.workspace) {
    return (
      <aside className="side" style={{ width: p.width }} aria-label="Panel lateral">
        <div className="empty">No hay carpeta abierta.<br /><br />
          <button className="btn primary" onClick={p.onOpenFolder}><Icon name="folder" />Abrir carpeta</button>
        </div>
      </aside>
    );
  }
  return (
    <aside className="side" style={{ width: p.width }} aria-label="Panel lateral">
      {p.view === 'explorer' && <>
        <div className="side-head">EXPLORADOR
          <span className="actions">
            {changedCount > 0 && <span className="dim" style={{ fontWeight: 400, letterSpacing: 0, alignSelf: 'center', marginRight: 6 }}>
              {changedCount} {changedCount === 1 ? 'cambio' : 'cambios'} del agente</span>}
            <button className="icon-btn" aria-label="Recargar árbol" title="Recargar árbol" onClick={p.onRefresh}><Icon name="refresh" size={14} /></button>
          </span>
        </div>
        <div className="side-body">
          {items.length ? <Tree items={items} depth={0} open={open} toggle={toggle} active={p.active} marks={p.marks} onOpen={p.onOpen} />
            : <div className="empty">Carpeta vacía. Pide al agente que cree algo.</div>}
        </div>
        <RestorePoints checkpoints={p.checkpoints.slice(0, 3)} busy={p.busy} onRestore={p.onRestore} compact />
      </>}
      {p.view === 'search' && <SearchView workspace={p.workspace} onSearch={p.onSearch} onOpen={p.onOpen} />}
      {p.view === 'git' && <GitView git={p.git} workspace={p.workspace} onOpen={p.onOpen} onRefresh={p.onRefresh} />}
      {p.view === 'restore' && <>
        <div className="side-head">PUNTOS DE RESTAURACIÓN</div>
        <div className="side-body" style={{ padding: '0 12px' }}>
          <p className="muted" style={{ marginTop: 0 }}>El agente guarda un punto cada vez que verificar pasa sin errores (los 3 últimos).
            Volver a uno reescribe esos archivos; los creados después no se tocan.</p>
          <RestorePoints checkpoints={p.checkpoints} busy={p.busy} onRestore={p.onRestore} />
        </div>
      </>}
      {p.view === 'rules' && <RulesView rules={p.rules} busy={p.busy} onAddRule={p.onAddRule} />}
    </aside>
  );
}

function RestorePoints({ checkpoints, busy, onRestore, compact = false }: {
  checkpoints: Checkpoint[]; busy: boolean; onRestore: (n: number) => void; compact?: boolean;
}) {
  if (compact && !checkpoints.length) return null;
  const body = checkpoints.length ? checkpoints.map((cp, i) => (
    <div key={cp.numero} className="list-row" style={{ padding: compact ? '2px 0' : undefined }}>
      <div className="grow">
        <div className={i ? 'muted' : ''}>#{cp.numero} · {i === 0 ? 'último verificado' : `paso ${cp.paso}`}</div>
        <div className="dim" style={{ fontSize: 11 }}>{cp.fecha} · {cp.archivos.length} archivos</div>
      </div>
      <button className={`btn small${i === 0 ? ' accent' : ''}`} disabled={busy} onClick={() => onRestore(cp.numero)}
        title={cp.archivos.join('\n')}>Volver</button>
    </div>
  )) : <div className="dim">Aún no hay ninguno.</div>;
  return compact ? <div className="side-section"><div className="section-title">PUNTOS DE RESTAURACIÓN</div>{body}</div> : <>{body}</>;
}

function SearchView({ workspace, onSearch, onOpen }: {
  workspace: string; onSearch: (q: string) => Promise<SearchHit[]>; onOpen: (p: string, line?: number) => void;
}) {
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { input.current?.focus(); }, []);
  useEffect(() => {
    if (query.trim().length < 2) { setHits(null); return; }
    let live = true;
    const t = setTimeout(() => onSearch(query).then((h) => live && setHits(h)), 250);
    return () => { live = false; clearTimeout(t); };
  }, [query, onSearch]);
  const groups = useMemo(() => {
    const g = new Map<string, SearchHit[]>();
    for (const h of hits ?? []) g.set(h.path, [...(g.get(h.path) ?? []), h]);
    return [...g.entries()];
  }, [hits]);
  return <>
    <div className="side-head">BUSCAR</div>
    <div style={{ padding: '0 10px 8px' }}>
      <input ref={input} className="input" style={{ width: '100%' }} placeholder="Texto en los archivos" value={query}
        onChange={(e) => setQuery(e.target.value)} aria-label="Buscar en el proyecto" />
      {hits && <div className="dim" style={{ fontSize: 12, marginTop: 6 }}>{hits.length >= 300 ? 'más de 300' : hits.length} resultados en {groups.length} archivos</div>}
    </div>
    <div className="side-body">
      {groups.map(([path, list]) => (
        <div key={path}>
          <div className="hit-file ellipsis" title={path}>{relativePath(workspace, path)}</div>
          {list.map((h) => <button key={h.line} className="hit ellipsis" onClick={() => onOpen(h.path, h.line)}>{h.line}: {h.text}</button>)}
        </div>
      ))}
    </div>
  </>;
}

function GitView({ git, workspace, onOpen, onRefresh }: {
  git: GitInfo | null; workspace: string; onOpen: (p: string) => void; onRefresh: () => void;
}) {
  return <>
    <div className="side-head">GIT
      <button className="icon-btn" aria-label="Actualizar" title="Actualizar" onClick={onRefresh}><Icon name="refresh" size={14} /></button>
    </div>
    <div className="side-body">
      {!git?.branch ? <div className="empty">Esta carpeta no es un repositorio git. Pide al agente «inicia git y haz el primer commit».</div> : <>
        <div className="list-row"><Icon name="git" />Rama <strong>{git.branch}</strong></div>
        <div className="section-title" style={{ padding: '8px 8px 4px' }}>CAMBIOS ({git.changes.length})</div>
        {git.changes.map((c) => (
          <button key={c.path} className="tree-row" onClick={() => onOpen(joinPath(git.root ?? workspace, c.path))}>
            <span className="ellipsis">{c.path}</span>
            <span className={`mark ${c.status.includes('?') || c.status.includes('A') ? 'A' : 'M'}`}>{c.status === '??' ? 'U' : c.status}</span>
          </button>
        ))}
        <div className="section-title" style={{ padding: '12px 8px 4px' }}>ÚLTIMOS COMMITS</div>
        {git.log.map((l) => <div key={l} className="hit ellipsis" style={{ paddingLeft: 8 }} title={l}>{l}</div>)}
      </>}
    </div>
  </>;
}

function RulesView({ rules, busy, onAddRule }: {
  rules: { reglas: string; lecciones: string } | null; busy: boolean; onAddRule: (t: string) => void;
}) {
  const [text, setText] = useState('');
  return <>
    <div className="side-head">REGLAS DEL AGENTE</div>
    <div className="side-body" style={{ padding: '0 12px', display: 'flex', flexDirection: 'column', gap: 10 }}>
      <p className="muted" style={{ margin: 0 }}>Se aplican en todas las tareas. Ejemplo: «usa siempre pytest».</p>
      <form className="field-row" onSubmit={(e) => { e.preventDefault(); if (text.trim()) { onAddRule(text.trim()); setText(''); } }}>
        <input className="input grow" value={text} onChange={(e) => setText(e.target.value)} placeholder="Nueva regla" aria-label="Nueva regla" />
        <button className="btn" disabled={busy || !text.trim()}>Añadir</button>
      </form>
      <div className="section-title">REGLAS</div>
      <pre className="rules">{rules?.reglas.trim() || 'Sin reglas todavía.'}</pre>
      <div className="section-title">LECCIONES APRENDIDAS</div>
      <pre className="rules">{rules?.lecciones.trim() || 'Sin lecciones todavía.'}</pre>
    </div>
  </>;
}
