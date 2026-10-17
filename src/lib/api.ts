// Comandos y eventos del backend de Tauri (src-tauri/src), con sus tipos.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface FileNode { name: string; path: string; parent_path: string | null; is_dir: boolean }
export interface ChatMessage { sender: 'user' | 'agent' | 'system'; text: string; timestamp: string }
export interface MissionResult { status: 'FINISH' | 'ERROR' | 'CANCELLED'; respuesta_conversacional: string }
export interface PendingMission { objective: string; workspace: string; step: number; role: string }
export interface Checkpoint { numero: number; paso: number; fecha: string; archivos: string[] }
export interface GitInfo { branch: string | null; root: string | null; changes: { status: string; path: string }[]; log: string[] }
export interface SystemStats {
  cpu: number; ramUsed: number; ramTotal: number;
  gpu: { used: number; total: number; util: number } | null;
}
export interface SearchHit { path: string; line: number; text: string }
export interface CommandOutput { code: number; output: string; ms: number }
export interface AgentProcess { id: number; vivo: boolean; comando: string }
export interface AgentSettings { carpeta: string; instalado: boolean; motor: boolean }
export interface Addon {
  id: string; nombre: string; repo: string; aporta: string; herramientas: string[];
  usa: { nombre: string; repo: string; papel: string }[]; activo: boolean; origen: string;
}
export interface UserProfile {
  displayName?: string | null; city?: string | null; homeCountry?: string | null; preferredLanguage?: string | null;
  timeZone?: string | null; responseStyle?: string | null; autoRememberFacts: boolean;
}
export interface ProfileSnapshot { profile: UserProfile; windowsRegionSuggestion: string | null; projectCountry: string | null }
export interface AskEvent { id: string; question: string; options: string[]; context: string }

/** Eventos de Agente Local tal cual los emite su protocolo --json (evento `agent-event`). */
export type AgentEvent =
  | { tipo: 'listo'; version: string; modelo: string; contexto: number; servidor: string; proyecto: string; entorno: string }
  | { tipo: 'paso'; paso: number; herramienta: string; detalle: string; resultado: string; error: boolean; salida: string; plan?: string }
  | { tipo: 'archivos'; rutas: string[] }
  | { tipo: 'nota' | 'aviso' | 'error'; texto: string }
  | { tipo: 'progreso'; fase: 'pensando' | 'escribiendo'; caracteres: number }
  | { tipo: 'confirmar'; comando: string; motivo?: string }
  | { tipo: 'pregunta'; pregunta: string; opciones: string[] }
  | { tipo: 'fin'; orden: string; estado: 'ok' | 'error' | 'incompleta'; respuesta: string; uso: string; datos?: unknown };

export type QuickOrder =
  | { tipo: 'verificar' | 'plan' | 'procesos' | 'nuevo' }
  | { tipo: 'revisar' | 'regla'; texto: string }
  | { tipo: 'detener'; id: number }
  | { tipo: 'restaurar'; numero: number };

type FinEvent = Extract<AgentEvent, { tipo: 'fin' }>;

export const api = {
  currentDirectory: () => invoke<string>('get_current_directory'),
  tree: async (path: string) => JSON.parse(await invoke<string>('get_workspace_tree', { path })) as FileNode[],
  readFile: (path: string) => invoke<string>('read_file_content', { path }),
  saveFile: (path: string, content: string) => invoke<void>('save_file_content', { path, content }),
  chatHistory: async (workspacePath: string) =>
    JSON.parse(await invoke<string>('load_chat_history', { workspacePath })) as ChatMessage[],
  saveChat: (workspacePath: string, message: ChatMessage) => invoke<void>('save_chat_message', { workspacePath, message }),
  clearChat: (workspacePath: string) => invoke<void>('clear_chat_history', { workspacePath }),

  mission: async (userMessage: string, workspacePath: string, model: string | null) =>
    JSON.parse(await invoke<string>('process_user_prompt', { userMessage, workspacePath, model })) as MissionResult,
  order: (order: QuickOrder, workspacePath: string) => invoke<FinEvent>('agent_order', { order, workspacePath }),
  cancel: () => invoke<boolean>('cancel_agent_mission'),
  answer: (id: string, answer: string) => invoke<void>('submit_user_answer', { id, answer }),
  models: () => invoke<string[]>('get_models'),
  agentSettings: () => invoke<AgentSettings>('get_agent_settings'),
  saveAgentSettings: (carpeta: string) => invoke<AgentSettings>('save_agent_settings', { carpeta }),
  installEngine: () => invoke<string>('install_engine'),
  addons: () => invoke<Addon[]>('get_addons'),
  activateAddon: (id: string) => invoke<void>('activate_addon', { id }),
  deactivateAddon: (id: string) => invoke<Addon[]>('deactivate_addon', { id }),
  openAddonRepo: (id: string) => invoke<void>('open_addon_repo', { id }),
  rules: () => invoke<{ reglas: string; lecciones: string }>('get_agent_rules'),
  pendingMission: (workspacePath: string) => invoke<PendingMission | null>('get_pending_mission', { workspacePath }),
  dismissPending: (workspacePath: string) => invoke<void>('dismiss_pending_mission', { workspacePath }),

  stats: () => invoke<SystemStats>('get_system_stats'),
  git: (workspacePath: string) => invoke<GitInfo>('get_git_info', { workspacePath }),
  checkpoints: (workspacePath: string) => invoke<Checkpoint[]>('list_checkpoints', { workspacePath }),
  image: (path: string) => invoke<string>('read_image_data_url', { path }),
  openExternal: (url: string) => invoke<void>('open_external', { url }),
  search: (workspacePath: string, query: string) => invoke<SearchHit[]>('search_workspace', { workspacePath, query }),
  run: (workspacePath: string, command: string) => invoke<CommandOutput>('run_command', { workspacePath, command }),
  baseline: (path: string) => invoke<string | null>('get_mission_baseline', { path }),

  profile: (workspacePath: string | null) => invoke<ProfileSnapshot>('get_user_profile', { workspacePath }),
  saveProfile: (profile: UserProfile) => invoke<UserProfile>('save_user_profile', { profile }),
  clearProfile: () => invoke<void>('clear_user_profile'),
};

export function onAgentEvent(handler: (event: AgentEvent) => void): Promise<UnlistenFn> {
  return listen<AgentEvent>('agent-event', (e) => handler(e.payload));
}
export function onFileUpdated(handler: (path: string) => void): Promise<UnlistenFn> {
  return listen<{ path: string }>('file-updated', (e) => handler(e.payload.path));
}
export function onAsk(handler: (ask: AskEvent) => void): Promise<UnlistenFn> {
  return listen<AskEvent>('agent-ask-user', (e) => handler(e.payload));
}
export function onAskClosed(handler: (id: string) => void): Promise<UnlistenFn> {
  return listen<{ id: string }>('agent-ask-user-closed', (e) => handler(e.payload.id));
}

export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
