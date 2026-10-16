import DOMPurify from 'dompurify';
import { marked } from 'marked';

export function markdown(text: string): string {
  return DOMPurify.sanitize(marked.parse(text, { async: false, gfm: true, breaks: true }));
}

const ABSOLUTE = /^([A-Za-z]:[\\/]|\\\\)/;

/** Ruta absoluta con separadores de Windows, como las que devuelve el árbol del backend. */
export function joinPath(root: string, path: string): string {
  const full = ABSOLUTE.test(path) ? path : `${root.replace(/[\\/]+$/, '')}\\${path}`;
  return full.replace(/\//g, '\\');
}

export function relativePath(root: string, path: string): string {
  const base = root.replace(/[\\/]+$/, '').toLowerCase();
  return path.toLowerCase().startsWith(base) ? path.slice(base.length).replace(/^[\\/]+/, '') : path;
}

export const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;
export const samePath = (a: string, b: string) => a.replace(/\//g, '\\').toLowerCase() === b.replace(/\//g, '\\').toLowerCase();

/** Líneas añadidas y quitadas (comparando multiconjuntos de líneas: rápido y suficiente para el aviso). */
export function diffCounts(before: string, after: string): { added: number; removed: number } {
  const count = new Map<string, number>();
  for (const line of before.split('\n')) count.set(line, (count.get(line) ?? 0) + 1);
  let added = 0;
  for (const line of after.split('\n')) {
    const left = count.get(line) ?? 0;
    if (left > 0) count.set(line, left - 1);
    else added++;
  }
  let removed = 0;
  for (const left of count.values()) removed += left;
  return { added, removed };
}

export function formatTokens(n: number): string {
  return n >= 1024 ? `${Math.round(n / 1024)}K` : String(n); // 65536 → 64K, como lo nombra Bionic
}

export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export const gb = (n: number) => n.toLocaleString('es', { maximumFractionDigits: 1, minimumFractionDigits: 1 });

const LANGUAGES: Record<string, string> = {
  js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'javascript', ts: 'typescript', tsx: 'typescript',
  py: 'python', html: 'html', htm: 'html', css: 'css', json: 'json', md: 'markdown', rs: 'rust', bat: 'bat',
  cmd: 'bat', ps1: 'powershell', sh: 'shell', yml: 'yaml', yaml: 'yaml', toml: 'ini', ini: 'ini', xml: 'xml',
  svg: 'xml', sql: 'sql', java: 'java', cs: 'csharp', go: 'go', c: 'c', h: 'c', cpp: 'cpp', php: 'php', rb: 'ruby',
};

export function languageOf(path: string): string {
  return LANGUAGES[baseName(path).split('.').pop()?.toLowerCase() ?? ''] ?? 'plaintext';
}

/** Servidor local que lanzó el agente ("python -m http.server 8000" → http://localhost:8000). */
export function serverUrl(command: string): string | null {
  const url = /https?:\/\/(localhost|127\.0\.0\.1):\d+\S*/.exec(command)?.[0];
  if (url) return url;
  const port = /(?:http\.server\s+|--port[=\s]+|\s-p\s+|localhost:|127\.0\.0\.1:)(\d{2,5})\b/.exec(command)?.[1]
    ?? (/http\.server\s*$/.test(command) ? '8000' : undefined); // python -m http.server sin puerto
  return port ? `http://localhost:${port}` : null;
}
