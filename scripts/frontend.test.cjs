const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { JSDOM } = require('jsdom');
const root = require('node:path').resolve(__dirname, '..');
const read = p => fs.readFileSync(require('node:path').join(root, p), 'utf8');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

test('frontend sanitizes messages, queues workspaces and releases listeners on failure', async () => {
  const dom = new JSDOM(read('src/index.html'), { url: 'http://localhost/', runScripts: 'outside-only', pretendToBeVisual: true });
  const w = dom.window;
  const handlers = new Map();
  const missions = [];
  let active = 0, maximum = 0;
  w.__TAURI__ = {
    dialog: { open: async () => null },
    event: { listen: async (name, fn) => {
      if (!handlers.has(name)) handlers.set(name, new Set());
      handlers.get(name).add(fn);
      return () => handlers.get(name).delete(fn);
    } },
    core: { invoke: async (name, args) => {
      if (name === 'get_current_directory') return '/initial';
      if (name === 'get_workspace_tree' || name === 'load_chat_history') return '[]';
      if (name === 'get_pending_mission') return { objective: 'Resume', workspace: '/resumed', step: 2 };
      if (name === 'get_ollama_models' || name === 'get_background_tasks') return [];
      if (name === 'process_user_prompt') {
        missions.push(args); maximum = Math.max(maximum, ++active);
        if (missions.length === 2) {
          for (const fn of handlers.get('agent-step') || []) {
            fn({ payload: { step: 0, status: 'SUCCESS', message: 'Plan generado: Fase 1: Implementar y verificar' } });
            fn({ payload: { step: 1, status: 'WARNING', message: '[SEMANTIC_VERIFICATION] Verificador falló.' } });
            fn({ payload: { step: 2, status: 'SUCCESS', message: '[TERMINAL] {"percentage":100.0}' } });
          }
        }
        await pause(20); --active;
        if (missions.length === 1) throw new Error('<img src=x onerror=alert(1)>');
        return JSON.stringify({ status: 'FINISH', respuesta_conversacional: 'SpectraSAT Ran 3 tests <img src=x onerror=alert(1)> <a href="javascript:alert(1)">link</a>' });
      }
      return '';
    } }
  };
  try {
    w.eval(read('src/marked.min.js'));
    w.eval(read('src/vendor/purify.min.js'));
    w.eval(read('src/main.js'));
    await pause(80);
    assert.equal(w.document.getElementById('mission-resume-banner').style.display, 'block');
    w.document.getElementById('resume-mission-btn').click();
    for (const fn of handlers.get('scheduled-task-fire')) {
      fn({ payload: { objective: 'Scheduled', workspace: '/scheduled' } });
    }
    await pause(150);
    assert.deepEqual(missions.map(m => m.workspacePath), ['/resumed', '/scheduled']);
    assert.equal(maximum, 1);
    assert.equal(handlers.get('agent-step').size, 0);
    assert.equal(w.document.querySelectorAll('[onerror], a[href^="javascript:"]').length, 0);
    assert.equal(w.document.querySelectorAll('.sat-cert-card, .test-suite-card').length, 0);
    assert.ok(w.document.querySelector('.message-body').textContent);
    assert.equal(w.document.getElementById('mission-stage').textContent, 'COMPLETADO');
    assert.equal(w.document.getElementById('mission-verification').textContent, '100% APROBADA');
    assert.equal(w.document.getElementById('mission-repairs').textContent, '1');
    assert.equal(w.document.getElementById('phase-progress-percentage').textContent, '100%');
    for (const fn of handlers.get('agent-ask-user')) fn({ payload: { id: 'review', question: 'Review', options: ['Approve'] } });
    assert.equal(w.document.getElementById('ask-user-modal').style.display, 'flex');
    for (const fn of handlers.get('agent-ask-user-closed')) fn({ payload: { id: 'review' } });
    assert.equal(w.document.getElementById('ask-user-modal').style.display, 'none');

  } finally { w.close(); }
});

test('every startup script is bundled locally', () => {
  const dom = new JSDOM(read('src/index.html'));
  for (const script of dom.window.document.querySelectorAll('script[src]')) {
    const src = script.getAttribute('src');
    assert.ok(!src.startsWith('http'));
    assert.ok(fs.existsSync(require('node:path').join(root, 'src', src)), src);
  }
  dom.window.close();
});
