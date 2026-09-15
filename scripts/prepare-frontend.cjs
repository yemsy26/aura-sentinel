const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const vendor = path.join(root, 'src', 'vendor');
fs.mkdirSync(vendor, { recursive: true });
fs.cpSync(path.join(root, 'node_modules/monaco-editor/min/vs'), path.join(vendor, 'monaco/vs'), { recursive: true });
fs.copyFileSync(path.join(root, 'node_modules/dompurify/dist/purify.min.js'), path.join(vendor, 'purify.min.js'));
for (const [pkg, license] of [['monaco-editor', 'LICENSE'], ['dompurify', 'LICENSE']]) {
    fs.copyFileSync(path.join(root, 'node_modules', pkg, license), path.join(vendor, pkg + '-LICENSE'));
}
