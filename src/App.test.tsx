import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Tauri simulado: comandos con respuestas fijas y eventos que la prueba emite.
const listeners = new Map<string, (e: { payload: unknown }) => void>();
const calls: [string, Record<string, unknown> | undefined][] = [];
let missionDone: (value: string) => void = () => undefined;

const responses: Record<string, (args?: Record<string, unknown>) => unknown> = {
  get_workspace_tree: () => JSON.stringify([
    { name: 'index.html', path: 'C:\\ws\\index.html', parent_path: 'C:\\ws', is_dir: false },
    { name: 'js', path: 'C:\\ws\\js', parent_path: 'C:\\ws', is_dir: true },
    { name: 'app.js', path: 'C:\\ws\\js\\app.js', parent_path: 'C:\\ws\\js', is_dir: false },
  ]),
  load_chat_history: () => JSON.stringify([{ sender: 'agent', text: 'Hola **anterior**', timestamp: '' }]),
  get_models: () => ['qwen3.6-35b-a3b', 'qwen3.5-9b'],
  get_agent_rules: () => ({ reglas: '- usa pytest', lecciones: '' }),
  get_system_stats: () => ({ cpu: 12, ramUsed: 21.4, ramTotal: 31.8, gpu: { used: 7183, total: 8188, util: 30 } }),
  get_git_info: () => ({ branch: 'main', changes: [], log: [] }),
  list_checkpoints: () => [{ numero: 1, paso: 4, fecha: '2026-10-16 10:00', archivos: ['index.html'] }],
  get_pending_mission: () => null,
  get_mission_baseline: () => null,
  save_chat_message: () => null,
  process_user_prompt: () => new Promise((resolve) => { missionDone = resolve; }),
  agent_order: (args) => ({ tipo: 'fin', orden: (args?.order as { tipo: string }).tipo, estado: 'ok',
    respuesta: 'VERIFICACIÓN (web): 0 ERRORES, 0 AVISOS', uso: '', datos: [] }),
  submit_user_answer: () => null,
  run_command: (args) => ({ code: 0, output: `salida de ${args?.command}\n`, ms: 30 }),
};
let respuestaAsk = false;
vi.mock('@tauri-apps/plugin-dialog', () => ({  // la confirmación nativa de Windows
  ask: vi.fn(async (texto: string) => { calls.push(['plugin:dialog|ask', { texto }]); return respuestaAsk; }),
  open: vi.fn(async () => null),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push([cmd, args]);
    return responses[cmd]?.(args) ?? null;
  }),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return () => listeners.delete(name);
  }),
}));
vi.mock('./components/EditorArea', async () => {
  const { forwardRef } = await import('react');
  return {
    EditorArea: forwardRef(function FakeEditor(p: { tabs: string[]; welcome: unknown }, _ref) {
      return <div data-testid="editor">{p.tabs.length ? p.tabs.join(',') : (p.welcome as never)}</div>;
    }),
  };
});

import App from './App';

const emit = (name: string, payload: unknown) => act(() => listeners.get(name)?.({ payload }));
const called = (cmd: string) => calls.filter(([c]) => c === cmd).map(([, a]) => a);

beforeEach(() => {
  calls.length = 0;
  localStorage.clear();
  localStorage.setItem('aura_last_workspace', JSON.stringify('C:\\ws'));
});
afterEach(cleanup);

describe('Aura Sentinel', () => {
  it('abre la última carpeta con su árbol, historial y modelos', async () => {
    render(<App />);
    expect(await screen.findByText('index.html')).toBeTruthy();
    expect(screen.getByRole('button', { name: /Cambiar espacio de trabajo/ }).textContent).toContain('ws');
    expect(await screen.findByText('anterior')).toBeTruthy(); // historial en markdown
    await waitFor(() => expect((screen.getByLabelText('Modelo activo').parentElement!.querySelector('select') as HTMLSelectElement).value)
      .toBe('qwen3.6-35b-a3b'));
    expect(screen.getByText(/7,0 \/ 8,0 GB/)).toBeTruthy();
    fireEvent.click(screen.getByText('js'));
    fireEvent.click(await screen.findByText('app.js'));
    expect(screen.getByTestId('editor').textContent).toBe('C:\\ws\\js\\app.js');
  });

  it('envía una misión y muestra plan, pasos, confirmación y respuesta', async () => {
    render(<App />);
    await screen.findByText('index.html');
    await waitFor(() => expect(listeners.has('agent-event')).toBe(true));
    const box = screen.getByLabelText('Instrucción para el agente');
    fireEvent.change(box, { target: { value: 'mejora @index.html ', selectionStart: 19 } });
    fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(called('process_user_prompt')).toHaveLength(1));
    expect(called('process_user_prompt')[0]).toMatchObject({
      userMessage: 'mejora @index.html\n\n(Archivos que menciono: index.html)', workspacePath: 'C:\\ws', model: 'qwen3.6-35b-a3b',
    });
    emit('agent-event', { tipo: 'paso', paso: 1, herramienta: 'plan', detalle: '', resultado: 'OK', error: false, salida: '',
      plan: '- [x] 1. Estructura\n- [ ] 2. Verificar en el navegador' });
    emit('agent-event', { tipo: 'paso', paso: 2, herramienta: 'ver_pantalla', detalle: 'index.html', error: true,
      resultado: 'VISUAL: PROBLEMAS en index.html', salida: 'VISUAL: PROBLEMAS\n- texto tapado por el canvas' });
    expect(screen.getByText('1 de 2')).toBeTruthy();
    expect(screen.getByText('Verificar en el navegador')).toBeTruthy();
    expect(screen.getAllByText('texto tapado por el canvas').length).toBeGreaterThan(0); // tarjeta y panel de problemas
    emit('agent-ask-user', { id: 'q1', question: 'El agente quiere ejecutar:\nrmdir /s /q dist', options: ['Permitir', 'Bloquear'], context: '' });
    fireEvent.click(screen.getByRole('button', { name: 'Permitir' }));
    expect(called('submit_user_answer')[0]).toEqual({ id: 'q1', answer: 'Permitir' });
    await act(async () => missionDone(JSON.stringify({ status: 'FINISH', respuesta_conversacional: 'Hecho: **catálogo** listo' })));
    expect(await screen.findByText('catálogo')).toBeTruthy();
    expect(called('save_chat_message').map((a) => (a?.message as { sender: string }).sender)).toEqual(['user', 'agent']);
  });

  it('las acciones rápidas usan las órdenes del agente', async () => {
    render(<App />);
    await screen.findByText('index.html');
    fireEvent.click(screen.getByRole('button', { name: 'Verificar' }));
    await waitFor(() => expect(called('agent_order').some((a) => (a?.order as { tipo: string }).tipo === 'verificar')).toBe(true));
    expect(await screen.findByText(/0 ERRORES/)).toBeTruthy();
  });

  it('la paleta abre archivos y ejecuta comandos', async () => {
    render(<App />);
    await screen.findByText('index.html');
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    const input = screen.getByLabelText('Buscar archivo o comando');
    fireEvent.change(input, { target: { value: 'app' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(screen.getByTestId('editor').textContent).toBe('C:\\ws\\js\\app.js');
  });

  it('nueva conversación pregunta antes y no borra nada si el usuario cancela', async () => {
    render(<App />);
    await screen.findByText('anterior');
    respuestaAsk = false;
    fireEvent.click(screen.getByRole('button', { name: 'Nueva conversación' }));
    await waitFor(() => expect(called('plugin:dialog|ask')).toHaveLength(1));
    expect(called('clear_chat_history')).toHaveLength(0);
    expect(screen.getByText('anterior')).toBeTruthy();
    respuestaAsk = true;
    fireEvent.click(screen.getByRole('button', { name: 'Nueva conversación' }));
    await waitFor(() => expect(called('clear_chat_history')).toHaveLength(1));
    await waitFor(() => expect(screen.queryByText('anterior')).toBeNull());
  });

  it('la terminal no tumba la interfaz aunque scrollIntoView devuelva una Promise (WebView2 reciente)', async () => {
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = vi.fn(() => Promise.resolve()) as never;
    try {
      render(<App />);
      await screen.findByText('index.html');
      fireEvent.click(screen.getByRole('tab', { name: 'Terminal' }));
      for (const comando of ['python saludo.py', 'git status']) {
        const caja = screen.getByLabelText('Comando');
        fireEvent.change(caja, { target: { value: comando } });
        fireEvent.submit(caja.closest('form')!);
        expect(await screen.findByText(new RegExp(`salida de ${comando}`))).toBeTruthy();
      }
      expect(screen.getAllByText(/código 0/)).toHaveLength(2);
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });
});
