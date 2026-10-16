// Monaco empaquetado por Vite con sus workers (sin CDN: la app funciona sin conexión).
import * as monaco from 'monaco-editor';
import EditorWorker from 'monaco-editor/editor/editor.worker?worker';
import CssWorker from 'monaco-editor/language/css/css.worker?worker';
import HtmlWorker from 'monaco-editor/language/html/html.worker?worker';
import JsonWorker from 'monaco-editor/language/json/json.worker?worker';
import TsWorker from 'monaco-editor/language/typescript/ts.worker?worker';

self.MonacoEnvironment = {
  getWorker(_id: string, label: string) {
    if (label === 'json') return new JsonWorker();
    if (label === 'css' || label === 'scss' || label === 'less') return new CssWorker();
    if (label === 'html' || label === 'handlebars' || label === 'razor') return new HtmlWorker();
    if (label === 'typescript' || label === 'javascript') return new TsWorker();
    return new EditorWorker();
  },
};

monaco.editor.defineTheme('aura', {
  base: 'vs-dark',
  inherit: true,
  rules: [],
  colors: {
    'editor.background': '#0E1015',
    'editor.lineHighlightBackground': '#161A22',
    'editorLineNumber.foreground': '#4B5263',
    'editorGutter.background': '#0E1015',
    'diffEditor.insertedLineBackground': '#5CCB8A1F',
    'diffEditor.removedLineBackground': '#EF7B7B1F',
  },
});

export { monaco };
