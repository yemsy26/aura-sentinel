import { useEffect, useMemo, useRef, useState } from 'react';
import { PHASE_LABEL, toolLabel, type AgentState, type ChatItem, type PlanItem, type RunEntry, type Step, type Summary } from '../lib/agent';
import { api, type PendingMission } from '../lib/api';
import { formatElapsed, formatTokens, joinPath, markdown, relativePath } from '../lib/text';
import { Meter } from './Bars';
import { Icon, type IconName } from './Icon';

const TOOL_ICON: Record<string, IconName> = {
  leer: 'file', escribir: 'edit', editar: 'edit', ejecutar: 'terminal', verificar: 'check', revisar: 'search',
  ver_pantalla: 'eye', plan: 'book', paso_hecho: 'check', git: 'git', restaurar: 'history', buscar: 'search',
  listar: 'folder', web_leer: 'external', web_buscar: 'search', crear_bat: 'play', terminar: 'check',
};

export type QuickKind = 'verificar' | 'revisar' | 'continuar' | 'nuevo';

interface Props {
  state: AgentState;
  width: number;
  workspace: string | null;
  files: string[];
  pending: PendingMission | null;
  onSend: (text: string) => void;
  onStop: () => void;
  onQuick: (kind: QuickKind) => void;
  onAnswer: (option: string) => void;
  onDismissPending: () => void;
  onOpenFile: (path: string) => void;
  onOpenUrl: (url: string) => void;
  draft?: { text: string; id: number } | null; // texto que llega del editor («Pedir al agente sobre esto»)
  reviewPlan?: boolean;
  onReviewPlan?: (value: boolean) => void;
}

export function AgentPanel(p: Props) {
  const { state } = p;
  const [now, setNow] = useState(Date.now());
  const [planOpen, setPlanOpen] = useState(true);
  const [lightbox, setLightbox] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const inner = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!state.busy) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [state.busy]);

  // Pegado al final mientras el usuario no suba; las capturas crecen al cargar, por eso se observa el tamaño.
  useEffect(() => {
    const el = scroller.current;
    if (state.ask) stick.current = true; // una confirmación siempre debe verse
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [state.items, state.ask, state.phase]);
  useEffect(() => {
    const el = scroller.current, content = inner.current;
    if (!el || !content || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => { if (stick.current) el.scrollTop = el.scrollHeight; });
    observer.observe(content);
    return () => observer.disconnect();
  }, []);

  const done = state.plan.filter((x) => x.done).length;
  const phaseClass = state.phase === 'waiting' ? 'waiting' : state.busy ? 'busy' : '';
  return (
    <aside className="agent" style={{ width: p.width }} aria-label="Agente">
      <div className="agent-head">
        <strong>Agente</strong>
        <span className={`state ${phaseClass}`}>{PHASE_LABEL[state.phase]}</span>
        <span className="agent-meta">
          {state.busy && state.startedAt ? `paso ${state.step} · ${formatElapsed(now - state.startedAt)}` : state.model ?? ''}
        </span>
        {state.busy && <button className="btn small danger" onClick={p.onStop}><Icon name="stop" size={12} />Detener</button>}
      </div>
      {state.context ? (
        <div className="agent-context">
          <Meter label="Contexto" value={state.used ?? 0} max={state.context}
            text={state.used === null ? `${formatTokens(state.context)} tokens` : `${formatTokens(state.used)} / ${formatTokens(state.context)} tokens`} />
        </div>
      ) : null}
      {state.plan.length > 0 && (
        <div style={{ padding: '10px 14px 0' }}>
          <PlanCard plan={state.plan} done={done} open={planOpen} onToggle={() => setPlanOpen(!planOpen)} />
        </div>
      )}
      <div className="transcript" ref={scroller} aria-live="polite"
        onScroll={(e) => { const el = e.currentTarget; stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60; }}>
        <div className="transcript-inner" ref={inner}>
        {!state.items.length && <Intro workspace={p.workspace} />}
        {state.items.map((item, i) => (
          <Item key={i} item={item} last={i === state.items.length - 1} busy={state.busy} workspace={p.workspace} onOpenUrl={p.onOpenUrl}
            onImage={setLightbox} onOpenFile={p.onOpenFile} />
        ))}
        {state.busy && state.phase !== 'waiting' && (
          <div className="thinking"><span className="pulse" />{PHASE_LABEL[state.phase]}
            {state.chars > 0 && <span className="dim">· {state.chars.toLocaleString('es')} caracteres</span>}</div>
        )}
        {state.ask?.context === 'pregunta' && <Question ask={state.ask} onAnswer={p.onAnswer} />}
        {state.ask && state.ask.context !== 'pregunta' && (
          <section className="card confirm" aria-label="Confirmación pendiente">
            <strong>¿Permitir este comando?</strong>
            {state.ask.question.includes(':\n') && <span className="dim">{state.ask.question.split(':\n')[0]}</span>}
            <code>{state.ask.question.replace(/^.*?:\n/s, '')}</code>
            <div className="field-row">
              {state.ask.options.map((o, i) => (
                <button key={o} className={`btn ${i === 0 ? 'primary' : ''}`} style={i === 0 ? { background: 'var(--warn)', borderColor: 'var(--warn)' } : undefined}
                  onClick={() => p.onAnswer(o)}>{o}</button>
              ))}
            </div>
          </section>
        )}
        {!state.busy && p.pending && (
          <section className="card resume" aria-label="Misión pendiente">
            <div className="card-head"><strong>Misión a medias</strong><span className="dim">{p.pending.role}</span></div>
            <div>{p.pending.objective}</div>
            <div className="field-row">
              <button className="btn primary" onClick={() => p.onQuick('continuar')}><Icon name="play" size={14} />Continuar</button>
              <button className="btn" onClick={p.onDismissPending}>Descartar</button>
            </div>
          </section>
        )}
        </div>
      </div>
      <Composer {...p} />
      {lightbox && <div className="lightbox" onClick={() => setLightbox(null)} role="dialog" aria-label="Captura"><img src={lightbox} alt="Captura de pantalla" /></div>}
    </aside>
  );
}

/** Pregunta del agente: botones para sus opciones y una respuesta libre. */
function Question({ ask, onAnswer }: { ask: { question: string; options: string[] }; onAnswer: (text: string) => void }) {
  const [text, setText] = useState('');
  const send = () => { if (text.trim()) onAnswer(text.trim()); };
  return (
    <section className="card question" aria-label="Pregunta del agente">
      <strong>El agente pregunta</strong>
      <div style={{ whiteSpace: 'pre-wrap' }}>{ask.question}</div>
      {ask.options.length > 0 && (
        <div className="field-row">
          {ask.options.map((o, i) => (
            <button key={o} className={`btn ${i === 0 ? 'primary' : ''}`} onClick={() => onAnswer(o)}>{o}</button>
          ))}
        </div>
      )}
      <div className="field-row">
        <input className="input" aria-label="Respuesta al agente" placeholder={ask.options.length ? 'U otra respuesta…' : 'Tu respuesta…'}
          value={text} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') send(); }} />
        <button className="btn" onClick={send} disabled={!text.trim()}>Responder</button>
      </div>
    </section>
  );
}

function Intro({ workspace }: { workspace: string | null }) {
  return (
    <div className="muted" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <strong style={{ color: 'var(--text)' }}>¿Qué construimos?</strong>
      <span>{workspace ? 'El agente planifica, escribe el código, lo prueba, mira la pantalla y corrige hasta que funciona.'
        : 'Abre una carpeta de proyecto para empezar.'}</span>
      <span className="dim">Ejemplos: «crea un juego de serpiente en pygame con puntuación», «añade tests a utils.py», «revisa por qué falla el login».</span>
    </div>
  );
}

function PlanCard({ plan, done, open, onToggle }: { plan: PlanItem[]; done: number; open: boolean; onToggle: () => void }) {
  const currentIndex = plan.findIndex((x) => !x.done);
  return (
    <section className="card" aria-label="Plan" style={{ maxHeight: open ? 230 : undefined, overflow: 'auto' }}>
      <button className="card-head" onClick={onToggle} style={{ background: 'transparent', border: 0, padding: 0 }} aria-expanded={open}>
        <strong>Plan</strong>
        <span className="muted" style={{ fontSize: 12, display: 'flex', alignItems: 'center', gap: 4 }}>{done} de {plan.length}
          <Icon name={open ? 'chevronDown' : 'chevronRight'} size={14} /></span>
      </button>
      {open && plan.map((x, i) => (
        <div key={i} className={`plan-item ${x.done ? 'done' : i === currentIndex ? 'current' : 'pending'}`}>
          {x.done ? <Icon name="check" /> : <span className="ring" />}<span>{x.text}</span>
        </div>
      ))}
    </section>
  );
}

function Item({ item, last, busy, workspace, onImage, onOpenFile, onOpenUrl }: {
  item: ChatItem; last: boolean; busy: boolean; workspace: string | null; onImage: (src: string) => void; onOpenFile: (p: string) => void;
  onOpenUrl: (url: string) => void;
}) {
  if (item.kind === 'user') return <div className="msg-user">{item.text}</div>;
  if (item.kind === 'agent') {
    // Un enlace no debe sacar la ventana del IDE: se abre en la vista previa (o se copia si no es local).
    const click = (e: React.MouseEvent) => {
      const link = (e.target as HTMLElement).closest('a');
      if (!link) return;
      e.preventDefault();
      onOpenUrl(link.href);
    };
    return <>
      <div className={`msg-agent ${item.status}`} onClick={click}
        dangerouslySetInnerHTML={{ __html: markdown(item.text || '(sin respuesta)') }} />
      {item.summary && <ResultCard summary={item.summary} workspace={workspace} onImage={onImage} onOpenFile={onOpenFile} />}
    </>;
  }
  return <Run entries={item.entries} live={last && busy} workspace={workspace} onImage={onImage} onOpenFile={onOpenFile} />;
}

/** Tarjeta final: verificación, revisión, última captura, lanzador y archivos cambiados (abre cada uno para Ver cambios). */
function ResultCard({ summary, workspace, onImage, onOpenFile }: {
  summary: Summary; workspace: string | null; onImage: (src: string) => void; onOpenFile: (p: string) => void;
}) {
  const [image, setImage] = useState<string | null>(null);
  useEffect(() => {
    if (summary.capture && workspace) void api.image(joinPath(workspace, summary.capture)).then(setImage).catch(() => setImage(null));
  }, [summary.capture, workspace]);
  const shown = summary.files.slice(0, 12);
  return (
    <section className="card result" aria-label="Resultado de la misión">
      <strong>Resultado</strong>
      {summary.verify && <div className={summary.verifyOk ? 'ok' : 'err'}>
        <Icon name="check" size={13} /> {summary.verify}</div>}
      {summary.review && <div className="dim">{summary.review}</div>}
      {image && <img className="capture" src={image} alt="Última captura de la pantalla" onClick={() => onImage(image)} />}
      {summary.launcher && <div>Para abrirlo: <code>{summary.launcher}</code></div>}
      {shown.length > 0 && <div className="result-files">
        <span className="dim">{summary.files.length === 1 ? '1 archivo cambiado' : `${summary.files.length} archivos cambiados`}</span>
        {shown.map((f) => <button key={f} className="btn small" title="Abrir (con Ver cambios)" onClick={() => onOpenFile(f)}>
          {workspace ? relativePath(workspace, f) : f}</button>)}
        {summary.files.length > shown.length && <span className="dim">y {summary.files.length - shown.length} más</span>}
      </div>}
    </section>
  );
}

const VISIBLE_STEPS = 6;

function Run({ entries, live, workspace, onImage, onOpenFile }: {
  entries: RunEntry[]; live: boolean; workspace: string | null; onImage: (src: string) => void; onOpenFile: (p: string) => void;
}) {
  const [all, setAll] = useState(false);
  const steps = entries.filter((e) => e.kind === 'step').length;
  const shown = useMemo(() => {
    if (all || entries.length <= VISIBLE_STEPS) return entries;
    return entries.slice(-VISIBLE_STEPS);
  }, [entries, all]);
  const hidden = entries.length - shown.length;
  return (
    <div className="run">
      {hidden > 0 && <button className="run-toggle" onClick={() => setAll(true)}>Mostrar {hidden} entradas anteriores ({steps} pasos en total)</button>}
      {all && entries.length > VISIBLE_STEPS && !live && <button className="run-toggle" onClick={() => setAll(false)}>Ocultar pasos anteriores</button>}
      {shown.map((e, i) => e.kind === 'note'
        ? <div key={i} className={`note ${e.note.level}`}>{e.note.text}</div>
        : <StepCard key={i} step={e.step} workspace={workspace} onImage={onImage} onOpenFile={onOpenFile} />)}
    </div>
  );
}

const FILE_TOOLS = new Set(['leer', 'escribir', 'editar']);

function StepCard({ step, workspace, onImage, onOpenFile }: {
  step: Step; workspace: string | null; onImage: (src: string) => void; onOpenFile: (p: string) => void;
}) {
  const [open, setOpen] = useState(step.tool === 'ver_pantalla' && step.error);
  const [image, setImage] = useState<string | null>(null);
  useEffect(() => {
    if (!open || !step.capture || !workspace || image) return;
    void api.image(joinPath(workspace, step.capture)).then(setImage).catch(() => setImage(null));
  }, [open, step.capture, workspace, image]);
  const isFile = FILE_TOOLS.has(step.tool) && step.detail && workspace;
  const detail = workspace ? relativePath(workspace, step.detail) || '.' : step.detail; // rutas cortas, como en el explorador
  return (
    <div className={`step${step.error ? ' error' : ''}`}>
      <button className="step-row" onClick={() => setOpen(!open)} aria-expanded={open}>
        <Icon name={TOOL_ICON[step.tool] ?? 'spark'} />
        <span className="grow ellipsis" title={`${step.detail}\n${step.result}`}>
          {toolLabel(step.tool)} {step.detail && <strong className={step.tool === 'ejecutar' ? 'mono' : ''} style={{ fontWeight: 500 }}>{detail}</strong>}
          {step.result && <span className={step.error ? 'err' : 'dim'}> · {step.result}</span>}
        </span>
        <span className="step-n">{step.n}</span>
      </button>
      {open && (
        <div className="step-detail">
          {isFile && <button className="btn small" style={{ alignSelf: 'flex-start' }} onClick={() => onOpenFile(joinPath(workspace!, step.detail))}>
            <Icon name="file" size={14} />Abrir {detail}</button>}
          {step.capture && (image ? <img className="capture" src={image} alt="Captura de la pantalla" onClick={() => onImage(image)} />
            : <div className="dim">Cargando captura…</div>)}
          {step.output && <pre>{step.output}</pre>}
        </div>
      )}
    </div>
  );
}

function Composer(p: Props) {
  const [text, setText] = useState('');
  const [mention, setMention] = useState<{ query: string; index: number } | null>(null);
  const area = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (!p.draft) return;
    setText((t) => `${t}${t && !t.endsWith(' ') ? ' ' : ''}${p.draft!.text}`);
    requestAnimationFrame(() => { const el = area.current; el?.focus(); el?.setSelectionRange(el.value.length, el.value.length); });
  }, [p.draft]);
  const matches = useMemo(() => {
    if (!mention) return [];
    const q = mention.query.toLowerCase();
    return p.files.filter((f) => f.toLowerCase().includes(q)).slice(0, 12);
  }, [mention, p.files]);

  useEffect(() => {
    const el = area.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(220, el.scrollHeight)}px`;
  }, [text]);

  const updateMention = (value: string, caret: number) => {
    const m = /(^|\s)@([^\s@]*)$/.exec(value.slice(0, caret));
    setMention(m ? { query: m[2], index: 0 } : null);
  };
  const pick = (file: string) => {
    const el = area.current;
    const caret = el?.selectionStart ?? text.length;
    const before = text.slice(0, caret).replace(/@([^\s@]*)$/, `@${file} `);
    setText(before + text.slice(caret));
    setMention(null);
    requestAnimationFrame(() => { el?.focus(); el?.setSelectionRange(before.length, before.length); });
  };
  const send = () => {
    const t = text.trim();
    if (!t || p.state.busy) return;
    p.onSend(t);
    setText('');
  };
  const attach = async () => {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open({ multiple: true, defaultPath: p.workspace ?? undefined });
    const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
    const root = (p.workspace ?? '').toLowerCase();
    const refs = list.map((f) => (root && f.toLowerCase().startsWith(root) ? `@${f.slice(root.length).replace(/^[\\/]+/, '')}` : `"${f}"`));
    if (refs.length) setText((t) => `${t}${t && !t.endsWith(' ') ? ' ' : ''}${refs.join(' ')} `);
  };

  const quick: { kind: QuickKind; label: string; title: string }[] = [
    { kind: 'verificar', label: 'Verificar', title: 'Ejecuta las comprobaciones del proyecto (tests, sintaxis, errores de la web)' },
    { kind: 'revisar', label: 'Revisar código', title: 'Revisión del código frente a lo que pediste' },
    { kind: 'continuar', label: 'Continuar', title: 'Sigue con la misión donde se quedó' },
    { kind: 'nuevo', label: 'Nueva conversación', title: 'Olvida el historial (los archivos y el plan se conservan)' },
  ];
  return (
    <div className="composer-zone">
      <div className="quick">
        {quick.map((q) => <button key={q.kind} className="btn chip" title={q.title} disabled={p.state.busy || !p.workspace}
          onClick={() => p.onQuick(q.kind)}>{q.label}</button>)}
      </div>
      <div className="composer">
        {mention && matches.length > 0 && (
          <div className="mentions" role="listbox" aria-label="Archivos">
            {matches.map((f, i) => <button key={f} role="option" aria-selected={i === mention.index}
              onMouseDown={(e) => { e.preventDefault(); pick(f); }}>{f}</button>)}
          </div>
        )}
        <label className="sr-only" htmlFor="composer">Instrucción para el agente</label>
        <textarea id="composer" ref={area} rows={2} value={text} disabled={!p.workspace}
          placeholder={p.workspace ? 'Describe la tarea. @ para mencionar un archivo' : 'Abre una carpeta para empezar'}
          onChange={(e) => { setText(e.target.value); updateMention(e.target.value, e.target.selectionStart); }}
          onKeyDown={(e) => {
            if (mention && matches.length) {
              if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                e.preventDefault();
                const d = e.key === 'ArrowDown' ? 1 : -1;
                setMention({ ...mention, index: (mention.index + d + matches.length) % matches.length });
                return;
              }
              if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pick(matches[mention.index]); return; }
              if (e.key === 'Escape') { setMention(null); return; }
            }
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
          }} />
        <div className="composer-bar">
          <button className="icon-btn bordered" aria-label="Adjuntar archivo" title="Adjuntar archivo" onClick={attach} disabled={!p.workspace}>
            <Icon name="attach" /></button>
          <label className="review-plan" title="El agente te enseña el plan y espera a que lo apruebes o pidas cambios antes de tocar nada">
            <input type="checkbox" checked={!!p.reviewPlan} onChange={(e) => p.onReviewPlan?.(e.target.checked)} />Revisar el plan</label>
          <span className="dim" style={{ fontSize: 12 }}>Enter envía · Shift+Enter salto de línea</span>
          {p.state.busy
            ? <button className="send stop" aria-label="Detener" title="Detener" onClick={p.onStop}><Icon name="stop" /></button>
            : <button className="send" aria-label="Enviar" title="Enviar" onClick={send} disabled={!text.trim() || !p.workspace}><Icon name="send" /></button>}
        </div>
      </div>
    </div>
  );
}
