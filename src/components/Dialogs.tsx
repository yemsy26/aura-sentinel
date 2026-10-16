import { Fragment, useEffect, useMemo, useRef, useState } from 'react';
import { api, errorText, type AgentSettings, type ProfileSnapshot, type UserProfile } from '../lib/api';
import { Icon, type IconName } from './Icon';

export interface PaletteCommand { id: string; label: string; icon: IconName; hint?: string; run: () => void }

export function CommandPalette({ commands, files, onOpenFile, onClose }: {
  commands: PaletteCommand[]; files: { path: string; label: string }[]; onOpenFile: (path: string) => void; onClose: () => void;
}) {
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    const cmd = q.startsWith('>') ? q.slice(1).trim() : q;
    const cmds = commands.filter((c) => c.label.toLowerCase().includes(cmd))
      .map((c) => ({ key: c.id, label: c.label, icon: c.icon, hint: c.hint, run: c.run }));
    if (q.startsWith('>')) return cmds;
    const fs = files.filter((f) => f.label.toLowerCase().includes(q)).slice(0, 40)
      .map((f) => ({ key: f.path, label: f.label, icon: 'file' as IconName, hint: 'archivo', run: () => onOpenFile(f.path) }));
    return q ? [...fs, ...cmds] : [...cmds, ...fs];
  }, [query, commands, files, onOpenFile]);
  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' });
  }, [index]);
  const choose = (i: number) => { const item = items[i]; if (!item) return; onClose(); item.run(); };
  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="dialog palette" role="dialog" aria-label="Paleta de comandos" onMouseDown={(e) => e.stopPropagation()}>
        <input autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Archivo o comando (> solo comandos)"
          aria-label="Buscar archivo o comando"
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') { e.preventDefault(); setIndex((i) => Math.min(i + 1, items.length - 1)); }
            if (e.key === 'ArrowUp') { e.preventDefault(); setIndex((i) => Math.max(i - 1, 0)); }
            if (e.key === 'Enter') { e.preventDefault(); choose(index); }
            if (e.key === 'Escape') onClose();
          }} />
        <div className="palette-list" ref={list} role="listbox">
          {items.map((item, i) => (
            <button key={item.key} className="palette-item" role="option" aria-selected={i === index}
              onMouseEnter={() => setIndex(i)} onClick={() => choose(i)}>
              <Icon name={item.icon} className="muted" /><span className="ellipsis">{item.label}</span>
              {item.hint && <span className="hint">{item.hint}</span>}
            </button>
          ))}
          {!items.length && <div className="empty">Nada coincide.</div>}
        </div>
      </div>
    </div>
  );
}

const STYLES: [string, string][] = [['', 'Sin preferencia'], ['conciso', 'Conciso'], ['detallado', 'Detallado'], ['paso a paso', 'Paso a paso']];
const LANGUAGES: [string, string][] = [['', 'Sin preferencia'], ['es', 'Español'], ['en', 'English'], ['pt', 'Português']];

export function SettingsDialog({ initialTab, workspace, onClose, onToast, onEngineChanged }: {
  initialTab: 'engine' | 'profile' | 'keys'; workspace: string | null; onClose: () => void;
  onToast: (t: string, error?: boolean) => void; onEngineChanged: () => void;
}) {
  const [tab, setTab] = useState(initialTab);
  const [engine, setEngine] = useState<AgentSettings | null>(null);
  const [folder, setFolder] = useState('');
  const [snapshot, setSnapshot] = useState<ProfileSnapshot | null>(null);
  const [profile, setProfile] = useState<UserProfile>({ autoRememberFacts: false });

  useEffect(() => {
    void api.agentSettings().then((s) => { setEngine(s); setFolder(s.carpeta); });
    void api.profile(workspace).then((s) => { setSnapshot(s); setProfile(s.profile); }).catch(() => undefined);
  }, [workspace]);

  const pickFolder = async () => {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const dir = await open({ directory: true, defaultPath: folder || undefined });
    if (typeof dir === 'string') setFolder(dir);
  };
  const saveEngine = async () => {
    try {
      setEngine(await api.saveAgentSettings(folder));
      onToast('Carpeta de Agente Local guardada.');
      onEngineChanged();
    } catch (e) { onToast(errorText(e), true); }
  };
  const saveProfile = async () => {
    try { setProfile(await api.saveProfile(profile)); onToast('Perfil guardado.'); } catch (e) { onToast(errorText(e), true); }
  };
  const field = (key: keyof UserProfile, label: string, placeholder = '') => (
    <label className="field"><span>{label}</span>
      <input className="input" value={(profile[key] as string | null | undefined) ?? ''} placeholder={placeholder}
        onChange={(e) => setProfile({ ...profile, [key]: e.target.value || null })} />
    </label>
  );
  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="dialog" role="dialog" aria-label="Configuración" onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}>
        <div className="dialog-head">
          <h2 className="grow">Configuración</h2>
          <div className="seg">
            <button aria-pressed={tab === 'engine'} onClick={() => setTab('engine')}>Motor</button>
            <button aria-pressed={tab === 'profile'} onClick={() => setTab('profile')}>Perfil</button>
            <button aria-pressed={tab === 'keys'} onClick={() => setTab('keys')}>Atajos</button>
          </div>
          <button className="icon-btn" aria-label="Cerrar" onClick={onClose}><Icon name="close" /></button>
        </div>
        {tab === 'engine' && <>
          <div className="dialog-body">
            <p className="muted" style={{ margin: 0 }}>Las misiones las ejecuta <strong>Agente Local</strong> con el modelo cargado en Bionic (puerto 1235).
              El modelo se elige en la barra superior; si no está cargado, el agente lo carga ajustado a la VRAM.</p>
            <label className="field"><span>Carpeta de Agente Local</span>
              <div className="field-row">
                <input className="input grow mono" value={folder} onChange={(e) => setFolder(e.target.value)} />
                <button className="btn" onClick={pickFolder}><Icon name="folder" />Elegir</button>
              </div>
            </label>
            {engine && <div className={engine.instalado ? 'ok' : 'err'}>
              {engine.instalado ? '✓ Instalación encontrada' : '✗ No es una instalación válida (necesita .venv y agente\\__main__.py; ejecuta instalar.bat)'}</div>}
          </div>
          <div className="dialog-foot"><button className="btn primary" onClick={saveEngine} disabled={!folder.trim()}>Guardar</button></div>
        </>}
        {tab === 'profile' && <>
          <div className="dialog-body">
            <p className="muted" style={{ margin: 0 }}>Se comparte con el agente al empezar cada conversación. Se guarda solo en este equipo.</p>
            <div className="field-row">{field('displayName', 'Nombre')}
              <label className="field grow"><span>Idioma</span>
                <select className="input" value={profile.preferredLanguage ?? ''} onChange={(e) => setProfile({ ...profile, preferredLanguage: e.target.value || null })}>
                  {LANGUAGES.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
                </select>
              </label>
            </div>
            <div className="field-row">{field('city', 'Ciudad')}{field('homeCountry', 'País (código de 2 letras)', snapshot?.windowsRegionSuggestion ?? 'ES')}</div>
            <div className="field-row">{field('timeZone', 'Zona horaria', 'Europe/Madrid')}
              <label className="field grow"><span>Estilo de respuesta</span>
                <select className="input" value={profile.responseStyle ?? ''} onChange={(e) => setProfile({ ...profile, responseStyle: e.target.value || null })}>
                  {STYLES.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
                </select>
              </label>
            </div>
            <label style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <input type="checkbox" checked={profile.autoRememberFacts} onChange={(e) => setProfile({ ...profile, autoRememberFacts: e.target.checked })} />
              Recordar datos que diga explícitamente («me llamo…», «vivo en…»)
            </label>
          </div>
          <div className="dialog-foot">
            <button className="btn danger" onClick={async () => {
              if (!window.confirm('¿Borrar el perfil local?')) return;
              await api.clearProfile(); setProfile({ autoRememberFacts: false }); onToast('Perfil borrado.');
            }}>Borrar perfil</button>
            <button className="btn primary" onClick={saveProfile}>Guardar</button>
          </div>
        </>}
        {tab === 'keys' && <div className="dialog-body"><Shortcuts /></div>}
      </div>
    </div>
  );
}

export const SHORTCUTS: [string, string][] = [
  ['Ctrl+K / Ctrl+P', 'Paleta: archivos y comandos'], ['Ctrl+S', 'Guardar archivo'], ['Ctrl+L', 'Escribir al agente'],
  ['Ctrl+J', 'Mostrar u ocultar el panel inferior'], ['Ctrl+B', 'Mostrar u ocultar el panel lateral'],
  ['Ctrl+Shift+F', 'Buscar en el proyecto'], ['Ctrl+Shift+E', 'Explorador'], ['Ctrl+W', 'Cerrar pestaña'],
  ['Esc', 'Cerrar diálogos'],
];

export function Shortcuts() {
  return (
    <div className="shortcuts" style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 18px' }}>
      {SHORTCUTS.map(([k, d]) => <Fragment key={k}><span className="kbd" style={{ color: 'var(--accent-soft)' }}>{k}</span><span>{d}</span></Fragment>)}
    </div>
  );
}
