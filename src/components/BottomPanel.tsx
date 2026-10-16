import { useEffect, useRef, useState } from 'react';
import type { AgentProcess, CommandOutput } from '../lib/api';
import type { Problem } from '../lib/agent';
import { serverUrl } from '../lib/text';
import { Icon } from './Icon';

export type BottomTab = 'terminal' | 'problems' | 'processes' | 'preview';

interface Props {
  tab: BottomTab;
  onTab: (t: BottomTab) => void;
  height: number;
  workspace: string | null;
  problems: Problem[];
  processes: AgentProcess[];
  busy: boolean;
  previewUrl: string;
  onPreviewUrl: (url: string) => void;
  onRun: (command: string) => Promise<CommandOutput>;
  onStopProcess: (id: number) => void;
  onRefreshProcesses: () => void;
  onOpenExternal: (url: string) => void;
  onAskAgent: (text: string) => void;
  onHide: () => void;
}

interface TermEntry { command: string; result: CommandOutput | null }

export function BottomPanel(p: Props) {
  const errors = p.problems.filter((x) => x.severity === 'error').length;
  const alive = p.processes.filter((x) => x.vivo);
  const tabs: { id: BottomTab; label: string; badge?: number; tone?: string }[] = [
    { id: 'terminal', label: 'Terminal' },
    { id: 'problems', label: 'Problemas', badge: p.problems.length, tone: errors ? 'err' : 'warn' },
    { id: 'processes', label: 'Procesos', badge: alive.length },
    { id: 'preview', label: 'Vista previa' },
  ];
  return (
    <section className="bottom" style={{ height: p.height }} aria-label="Panel inferior">
      <div className="bottom-tabs" role="tablist">
        {tabs.map((t) => (
          <button key={t.id} role="tab" aria-selected={p.tab === t.id} className="bottom-tab" onClick={() => p.onTab(t.id)}>
            {t.label}{t.badge ? <span className={`badge ${t.tone ?? ''}`}>{t.badge}</span> : null}
          </button>
        ))}
        <span className="grow" />
        <button className="icon-btn" aria-label="Ocultar panel" title="Ocultar panel (Ctrl+J)" onClick={p.onHide}><Icon name="close" size={14} /></button>
      </div>
      <div className="bottom-body">
        {p.tab === 'terminal' && <Terminal workspace={p.workspace} onRun={p.onRun} />}
        {p.tab === 'problems' && (p.problems.length ? (
          <ul style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
            {p.problems.map((x, i) => (
              <li key={i} className="problem">
                <Icon name={x.severity === 'error' ? 'error' : 'warning'} className={x.severity === 'error' ? 'err' : 'warn'} />
                <span className="grow">{x.text}</span>
                <span className="src">{x.source === 'ver_pantalla' ? 'captura' : x.source}</span>
              </li>
            ))}
            <li className="problem">
              <button className="btn small" disabled={p.busy}
                onClick={() => p.onAskAgent('Corrige estos problemas de la última verificación:\n' + p.problems.map((x) => `- ${x.text}`).join('\n'))}>
                <Icon name="spark" size={14} />Pedir al agente que los corrija
              </button>
            </li>
          </ul>
        ) : <div className="empty">Sin problemas. Aparecen al verificar el proyecto o al revisar la pantalla.</div>)}
        {p.tab === 'processes' && <>
          <div className="preview-bar">
            <span className="muted grow">Procesos que el agente dejó en marcha (servidores, juegos…)</span>
            <button className="btn small" onClick={p.onRefreshProcesses} disabled={p.busy}><Icon name="refresh" size={14} />Actualizar</button>
          </div>
          {p.processes.length ? p.processes.map((x) => {
            const url = serverUrl(x.comando);
            return (
              <div key={x.id} className="problem" style={{ alignItems: 'center' }}>
                <span className={`dot ${x.vivo ? 'ok' : ''}`} />
                <code className="grow ellipsis" title={x.comando}>#{x.id} {x.comando}</code>
                {url && x.vivo && <button className="btn small" onClick={() => { p.onPreviewUrl(url); p.onTab('preview'); }}>Vista previa</button>}
                {x.vivo && <button className="btn small danger" disabled={p.busy} onClick={() => p.onStopProcess(x.id)}>Detener</button>}
              </div>
            );
          }) : <div className="empty">Ninguno.</div>}
        </>}
        {p.tab === 'preview' && <Preview url={p.previewUrl} onUrl={p.onPreviewUrl} onOpenExternal={p.onOpenExternal} />}
      </div>
    </section>
  );
}

function Terminal({ workspace, onRun }: { workspace: string | null; onRun: (c: string) => Promise<CommandOutput> }) {
  const [log, setLog] = useState<TermEntry[]>([]);
  const [command, setCommand] = useState('');
  const [history, setHistory] = useState<string[]>([]);
  const [cursor, setCursor] = useState(-1);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => end.current?.scrollIntoView({ block: 'end' }), [log]);
  const running = log.some((e) => !e.result);
  const run = async () => {
    const c = command.trim();
    if (!c || running || !workspace) return;
    if (c === 'cls' || c === 'clear') { setLog([]); setCommand(''); return; }
    setCommand('');
    setHistory((h) => [c, ...h.filter((x) => x !== c)].slice(0, 50));
    setCursor(-1);
    setLog((l) => [...l, { command: c, result: null }]);
    const result = await onRun(c);
    setLog((l) => l.map((e, i) => (i === l.length - 1 ? { ...e, result } : e)));
  };
  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div className="grow" style={{ overflow: 'auto' }}>
        {!log.length && <div className="empty">Ejecuta comandos en la carpeta del proyecto (cmd, máximo 2 minutos). Ej.: <code>npm test</code>, <code>python -m pytest</code>, <code>git status</code>.</div>}
        {log.map((e, i) => (
          <pre key={i} className="term-out">
            <span className="term-cmd">&gt; {e.command}</span>{'\n'}
            {e.result ? <>{e.result.output.trimEnd()}{'\n'}<span className={e.result.code === 0 ? 'ok' : 'err'}>
              [código {e.result.code} · {(e.result.ms / 1000).toFixed(1)} s]</span></> : <span className="dim">ejecutando…</span>}
          </pre>
        ))}
        <div ref={end} />
      </div>
      <form className="term-input" onSubmit={(e) => { e.preventDefault(); void run(); }}>
        <span className="term-cmd mono">&gt;</span>
        <input value={command} onChange={(e) => setCommand(e.target.value)} disabled={!workspace} aria-label="Comando"
          placeholder={workspace ? 'Escribe un comando y pulsa Enter' : 'Abre una carpeta primero'}
          onKeyDown={(e) => {
            if (e.key === 'ArrowUp' && history.length) { e.preventDefault(); const n = Math.min(cursor + 1, history.length - 1); setCursor(n); setCommand(history[n]); }
            if (e.key === 'ArrowDown' && cursor >= 0) { e.preventDefault(); const n = cursor - 1; setCursor(n); setCommand(n >= 0 ? history[n] : ''); }
          }} />
      </form>
    </div>
  );
}

function Preview({ url, onUrl, onOpenExternal }: { url: string; onUrl: (u: string) => void; onOpenExternal: (u: string) => void }) {
  const [draft, setDraft] = useState(url);
  const [key, setKey] = useState(0);
  useEffect(() => setDraft(url), [url]);
  return (
    <div className="preview">
      <form className="preview-bar" onSubmit={(e) => { e.preventDefault(); onUrl(draft.trim()); setKey((k) => k + 1); }}>
        <input className="input grow mono" value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="http://localhost:8000"
          aria-label="Dirección de la vista previa" />
        <button className="icon-btn bordered" aria-label="Recargar" title="Recargar" type="submit"><Icon name="refresh" size={14} /></button>
        <button className="icon-btn bordered" type="button" aria-label="Abrir en el navegador" title="Abrir en el navegador"
          disabled={!url} onClick={() => onOpenExternal(url)}><Icon name="external" size={14} /></button>
      </form>
      {url ? <iframe key={key} src={url} title="Vista previa" sandbox="allow-scripts allow-same-origin allow-forms allow-modals" />
        : <div className="empty">Indica la dirección de un servidor local (los que lanza el agente aparecen en Procesos).</div>}
    </div>
  );
}
