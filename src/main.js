const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;


async function loadOllamaModels() {
    try {
        const models = await invoke('get_ollama_models').then(list => list.map(name => ({name})));
        
        const orchestratorSelect = document.getElementById('orchestrator-select');
        const programmerSelect = document.getElementById('programmer-select');
        
        orchestratorSelect.innerHTML = '';
        programmerSelect.innerHTML = '';
        
        models.forEach(m => {
            const opt1 = document.createElement('option');
            opt1.value = m.name;
            opt1.textContent = m.name;
            const opt2 = document.createElement('option');
            opt2.value = m.name;
            opt2.textContent = m.name;
            
            orchestratorSelect.appendChild(opt1);
            programmerSelect.appendChild(opt2);
        });
        
        // Default selections
        if(models.some(m => m.name.includes("llama3.1:8b"))) orchestratorSelect.value = "llama3.1:8b";
        if(models.some(m => m.name.includes("qwen2.5-coder:14b"))) programmerSelect.value = "qwen2.5-coder:14b";
        
    } catch (e) {
        console.error("No se pudieron cargar los modelos de Ollama", e);
    }
}

document.addEventListener('DOMContentLoaded', async () => {
    const chatInput = document.getElementById('chat-input');
    const chatMessages = document.getElementById('chat-messages');
    const systemThoughts = document.getElementById('system-thoughts');
    const loadWorkspaceBtn = document.getElementById('load-workspace-btn');
    const workspaceTree = document.getElementById('workspace-tree');
    const clearChatBtn = document.getElementById('clear-chat-btn');
    const editorTabsContainer = document.getElementById('editor-tabs');

    let currentWorkspace = "Ninguno";
    let expandedFolders = new Set();

    // SPRINT 3: TABS STATE
    // Map: filePath -> { model, viewState, isDirty }
    let openTabs = new Map();
    let activeTabPath = null;

    window.editor = null;
    window.currentOpenFile = null;

    // ─────────────────────────────────────────────
    // MONACO INIT
    // ─────────────────────────────────────────────
    if (window.require) {
        window.require(['vs/editor/editor.main'], function () {
            window.editor = monaco.editor.create(document.getElementById('monaco-editor-host'), {
                theme: "vs-dark",
                automaticLayout: true,
                minimap: { enabled: true, scale: 1 },
                fontSize: 13,
                fontFamily: "'JetBrains Mono', 'Fira Code', 'Consolas', monospace",
                scrollBeyondLastLine: false,
                renderWhitespace: "selection",
                cursorBlinking: "smooth",
                smoothScrolling: true,
                lineNumbersMinChars: 3,
            });

            // Ctrl+S saves the active tab
            window.editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, async function () {
                await saveActiveTab();
            });

            // Mark tab dirty on any change
            window.editor.onDidChangeModelContent(() => {
                if (activeTabPath && openTabs.has(activeTabPath)) {
                    const tab = openTabs.get(activeTabPath);
                    if (!tab.isDirty) {
                        tab.isDirty = true;
                        renderTabs();
    loadOllamaModels();
                    }
                }
            });

            // HOT-RELOAD: fires when the backend agent writes a file
            window.__TAURI__.event.listen('file-updated', async (event) => {
                const updatedPath = event.payload.path;
                const shortName = updatedPath.split(/[\\\/]/).pop();
                logSystemThought(`[HOT-RELOAD] Agente modifico: ${shortName}`, '#e3b341');
                if (openTabs.has(updatedPath)) {
                    try {
                        const newContent = await invoke('read_file_content', { path: updatedPath });
                        const tab = openTabs.get(updatedPath);
                        const fullRange = tab.model.getFullModelRange();
                        tab.model.pushEditOperations([], [{ range: fullRange, text: newContent }], () => null);
                        tab.isDirty = false;
                        renderTabs();
    loadOllamaModels();
                    } catch (e) {
                        console.error("Hot-reload error:", e);
                    }
                }
            });
        });
    }

    // ─────────────────────────────────────────────
    // TABS RENDERING
    // ─────────────────────────────────────────────
    function getFileIcon(name) {
        const ext = name.split('.').pop().toLowerCase();
        if (['js', 'jsx', 'ts', 'tsx', 'mjs'].includes(ext)) return '\u26a1';
        if (ext === 'rs') return '\ud83e\udd80';
        if (ext === 'css') return '\ud83c\udfa8';
        if (ext === 'html') return '\ud83c\udf10';
        if (['json', 'toml', 'yaml', 'yml'].includes(ext)) return '\ud83d\udce6';
        if (ext === 'md') return '\ud83d\udcdd';
        if (ext === 'py') return '\ud83d\udc0d';
        if (['sh', 'bash', 'ps1'].includes(ext)) return '\u2699\ufe0f';
        if (ext === 'lock') return '\ud83d\udd12';
        return '\ud83d\udcc4';
    }

    function renderTabs() {
        if (!editorTabsContainer) return;
        editorTabsContainer.innerHTML = '';

        if (openTabs.size === 0) {
            editorTabsContainer.innerHTML = '<div style="padding: 8px 12px; color: #6e7681; font-size: 12px; font-style: italic;">Abre un archivo desde el Workspace...</div>';
            if (window.editor) window.editor.setModel(null);
            window.currentOpenFile = null;
            activeTabPath = null;
            return;
        }

        for (let [path, tab] of openTabs.entries()) {
            const tabEl = document.createElement('div');
            const isActive = path === activeTabPath;
            tabEl.className = 'editor-tab' + (isActive ? ' active' : '');

            const filename = path.split(/[\\\/]/).pop();
            const icon = getFileIcon(filename);

            const iconSpan = document.createElement('span');
            iconSpan.textContent = icon;
            iconSpan.style.marginRight = '5px';
            iconSpan.style.fontSize = '11px';

            const titleSpan = document.createElement('span');
            titleSpan.textContent = filename;
            titleSpan.title = path;
            titleSpan.style.maxWidth = '130px';
            titleSpan.style.overflow = 'hidden';
            titleSpan.style.textOverflow = 'ellipsis';
            titleSpan.style.whiteSpace = 'nowrap';

            const dirtyDot = document.createElement('span');
            dirtyDot.style.color = '#e3b341';
            dirtyDot.style.marginLeft = '4px';
            dirtyDot.style.fontSize = '10px';
            dirtyDot.textContent = tab.isDirty ? '\u25cf' : '';

            const closeBtn = document.createElement('span');
            closeBtn.className = 'close-btn';
            closeBtn.textContent = '\u00d7';
            closeBtn.title = 'Cerrar pestana';
            closeBtn.onclick = (e) => { e.stopPropagation(); closeTab(path); };

            tabEl.onclick = () => switchTab(path);
            tabEl.appendChild(iconSpan);
            tabEl.appendChild(titleSpan);
            tabEl.appendChild(dirtyDot);
            tabEl.appendChild(closeBtn);
            editorTabsContainer.appendChild(tabEl);
        }
    }

    function closeTab(path) {
        if (!openTabs.has(path)) return;
        const tab = openTabs.get(path);
        if (path === activeTabPath && window.editor) {
            tab.viewState = window.editor.saveViewState();
        }
        tab.model.dispose();
        openTabs.delete(path);

        if (path === activeTabPath) {
            const remaining = Array.from(openTabs.keys());
            if (remaining.length > 0) switchTab(remaining[remaining.length - 1]);
            else { activeTabPath = null; renderTabs(); }
        } else {
            renderTabs();
    loadOllamaModels();
        }
    }

    function switchTab(path) {
        if (!openTabs.has(path) || !window.editor) return;
        if (activeTabPath && openTabs.has(activeTabPath)) {
            openTabs.get(activeTabPath).viewState = window.editor.saveViewState();
        }
        activeTabPath = path;
        window.currentOpenFile = path;
        const tab = openTabs.get(path);
        window.editor.setModel(tab.model);
        if (tab.viewState) window.editor.restoreViewState(tab.viewState);
        window.editor.focus();
        renderTabs();
    loadOllamaModels();
    }

    async function saveActiveTab() {
        if (!activeTabPath || !openTabs.has(activeTabPath)) return;
        const tab = openTabs.get(activeTabPath);
        try {
            await invoke('save_file_content', { path: activeTabPath, content: tab.model.getValue() });
            tab.isDirty = false;
            renderTabs();
    loadOllamaModels();
            logSystemThought(`[GUARDADO] ${activeTabPath.split(/[\\\/]/).pop()}`, '#3fb950');
        } catch (e) {
            logSystemThought(`[ERROR] No se pudo guardar: ${e}`, '#f85149');
        }
    }

    // ─────────────────────────────────────────────
    // LOGGING
    // ─────────────────────────────────────────────
    function logSystemThought(message, color = '#00ff00') {
        const p = document.createElement('p');
        const timeStr = new Date().toTimeString().split(' ')[0];
        p.textContent = `[${timeStr}] ${message}`;
        p.style.color = color;
        p.style.margin = '1px 0';
        systemThoughts.appendChild(p);
        systemThoughts.scrollTop = systemThoughts.scrollHeight;
    }

    function appendMessageToDOM(sender, text, updateScroll = true) {
        const div = document.createElement('div');
        div.style.marginBottom = '12px';
        div.style.lineHeight = '1.5';
        const isUser = sender === 'user';
        const label = isUser ? 'YOU' : 'AURA';
        const labelColor = isUser ? '#79c0ff' : '#3fb950';
        const rendered = String(text).replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>');
        div.innerHTML = `<span style="color:${labelColor};font-weight:bold;">[${label}]</span> <span style="color:#c9d1d9;">${rendered}</span>`;
        chatMessages.appendChild(div);
        if (updateScroll) chatMessages.scrollTop = chatMessages.scrollHeight;
        return div;
    }

    async function saveMessageToDisk(sender, text) {
        try {
            await invoke('save_chat_message', {
                workspacePath: currentWorkspace,
                message: { sender, text, timestamp: new Date().toISOString() }
            });
        } catch (e) { console.error("Error guardando chat:", e); }
    }

    async function loadChatHistory() {
        chatMessages.innerHTML = '';
        try {
            const chatJson = await invoke('load_chat_history', { workspacePath: currentWorkspace });
            const logs = JSON.parse(chatJson);
            if (logs.length > 0) {
                logs.forEach(msg => appendMessageToDOM(msg.sender, msg.text, false));
                chatMessages.scrollTop = chatMessages.scrollHeight;
            } else {
                chatMessages.innerHTML = '<div style="color:#6e7681;text-align:center;margin-top:30px;font-style:italic;">Esperando instrucciones tacticas...</div>';
            }
        } catch (e) { console.error("Error cargando historial:", e); }
    }

    clearChatBtn.addEventListener('click', async () => {
        if (currentWorkspace !== "Ninguno") {
            try {
                await invoke('clear_chat_history', { workspacePath: currentWorkspace });
                chatMessages.innerHTML = '<div style="color:#6e7681;text-align:center;margin-top:30px;font-style:italic;">Chat limpiado.</div>';
                logSystemThought(`[SISTEMA] Historial eliminado.`, '#6e7681');
            } catch (e) { logSystemThought(`[ERROR] ${e}`, '#f85149'); }
        }
    });

    // ─────────────────────────────────────────────
    // AUTO-LOAD WORKSPACE
    // ─────────────────────────────────────────────
    logSystemThought("\u25c8 AURA-SENTINEL v3.0 \u2014 Inicializando...", '#58a6ff');
    try {
        const defaultCwd = await invoke('get_current_directory');
        const savedWorkspace = localStorage.getItem('aura_workspace');
        currentWorkspace = savedWorkspace || defaultCwd;

        if (savedWorkspace) logSystemThought(`[MEMORIA] Workspace restaurado: ${currentWorkspace}`, '#d29922');
        else logSystemThought(`[WORKSPACE] ${currentWorkspace}`);

        const initResult = await invoke('init_memory_log', { workspacePath: currentWorkspace });
        logSystemThought(`[MEM] ${initResult}`, '#6e7681');

        logSystemThought("Escaneando estructura del proyecto...");
        const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
        const treeData = JSON.parse(treeJson);
        renderTree(treeData, workspaceTree);
        logSystemThought(`\u2713 ${treeData.length} nodos. Listo para operar.`, '#3fb950');

        await loadChatHistory();

        const memoryJson = await invoke('read_memory_logs', { workspacePath: currentWorkspace });
        const memoryLogs = JSON.parse(memoryJson);
        if (memoryLogs.length > 0) {
            memoryLogs.forEach(log => logSystemThought(`[LOG] ${log.timestamp} \u00b7 ${log.file_path}`, '#d29922'));
        } else {
            logSystemThought(`[MEM] Proyecto nuevo. Sin registros previos.`, '#6e7681');
        }
    } catch (error) {
        logSystemThought(`[FATAL] Error de auto-carga: ${error}`, '#f85149');
    }

    // ─────────────────────────────────────────────
    // MANUAL WORKSPACE LOAD
    // ─────────────────────────────────────────────
    loadWorkspaceBtn.addEventListener('click', async () => {
        try {
            const selectedPath = await open({ directory: true, multiple: false });
            if (selectedPath) {
                currentWorkspace = selectedPath;
                localStorage.setItem('aura_workspace', currentWorkspace);
                systemThoughts.innerHTML = '';
                logSystemThought(`[WORKSPACE] Cambiado a: ${currentWorkspace}`, '#58a6ff');

                for (let [, tab] of openTabs.entries()) tab.model.dispose();
                openTabs.clear();
                activeTabPath = null;
                renderTabs();
    loadOllamaModels();

                const initResult = await invoke('init_memory_log', { workspacePath: currentWorkspace });
                logSystemThought(`[MEM] ${initResult}`, '#6e7681');

                const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
                const treeData = JSON.parse(treeJson);
                renderTree(treeData, workspaceTree);
                logSystemThought(`\u2713 ${treeData.length} nodos identificados.`, '#3fb950');

                await loadChatHistory();
                const memoryJson = await invoke('read_memory_logs', { workspacePath: currentWorkspace });
                const mLogs = JSON.parse(memoryJson);
                if (mLogs.length > 0) mLogs.forEach(log => logSystemThought(`[LOG] ${log.file_path}`, '#d29922'));
                else logSystemThought(`[MEM] Proyecto nuevo.`, '#6e7681');
            }
        } catch (error) { logSystemThought(`[ERROR] ${error}`, '#f85149'); }
    });

    // ─────────────────────────────────────────────
    // FILE TREE
    // ─────────────────────────────────────────────
    function renderTree(nodes, container) {
        const currentExpanded = new Set();
        container.querySelectorAll('ul.tree-list ul').forEach(ul => {
            if (ul.style.display === 'block') {
                const parentSpan = ul.previousElementSibling;
                if (parentSpan && parentSpan.dataset.key) currentExpanded.add(parentSpan.dataset.key);
            }
        });
        expandedFolders.forEach(f => currentExpanded.add(f));
        expandedFolders = currentExpanded;
        container.innerHTML = '';

        const nodeMap = {};
        const rootNodes = [];
        nodes.forEach(node => { node.children = []; nodeMap[node.path.replace(/\\/g, '/')] = node; });
        nodes.forEach(node => {
            let placed = false;
            if (node.parent_path) {
                const normP = node.parent_path.replace(/\\/g, '/');
                if (nodeMap[normP]) { nodeMap[normP].children.push(node); placed = true; }
            }
            if (!placed) rootNodes.push(node);
        });

        function createTreeElement(node) {
            const li = document.createElement('li');
            li.className = node.is_dir ? 'tree-folder' : 'tree-file';
            li.title = node.path;

            const span = document.createElement('span');
            span.dataset.key = node.path;
            span.style.cssText = 'display:inline-flex;align-items:center;padding:2px 5px;border-radius:3px;cursor:pointer;user-select:none;';

            const iconEl = document.createElement('span');
            iconEl.style.marginRight = '5px';

            const nameEl = document.createElement('span');
            nameEl.textContent = node.name;

            li.appendChild(span);

            if (node.is_dir && node.children.length > 0) {
                const isExpanded = rootNodes.includes(node) || expandedFolders.has(node.path);
                iconEl.textContent = isExpanded ? '\ud83d\udcc2' : '\ud83d\udcc1';
                span.appendChild(iconEl);
                span.appendChild(nameEl);

                const ul = document.createElement('ul');
                ul.style.cssText = `display:${isExpanded ? 'block' : 'none'};padding-left:16px;list-style:none;margin:0;`;

                node.children.sort((a, b) => {
                    if (a.is_dir && !b.is_dir) return -1;
                    if (!a.is_dir && b.is_dir) return 1;
                    return a.name.localeCompare(b.name);
                });
                node.children.forEach(child => ul.appendChild(createTreeElement(child)));
                li.appendChild(ul);

                span.addEventListener('click', (e) => {
                    e.stopPropagation();
                    const willExpand = ul.style.display === 'none';
                    ul.style.display = willExpand ? 'block' : 'none';
                    iconEl.textContent = willExpand ? '\ud83d\udcc2' : '\ud83d\udcc1';
                    if (willExpand) expandedFolders.add(node.path);
                    else expandedFolders.delete(node.path);
                });
            } else if (node.is_dir) {
                iconEl.textContent = '\ud83d\udcc1';
                span.appendChild(iconEl);
                span.appendChild(nameEl);
            } else {
                iconEl.textContent = getFileIcon(node.name);
                nameEl.style.color = openTabs.has(node.path) ? '#c9d1d9' : '#8b949e';
                span.appendChild(iconEl);
                span.appendChild(nameEl);

                span.addEventListener('click', async (e) => {
                    e.stopPropagation();
                    if (!window.editor) { logSystemThought('[WARN] Monaco no esta listo.', '#d29922'); return; }

                    if (openTabs.has(node.path)) { switchTab(node.path); return; }

                    try {
                        const fileContent = await invoke('read_file_content', { path: node.path });
                        const ext = node.name.split('.').pop().toLowerCase();
                        let lang = 'plaintext';
                        if (['js','jsx','ts','tsx','mjs'].includes(ext)) lang = 'javascript';
                        else if (ext === 'rs') lang = 'rust';
                        else if (ext === 'html') lang = 'html';
                        else if (ext === 'css') lang = 'css';
                        else if (['json','toml'].includes(ext)) lang = 'json';
                        else if (['yaml','yml'].includes(ext)) lang = 'yaml';
                        else if (ext === 'md') lang = 'markdown';
                        else if (ext === 'py') lang = 'python';
                        else if (['sh','bash','ps1'].includes(ext)) lang = 'shell';

                        const uri = monaco.Uri.file(node.path);
                        let model = monaco.editor.getModel(uri);
                        if (!model) model = monaco.editor.createModel(fileContent, lang, uri);

                        openTabs.set(node.path, { model, viewState: null, isDirty: false });
                        nameEl.style.color = '#c9d1d9';
                        switchTab(node.path);
                    } catch (err) {
                        logSystemThought(`[ERROR] No se pudo leer: ${err}`, '#f85149');
                    }
                });
            }
            return li;
        }

        const ul = document.createElement('ul');
        ul.className = 'tree-list';
        ul.style.cssText = 'list-style:none;padding-left:4px;margin:0;';
        rootNodes.sort((a, b) => {
            if (a.is_dir && !b.is_dir) return -1;
            if (!a.is_dir && b.is_dir) return 1;
            return a.name.localeCompare(b.name);
        });
        rootNodes.forEach(root => ul.appendChild(createTreeElement(root)));
        container.appendChild(ul);
    }

    // ─────────────────────────────────────────────
    // CHAT INPUT
    // ─────────────────────────────────────────────
    chatInput.addEventListener('keypress', async (e) => {
        if (e.key === 'Enter' && chatInput.value.trim() !== '') {
            let text = chatInput.value.trim();
            chatInput.value = '';
            chatInput.disabled = true;

            text = text.replace(/^\[USER\]\s*/i, '');
            const sysIdx = text.indexOf('[SYSTEM]');
            if (sysIdx !== -1) text = text.substring(0, sysIdx).trim();

            appendMessageToDOM('user', text);
            await saveMessageToDisk('user', text);

            let loadingMsg = appendMessageToDOM('system', '\u23f3 Analizando...');
            logSystemThought("\u25ba Enviando prompt al Orquestador...", '#58a6ff');

            try {
                const unlisten = await window.__TAURI__.event.listen('agent-step', (event) => {
                    const { step, status, message } = event.payload;
                    let color = '#79c0ff';
                    if (status === 'DECISION') color = '#e3b341';
                    else if (status === 'ACTION') color = '#bc8cff';
                    else if (status === 'SUCCESS') color = '#3fb950';
                    else if (status === 'ERROR' || status === 'FATAL') color = '#f85149';
                    else if (status === 'VALIDATING') color = '#1f6feb';
                    else if (status === 'WARNING') color = '#d29922';

                    logSystemThought(`[${step}] [${status}] ${message}`, color);
                    loadingMsg.innerHTML = `<span style="color:${color};font-weight:bold;">[PASO ${step}]</span> <span style="color:#c9d1d9;">${message}</span>`;

                    if (status === 'SUCCESS') {
                        invoke('get_workspace_tree', { path: currentWorkspace }).then(tj => renderTree(JSON.parse(tj), workspaceTree));
                    }
                });

                const responseString = await invoke('process_user_prompt', {
                    userMessage: text,
                    workspacePath: currentWorkspace,
                    orchestratorModel: document.getElementById('orchestrator-select').value || "llama3.1:8b",
                    programmerModel: document.getElementById('programmer-select').value || "qwen2.5-coder:14b"
                });

                unlisten();
                chatInput.disabled = false;
                chatInput.focus();

                try {
                    const data = JSON.parse(responseString);
                    if (data.status === 'FINISH' || data.status === 'ERROR') {
                        loadingMsg.innerHTML = `<span style="color:#3fb950;font-weight:bold;">[AURA]</span> <span style="color:#c9d1d9;">${data.respuesta_conversacional}</span>`;
                        await saveMessageToDisk('system', data.respuesta_conversacional);
                    } else {
                        loadingMsg.innerHTML = `<span style="color:#8b949e;">[SYSTEM]</span> ${responseString}`;
                    }
                } catch {
                    loadingMsg.innerHTML = `<span style="color:#8b949e;">[SYSTEM]</span> ${responseString}`;
                }

                const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
                renderTree(JSON.parse(treeJson), workspaceTree);

            } catch (error) {
                chatInput.disabled = false;
                logSystemThought(`[ERROR PIPELINE] ${error}`, '#f85149');
                loadingMsg.innerHTML = `<span style="color:#f85149;font-weight:bold;">[ERROR]</span> <span style="color:#c9d1d9;">${error}</span>`;
            }
        }
    });

    // ─────────────────────────────────────────────
    // TELEMETRY
    // ─────────────────────────────────────────────
    setInterval(async () => {
        try {
            const stats = await invoke('get_system_stats');
            document.getElementById('ram-usage').textContent = stats;
        } catch (e) { /* silent */ }
    }, 2000);

    // Initialize tabs bar
    renderTabs();
    loadOllamaModels();

    // ─────────────────────────────────────────────
    // UI IMPROVEMENTS: RESIZING & COLLAPSING
    // ─────────────────────────────────────────────
    const root = document.documentElement;
    const workspacePanel = document.getElementById('workspace-panel');
    const rightStack = document.querySelector('.panel-right-stack');
    const gutter1 = document.getElementById('gutter-1');
    const gutter2 = document.getElementById('gutter-2');

    // 1. Load Persisted State
    try {
        const layoutState = JSON.parse(localStorage.getItem('aura_layout') || '{}');
        if (layoutState.leftWidth) root.style.setProperty('--left-panel-width', layoutState.leftWidth);
        if (layoutState.rightWidth) root.style.setProperty('--right-panel-width', layoutState.rightWidth);
        
        if (layoutState.panels) {
            Object.entries(layoutState.panels).forEach(([id, isMinimized]) => {
                const el = document.getElementById(id);
                if (el && isMinimized) {
                    el.classList.add(id === 'workspace-panel' ? 'minimized-h' : 'minimized-v');
                    if(id === 'workspace-panel') root.style.setProperty('--left-panel-width', '40px');
                }
            });
        }
    } catch(e) {}

    function saveLayoutState() {
        const panels = {};
        ['workspace-panel', 'chat-panel', 'console-panel', 'telemetry-panel'].forEach(id => {
            const el = document.getElementById(id);
            if (el) {
                panels[id] = el.classList.contains('minimized-v') || el.classList.contains('minimized-h');
            }
        });
        localStorage.setItem('aura_layout', JSON.stringify({
            leftWidth: root.style.getPropertyValue('--left-panel-width'),
            rightWidth: root.style.getPropertyValue('--right-panel-width'),
            panels
        }));
        if(window.editor) setTimeout(() => window.editor.layout(), 250);
    }

    // 2. Resizing Logic (Drag & Drop Native API)
    let isResizing = null;
    let startX = 0;
    let startLeftWidth = 0;
    let startRightWidth = 0;

    gutter1.addEventListener('mousedown', (e) => {
        isResizing = 'left';
        startX = e.clientX;
        startLeftWidth = parseInt(getComputedStyle(root).getPropertyValue('--left-panel-width')) || 280;
        document.body.style.cursor = 'col-resize';
        gutter1.classList.add('active');
        e.preventDefault();
    });

    gutter2.addEventListener('mousedown', (e) => {
        isResizing = 'right';
        startX = e.clientX;
        startRightWidth = parseInt(getComputedStyle(root).getPropertyValue('--right-panel-width')) || 400;
        document.body.style.cursor = 'col-resize';
        gutter2.classList.add('active');
        e.preventDefault();
    });

    document.addEventListener('mousemove', (e) => {
        if (!isResizing) return;
        if (isResizing === 'left') {
            const newWidth = Math.max(40, startLeftWidth + (e.clientX - startX));
            root.style.setProperty('--left-panel-width', `${newWidth}px`);
            if (newWidth > 100) workspacePanel.classList.remove('minimized-h');
        } else if (isResizing === 'right') {
            const newWidth = Math.max(200, startRightWidth - (e.clientX - startX));
            root.style.setProperty('--right-panel-width', `${newWidth}px`);
        }
        if(window.editor) window.editor.layout();
    });

    document.addEventListener('mouseup', () => {
        if (isResizing) {
            isResizing = null;
            document.body.style.cursor = 'default';
            gutter1.classList.remove('active');
            gutter2.classList.remove('active');
            saveLayoutState();
        }
    });

    // 3. Minimizing Panels (Click on headers)
    document.querySelectorAll('.panel').forEach(panel => {
        const minBtn = panel.querySelector('.min-btn');
        if (!minBtn) return;
        minBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            if (panel.id === 'workspace-panel') {
                const isMin = panel.classList.toggle('minimized-h');
                root.style.setProperty('--left-panel-width', isMin ? '40px' : '280px');
            } else {
                panel.classList.toggle('minimized-v');
            }
            saveLayoutState();
        });
    });

    // 4. Keyboard Shortcuts
    document.addEventListener('keydown', (e) => {
        // Ctrl+Shift+M: Minimize/Restore all vertical panels + workspace
        if (e.ctrlKey && e.shiftKey && e.code === 'KeyM') {
            e.preventDefault();
            const panelsToToggle = ['chat-panel', 'console-panel', 'telemetry-panel'];
            let allMin = true;
            panelsToToggle.forEach(id => {
                if(!document.getElementById(id).classList.contains('minimized-v')) allMin = false;
            });
            
            panelsToToggle.forEach(id => {
                const el = document.getElementById(id);
                if(allMin) el.classList.remove('minimized-v');
                else el.classList.add('minimized-v');
            });
            
            if (allMin) {
                workspacePanel.classList.remove('minimized-h');
                root.style.setProperty('--left-panel-width', '280px');
            } else {
                workspacePanel.classList.add('minimized-h');
                root.style.setProperty('--left-panel-width', '40px');
            }
            saveLayoutState();
        }
        
        // Ctrl+Shift+R: Reset layout
        if (e.ctrlKey && e.shiftKey && e.code === 'KeyR') {
            e.preventDefault();
            root.style.setProperty('--left-panel-width', '280px');
            root.style.setProperty('--right-panel-width', '400px');
            document.querySelectorAll('.panel').forEach(p => {
                p.classList.remove('minimized-v', 'minimized-h');
            });
            saveLayoutState();
        }
    });


    // --- BACKGROUND TASKS MANAGER ---
    const bgTasksSection = document.getElementById('bg-tasks-section');
    const bgTasksList = document.getElementById('bg-tasks-list');
    
    async function updateBackgroundTasks() {
        try {
            const tasks = await invoke('get_background_tasks');
            if (tasks.length === 0) {
                bgTasksSection.style.display = 'none';
                bgTasksList.innerHTML = '';
            } else {
                bgTasksSection.style.display = 'block';
                bgTasksList.innerHTML = '';
                tasks.forEach(task => {
                    const taskEl = document.createElement('div');
                    taskEl.style.display = 'flex';
                    taskEl.style.justifyContent = 'space-between';
                    taskEl.style.alignItems = 'center';
                    taskEl.style.background = '#21262d';
                    taskEl.style.padding = '2px 6px';
                    taskEl.style.borderRadius = '4px';
                    
                    const cmdText = task.command.length > 40 ? task.command.substring(0, 37) + '...' : task.command;
                    
                    const span = document.createElement('span');
                    span.style.color = '#8b949e';
                    span.title = task.command;
                    span.textContent = `[${task.id}] ${cmdText}`;
                    
                    const killBtn = document.createElement('button');
                    killBtn.innerHTML = '&#x2715;'; // X mark
                    killBtn.style.background = 'transparent';
                    killBtn.style.border = 'none';
                    killBtn.style.color = '#ff7b72';
                    killBtn.style.cursor = 'pointer';
                    killBtn.style.fontSize = '10px';
                    killBtn.title = 'Kill Process';
                    
                    killBtn.onclick = async () => {
                        killBtn.disabled = true;
                        killBtn.style.opacity = '0.5';
                        try {
                            await invoke('ui_kill_task', { taskId: task.id });
                            setTimeout(updateBackgroundTasks, 500); // refresh after kill
                        } catch (e) {
                            console.error("Failed to kill task", e);
                            killBtn.disabled = false;
                            killBtn.style.opacity = '1';
                        }
                    };
                    
                    taskEl.appendChild(span);
                    taskEl.appendChild(killBtn);
                    bgTasksList.appendChild(taskEl);
                });
            }
        } catch (e) {
            console.error("Error fetching background tasks", e);
        }
    }
    
    // Poll every 5 seconds for background tasks
    setInterval(updateBackgroundTasks, 5000);
    setTimeout(updateBackgroundTasks, 1000);

});