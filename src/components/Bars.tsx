import { useEffect, useRef, useState } from 'react';
import type { GitInfo, SystemStats } from '../lib/api';
import { baseName, formatTokens, gb } from '../lib/text';
import { Icon, type IconName } from './Icon';

function Meter({ label, value, max, text }: { label: string; value: number; max: number; text: string }) {
  const ratio = max > 0 ? Math.min(1, value / max) : 0;
  const tone = ratio > 0.95 ? 'err' : ratio > 0.8 ? 'warn' : '';
  return (
    <div className="meter" role="meter" aria-label={label} aria-valuenow={Math.round(ratio * 100)} aria-valuemin={0} aria-valuemax={100}>
      <div className="meter-label"><span>{label}</span><span>{text}</span></div>
      <div className="meter-track"><div className={`meter-fill ${tone}`} style={{ width: `${ratio * 100}%` }} /></div>
    </div>
  );
}

export { Meter };

interface TopBarProps {
  workspace: string | null;
  recents: string[];
  onOpenFolder: () => void;
  onPickRecent: (path: string) => void;
  onPalette: () => void;
  models: string[];
  model: string;
  onModel: (model: string) => void;
  bionic: 'ok' | 'err' | 'unknown';
  context: number | null;
  stats: SystemStats | null;
  onSettings: () => void;
}

export function TopBar(p: TopBarProps) {
  const [menu, setMenu] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => { if (!menuRef.current?.contains(e.target as Node)) setMenu(false); };
    window.addEventListener('mousedown', close);
    return () => window.removeEventListener('mousedown', close);
  }, [menu]);
  const gpu = p.stats?.gpu;
  return (
    <header className="topbar">
      <div className="brand"><Icon name="logo" size={20} />Aura Sentinel</div>
      <div ref={menuRef} style={{ position: 'relative' }}>
        <button className="workspace-btn" aria-label="Cambiar espacio de trabajo" aria-expanded={menu} onClick={() => setMenu(!menu)}
          title={p.workspace ?? ''}>
          <Icon name="folder" className="muted" />
          <span className="ellipsis">{p.workspace ? baseName(p.workspace) : 'Abrir carpeta'}</span>
          <Icon name="chevronDown" size={14} className="muted" />
        </button>
        {menu && (
          <div className="mentions" style={{ top: 'calc(100% + 4px)', bottom: 'auto', right: 'auto', width: 320 }} role="menu">
            <button role="menuitem" onClick={() => { setMenu(false); p.onOpenFolder(); }}>
              <Icon name="plus" size={14} /> Abrir carpeta…
            </button>
            {p.recents.length > 0 && <div className="section-title" style={{ padding: '6px 8px 2px' }}>RECIENTES</div>}
            {p.recents.map((path) => (
              <button key={path} role="menuitem" title={path} onClick={() => { setMenu(false); p.onPickRecent(path); }}>
                <div>{baseName(path)}</div><div className="dim ellipsis" style={{ fontSize: 11 }}>{path}</div>
              </button>
            ))}
          </div>
        )}
      </div>
      <button className="palette-btn" onClick={p.onPalette} aria-label="Paleta de comandos">
        <Icon name="search" /> <span className="grow">Buscar archivo o ejecutar comando</span><span className="kbd">Ctrl+K</span>
      </button>
      <div className="topbar-right">
        <label className="model-chip" title={p.bionic === 'err' ? 'Bionic no responde en :1235' : 'Modelo para las misiones'}>
          <span className={`dot ${p.bionic === 'ok' ? 'ok' : p.bionic === 'err' ? 'err' : ''}`} />
          <span className="sr-only">Modelo activo</span>
          <select value={p.model} onChange={(e) => p.onModel(e.target.value)} disabled={!p.models.length}>
            {!p.models.length && <option value="">sin modelos</option>}
            {p.models.map((m) => <option key={m} value={m}>{m}</option>)}
          </select>
          {p.context ? <span className="dim">{formatTokens(p.context)}</span> : null}
        </label>
        {gpu && <Meter label="VRAM" value={gpu.used} max={gpu.total} text={`${gb(gpu.used / 1024)} / ${gb(gpu.total / 1024)} GB`} />}
        <button className="icon-btn bordered" aria-label="Configuración" onClick={p.onSettings}><Icon name="settings" /></button>
      </div>
    </header>
  );
}

export type SideView = 'explorer' | 'search' | 'git' | 'restore' | 'rules';

const VIEWS: { id: SideView; icon: IconName; label: string }[] = [
  { id: 'explorer', icon: 'file', label: 'Explorador (Ctrl+Shift+E)' },
  { id: 'search', icon: 'search', label: 'Buscar en el proyecto (Ctrl+Shift+F)' },
  { id: 'git', icon: 'git', label: 'Control de versiones (git)' },
  { id: 'restore', icon: 'history', label: 'Puntos de restauración' },
  { id: 'rules', icon: 'book', label: 'Reglas y lecciones del agente' },
];

export function ActivityBar({ view, onView, changes, onProfile }: {
  view: SideView | null; onView: (v: SideView) => void; changes: number; onProfile: () => void;
}) {
  return (
    <nav className="activity" aria-label="Vistas">
      {VIEWS.map((v) => (
        <button key={v.id} aria-label={v.label} title={v.label} aria-pressed={view === v.id} onClick={() => onView(v.id)}>
          <Icon name={v.icon} size={20} />
          {v.id === 'git' && changes > 0 && <span className="badge">{changes}</span>}
        </button>
      ))}
      <div className="spacer" />
      <button aria-label="Perfil y preferencias" title="Perfil y preferencias" onClick={onProfile}><Icon name="user" size={20} /></button>
    </nav>
  );
}

export function StatusBar({ git, errors, warnings, server, bionic, stats, onProblems, onPreview, onGit, onToggleBottom }: {
  git: GitInfo | null; errors: number; warnings: number; server: string | null; bionic: 'ok' | 'err' | 'unknown';
  stats: SystemStats | null; onProblems: () => void; onPreview: () => void; onGit: () => void; onToggleBottom: () => void;
}) {
  return (
    <footer className="statusbar" aria-label="Barra de estado">
      {git?.branch && <button onClick={onGit}><Icon name="git" size={13} />{git.branch}{git.changes.length ? ` · ${git.changes.length}` : ''}</button>}
      <button onClick={onProblems} className={errors ? 'err' : ''}><Icon name="error" size={13} />{errors}</button>
      <button onClick={onProblems} className={warnings ? 'warn' : ''}><Icon name="warning" size={13} />{warnings}</button>
      {server && <button onClick={onPreview}><Icon name="eye" size={13} />{server.replace(/^https?:\/\//, '')}</button>}
      <div className="right">
        <button onClick={onToggleBottom} title="Panel inferior (Ctrl+J)"><Icon name="terminal" size={13} />Panel</button>
        <span className={bionic === 'err' ? 'err' : ''}>Bionic :1235 {bionic === 'ok' ? '✓' : bionic === 'err' ? '✗' : ''}</span>
        {stats && <span>CPU {Math.round(stats.cpu)}%</span>}
        {stats && <span>RAM {gb(stats.ramUsed)} / {gb(stats.ramTotal)} GB</span>}
      </div>
    </footer>
  );
}

/** Divisor arrastrable: devuelve el nuevo tamaño en px. */
export function Splitter({ axis, size, onSize, min, max, invert = false }: {
  axis: 'x' | 'y'; size: number; onSize: (n: number) => void; min: number; max: number; invert?: boolean;
}) {
  const [dragging, setDragging] = useState(false);
  const start = (e: React.PointerEvent) => {
    const from = axis === 'x' ? e.clientX : e.clientY;
    const initial = size;
    setDragging(true);
    const move = (ev: PointerEvent) => {
      const delta = (axis === 'x' ? ev.clientX : ev.clientY) - from;
      onSize(Math.max(min, Math.min(max, initial + (invert ? -delta : delta))));
    };
    const up = () => {
      setDragging(false);
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return <div className={`splitter-${axis}${dragging ? ' dragging' : ''}`} onPointerDown={start} role="separator"
    aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'} />;
}
