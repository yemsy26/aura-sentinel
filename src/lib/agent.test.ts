import { describe, expect, it } from 'vitest';
import { agentReducer, captureOf, initialAgent, parsePlan, parseProblems, parseUsage, type AgentState } from './agent';
import type { AgentEvent } from './api';
import { diffCounts, joinPath, relativePath, serverUrl } from './text';

const ev = (state: AgentState, event: AgentEvent) => agentReducer(state, { type: 'event', event });

describe('protocolo de Agente Local', () => {
  it('lee el plan con pasos hechos y numeración', () => {
    expect(parsePlan('# Plan\n- [x] 1. Crear index.html — hecho\n- [ ] 2. Probar')).toEqual([
      { text: 'Crear index.html', done: true }, { text: 'Probar', done: false },
    ]);
  });

  it('lee el uso del contexto', () => {
    expect(parseUsage('24123/65536 tokens (36%)')).toEqual([24123, 65536]);
    expect(parseUsage('')).toBeNull();
  });

  it('separa errores y avisos de verificar y los de ver_pantalla', () => {
    const report = 'VERIFICACIÓN (web): 1 ERRORES, 1 AVISOS\nERRORES (corrígelos todos):\n- three está DESACTUALIZADA\n' +
      'AVISOS (revisa cada uno):\n- sin favicon\nOK:\n- index.html carga';
    expect(parseProblems(report, 'verificar')).toEqual([
      { severity: 'error', text: 'three está DESACTUALIZADA', source: 'verificar' },
      { severity: 'warning', text: 'sin favicon', source: 'verificar' },
    ]);
    const visual = 'VISUAL: PROBLEMAS en index.html (revisado por qwen)\nCaptura: .agente/capturas/pantalla_index.png\n- texto tapado';
    expect(parseProblems(visual, 'ver_pantalla')).toHaveLength(1);
    expect(captureOf(visual)).toBe('.agente/capturas/pantalla_index.png');
  });

  it('una misión agrupa sus pasos y notas y termina con la respuesta', () => {
    let s = agentReducer(initialAgent, { type: 'send', text: 'crea un juego', now: 1000 });
    expect(s.busy).toBe(true);
    s = ev(s, { tipo: 'listo', version: '1', modelo: 'qwen', contexto: 65536, servidor: '', proyecto: '', entorno: '' });
    s = ev(s, { tipo: 'progreso', fase: 'escribiendo', caracteres: 120 });
    expect(s.phase).toBe('writing');
    s = ev(s, { tipo: 'paso', paso: 1, herramienta: 'plan', detalle: '', resultado: 'OK', error: false, salida: '', plan: '- [ ] 1. Crear juego.py' });
    s = ev(s, { tipo: 'aviso', texto: 'cuidado' });
    s = ev(s, { tipo: 'paso', paso: 2, herramienta: 'verificar', detalle: '', resultado: 'VERIFICACIÓN: 1 ERRORES', error: true,
      salida: 'ERRORES (x):\n- falla test_a' });
    expect(s.phase).toBe('verifying');
    expect(s.plan).toEqual([{ text: 'Crear juego.py', done: false }]);
    expect(s.problems.verificar[0].text).toBe('falla test_a');
    expect(s.items).toHaveLength(2);
    const run = s.items[1];
    expect(run.kind === 'run' && run.entries.map((e) => e.kind)).toEqual(['step', 'note', 'step']);
    s = ev(s, { tipo: 'fin', orden: 'tarea', estado: 'ok', respuesta: 'Listo', uso: '30000/65536 tokens (45%)' });
    expect(s.used).toBe(30000);
    s = agentReducer(s, { type: 'done', text: 'Listo', status: 'FINISH' });
    expect(s.busy).toBe(false);
    expect(s.items.at(-1)).toMatchObject({ kind: 'agent', text: 'Listo' });
  });

  it('los archivos cambiados no se repiten y una misión nueva los reinicia', () => {
    let s = agentReducer(initialAgent, { type: 'changed', path: 'C:\\ws\\a.py' });
    s = agentReducer(s, { type: 'changed', path: 'c:\\WS\\A.py' });
    expect(s.changed).toEqual(['C:\\ws\\a.py']);
    s = agentReducer(s, { type: 'send', text: 'otra', now: 0 });
    expect(s.changed).toEqual([]);
  });

  it('cierra la confirmación solo si es la misma pregunta', () => {
    let s = agentReducer(initialAgent, { type: 'ask', ask: { id: 'a', question: 'q', options: ['Permitir'], context: '' } });
    expect(s.phase).toBe('waiting');
    expect(agentReducer(s, { type: 'askClosed', id: 'b' }).ask).not.toBeNull();
    s = agentReducer(s, { type: 'askClosed', id: 'a' });
    expect(s.ask).toBeNull();
  });
});

describe('utilidades de texto', () => {
  it('cuenta líneas añadidas y quitadas', () => {
    expect(diffCounts('a\nb\nc', 'a\nc\nd\ne')).toEqual({ added: 2, removed: 1 });
  });
  it('rutas del proyecto', () => {
    expect(joinPath('C:\\ws\\', 'src/a.js')).toBe('C:\\ws\\src\\a.js');
    expect(joinPath('C:\\ws', 'D:\\otro\\b.js')).toBe('D:\\otro\\b.js');
    expect(relativePath('C:\\WS', 'C:\\ws\\src\\a.js')).toBe('src\\a.js');
  });
  it('detecta el servidor de un proceso', () => {
    expect(serverUrl('python -m http.server 8000')).toBe('http://localhost:8000');
    expect(serverUrl('npx vite --port 5173')).toBe('http://localhost:5173');
    expect(serverUrl('python juego.py')).toBeNull();
  });
});
