// Estado del panel del agente a partir de los eventos de Agente Local. Funciones puras: se prueban sin Tauri.
import type { AgentEvent, AskEvent, ChatMessage } from './api';

export type Phase = 'idle' | 'starting' | 'thinking' | 'writing' | 'acting' | 'verifying' | 'waiting';

export interface Step {
  n: number; tool: string; detail: string; result: string; output: string; error: boolean; capture: string | null;
}
export interface Note { level: 'nota' | 'aviso' | 'error'; text: string }
export type RunEntry = { kind: 'step'; step: Step } | { kind: 'note'; note: Note };
/** Resultado de una misión para la tarjeta final: lo que ya llegó en sus pasos (sin pedir nada al modelo). */
export interface Summary {
  files: string[]; verify: string | null; verifyOk: boolean; review: string | null; capture: string | null; launcher: string | null;
}
export type ChatItem =
  | { kind: 'user'; text: string; time: string }
  | { kind: 'agent'; text: string; time: string; status: 'FINISH' | 'ERROR' | 'CANCELLED'; summary?: Summary }
  | { kind: 'run'; id: number; entries: RunEntry[] };
export interface PlanItem { text: string; done: boolean }
export interface Problem { severity: 'error' | 'warning'; text: string; source: string }

export interface AgentState {
  items: ChatItem[];
  plan: PlanItem[];
  phase: Phase;
  busy: boolean;
  step: number;
  chars: number;
  startedAt: number | null;
  model: string | null;
  context: number | null;
  used: number | null;
  ask: AskEvent | null;
  problems: Record<string, Problem[]>;
  changed: string[];
}

export const initialAgent: AgentState = {
  items: [], plan: [], phase: 'idle', busy: false, step: 0, chars: 0, startedAt: null, model: null, context: null,
  used: null, ask: null, problems: {}, changed: [],
};

export type AgentAction =
  | { type: 'event'; event: AgentEvent }
  | { type: 'send'; text: string; now: number }
  | { type: 'done'; text: string; status: 'FINISH' | 'ERROR' | 'CANCELLED' }
  | { type: 'ask'; ask: AskEvent | null }
  | { type: 'askClosed'; id: string }
  | { type: 'changed'; path: string }
  | { type: 'history'; messages: ChatMessage[] }
  | { type: 'reset' };

const VERIFY = new Set(['verificar', 'revisar', 'ver_pantalla']);

export function parsePlan(text: string): PlanItem[] {
  return text.split('\n').filter((l) => l.startsWith('- [')).map((line) => {
    let rest = line.slice(5).trim().replace(/^\d+\.\s+/, '');
    rest = rest.split(' — ')[0];
    return { text: rest, done: line.startsWith('- [x]') };
  });
}

/** "24123/65536 tokens (36%)" → [24123, 65536] */
export function parseUsage(uso: string): [number, number] | null {
  const m = /(\d+)\s*\/\s*(\d+)/.exec(uso);
  return m ? [Number(m[1]), Number(m[2])] : null;
}

/** Líneas "- ..." de un informe de verificar o ver_pantalla, con su gravedad según la sección. */
export function parseProblems(report: string, source: string): Problem[] {
  const out: Problem[] = [];
  let severity: Problem['severity'] | null = source === 'ver_pantalla' ? 'warning' : null;
  for (const raw of report.split('\n')) {
    const line = raw.trim();
    if (/^ERRORES\b/.test(line)) severity = 'error';
    else if (/^AVISOS\b/.test(line)) severity = 'warning';
    else if (/^OK:?$/.test(line)) severity = null;
    else if (line.startsWith('- ') && severity) out.push({ severity, text: line.slice(2), source });
  }
  return out;
}

export function captureOf(output: string): string | null {
  return /Captura:\s*(\S+\.png)/.exec(output)?.[1] ?? null;
}

/** Resumen de la última misión: su verificación y revisión, la última captura, el lanzador y los archivos cambiados. */
export function summarize(items: ChatItem[], changed: string[]): Summary | undefined {
  const userAt = items.map((i) => i.kind).lastIndexOf('user');
  const run = items.slice(userAt + 1).find((i) => i.kind === 'run');
  const steps = run?.kind === 'run' ? run.entries.flatMap((e) => (e.kind === 'step' ? [e.step] : [])) : [];
  if (!steps.length) return undefined;
  const last = (tool: string) => [...steps].reverse().find((s) => s.tool === tool);
  const verify = last('verificar');
  const veredicto = /VEREDICTO:\s*(APROBADO|CAMBIOS NECESARIOS)/i.exec(last('revisar')?.output ?? '')?.[1].toUpperCase();
  const bats = steps.filter((s) => s.tool === 'crear_bat' && !s.error && s.detail).map((s) => s.detail.replace(/(\.bat)?$/i, '.bat'));
  return {
    files: changed, verify: verify?.result ?? null, verifyOk: !!verify && !verify.error,
    review: veredicto ? (veredicto === 'APROBADO' ? 'Revisión independiente: aprobada' : 'Revisión independiente: pidió cambios') : null,
    capture: [...steps].reverse().find((s) => s.capture)?.capture ?? null,
    launcher: [...bats].reverse().find((b) => !/^test/i.test(b)) ?? bats.at(-1) ?? null, // el que abre la app, no el de tests
  };
}

function withRun(items: ChatItem[], entry: RunEntry): ChatItem[] {
  const last = items[items.length - 1];
  if (last?.kind === 'run') return [...items.slice(0, -1), { ...last, entries: [...last.entries, entry] }];
  return [...items, { kind: 'run', id: items.length, entries: [entry] }];
}

function onEvent(state: AgentState, event: AgentEvent): AgentState {
  switch (event.tipo) {
    case 'listo':
      return { ...state, model: event.modelo, context: event.contexto, phase: state.busy ? 'thinking' : state.phase };
    case 'progreso':
      return { ...state, phase: event.fase === 'escribiendo' ? 'writing' : 'thinking', chars: event.caracteres };
    case 'paso': {
      const step: Step = {
        n: event.paso, tool: event.herramienta, detail: event.detalle, result: event.resultado, output: event.salida,
        error: event.error, capture: event.herramienta === 'ver_pantalla' ? captureOf(event.salida) : null,
      };
      const next = {
        ...state, items: withRun(state.items, { kind: 'step', step }), step: event.paso, chars: 0,
        phase: (VERIFY.has(event.herramienta) ? 'verifying' : 'acting') as Phase,
      };
      if (event.plan !== undefined) next.plan = parsePlan(event.plan);
      if (event.herramienta === 'verificar' || event.herramienta === 'ver_pantalla') {
        next.problems = { ...state.problems, [event.herramienta]: parseProblems(event.salida, event.herramienta) };
      }
      return next;
    }
    case 'nota': case 'aviso': case 'error':
      return { ...state, items: withRun(state.items, { kind: 'note', note: { level: event.tipo, text: event.texto } }) };
    case 'archivos': // las rutas absolutas llegan con file-updated (acción 'changed')
      return state;
    case 'confirmar': case 'pregunta':
      return { ...state, phase: 'waiting' };
    case 'fin': {
      const usage = parseUsage(event.uso);
      const next = { ...state, used: usage?.[0] ?? state.used, context: usage?.[1] ?? state.context };
      if (event.orden === 'verificar') next.problems = { ...state.problems, verificar: parseProblems(event.respuesta, 'verificar') };
      return next;
    }
  }
}

export function agentReducer(state: AgentState, action: AgentAction): AgentState {
  switch (action.type) {
    case 'event':
      return onEvent(state, action.event);
    case 'send':
      return {
        ...state, busy: true, phase: 'starting', step: 0, chars: 0, startedAt: action.now, changed: [],
        items: [...state.items, { kind: 'user', text: action.text, time: new Date(action.now).toISOString() }],
      };
    case 'done': {
      const summary = action.status === 'FINISH' ? summarize(state.items, state.changed) : undefined;
      return {
        ...state, busy: false, phase: 'idle', ask: null,
        items: [...state.items, { kind: 'agent', text: action.text, status: action.status, time: new Date().toISOString(), summary }],
      };
    }
    case 'askClosed':
      return state.ask?.id === action.id ? { ...state, ask: null, phase: state.busy ? 'acting' : 'idle' } : state;
    case 'changed':
      return state.changed.some((c) => c.toLowerCase() === action.path.toLowerCase())
        ? state : { ...state, changed: [...state.changed, action.path] };
    case 'ask':
      return { ...state, ask: action.ask, phase: action.ask ? 'waiting' : state.busy ? 'acting' : 'idle' };
    case 'history':
      return {
        ...initialAgent, model: state.model, context: state.context,
        items: action.messages.map((m) => m.sender === 'user'
          ? { kind: 'user' as const, text: m.text, time: m.timestamp }
          : { kind: 'agent' as const, text: m.text, time: m.timestamp, status: 'FINISH' as const }),
      };
    case 'reset':
      return { ...initialAgent, model: state.model, context: state.context };
  }
}

export const PHASE_LABEL: Record<Phase, string> = {
  idle: 'Listo', starting: 'Iniciando', thinking: 'Pensando', writing: 'Escribiendo', acting: 'Trabajando',
  verifying: 'Verificando', waiting: 'Esperando tu respuesta',
};

const TOOL_LABEL: Record<string, string> = {
  leer: 'Leyó', escribir: 'Escribió', editar: 'Editó', ejecutar: 'Ejecutó', verificar: 'Verificó', revisar: 'Revisó',
  ver_pantalla: 'Vio la pantalla', plan: 'Planificó', paso_hecho: 'Completó un paso', buscar: 'Buscó', listar: 'Listó',
  web_leer: 'Leyó la web', web_buscar: 'Buscó en la web', git: 'Git', restaurar: 'Restauró', crear_bat: 'Creó lanzador',
  procesos: 'Procesos', calcular: 'Calculó', resolver_sat: 'Resolvió SAT', experimentar: 'Experimentó',
  abrir_android_studio: 'Abrió Android Studio', terminar: 'Terminó',
};

export function toolLabel(tool: string): string {
  return TOOL_LABEL[tool] ?? tool;
}
