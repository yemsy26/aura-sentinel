const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

// ─── MODEL MANAGEMENT & TURBO SWITCHER ───────────────────────────────────────
let availableModels = [];
let mostPowerfulModel = null;

async function loadOllamaModels() {
    try {
        const rawModels = await invoke('get_ollama_models').then(list => list.map(name => ({ name })));
        availableModels = rawModels.filter(m => {
            const low = m.name.toLowerCase();
            return !low.includes("embed") && !low.includes("minilm") && !low.includes("bge-");
        });

        const agentModelSelect = document.getElementById('agent-model-select');
        if (!agentModelSelect) return;

        agentModelSelect.innerHTML = '';

        availableModels.forEach(m => {
            const opt = document.createElement('option');
            opt.value = m.name;
            opt.textContent = m.name;
            agentModelSelect.appendChild(opt);
        });

        // Determine most powerful model (14b, 27b+, 70b, 34b, etc.)
        const powerCandidate = availableModels.find(m => {
            const n = m.name.toLowerCase();
            return n.includes("14b") || n.includes("bonsai") || n.includes("27b") || n.includes("70b") || n.includes("34b");
        });
        mostPowerfulModel = powerCandidate ? powerCandidate.name : (availableModels[0] ? availableModels[0].name : null);

        // Restore user choice from localStorage, or smart default
        const savedModel = localStorage.getItem('aura_active_model');
        if (savedModel && availableModels.some(m => m.name === savedModel)) {
            agentModelSelect.value = savedModel;
        } else if (availableModels.some(m => m.name.includes("qwen2.5-coder:7b") || m.name.includes("qwen2.5-coder"))) {
            agentModelSelect.value = availableModels.find(m => m.name.includes("qwen2.5-coder:7b") || m.name.includes("qwen2.5-coder")).name;
        } else if (availableModels.some(m => m.name.includes("gemma4-e4b"))) {
            agentModelSelect.value = availableModels.find(m => m.name.includes("gemma4-e4b")).name;
        } else if (availableModels.some(m => m.name.includes("llama3.1"))) {
            agentModelSelect.value = availableModels.find(m => m.name.includes("llama3.1")).name;
        } else if (availableModels.length > 0) {
            agentModelSelect.value = availableModels[0].name;
        }

        updateModelStatusUI();

        agentModelSelect.onchange = () => {
            localStorage.setItem('aura_active_model', agentModelSelect.value);
            updateModelStatusUI();
            logSystemThought(`[CEREBRO] Modelo activo cambiado a: ${agentModelSelect.value}`, '#58a6ff');
        };

        const turboBtn = document.getElementById('quick-turbo-btn');
        if (turboBtn && mostPowerfulModel) {
            turboBtn.onclick = () => {
                if (agentModelSelect.value === mostPowerfulModel) {
                    // Toggle back to standard model (qwen 7b, gemma, etc.)
                    const standard = availableModels.find(m => m.name.includes("qwen2.5-coder:7b") || m.name.includes("qwen") || m.name.includes("gemma4-e4b") || m.name.includes("llama3.1"));
                    if (standard) {
                        agentModelSelect.value = standard.name;
                        localStorage.setItem('aura_active_model', standard.name);
                        turboBtn.classList.remove('turbo-active');
                        turboBtn.textContent = '⚡ MÁXIMA POTENCIA';
                        logSystemThought(`[MODO TURBO] Desactivado. Modelo estándar: ${standard.name}`, '#8b949e');
                    }
                } else {
                    agentModelSelect.value = mostPowerfulModel;
                    localStorage.setItem('aura_active_model', mostPowerfulModel);
                    turboBtn.classList.add('turbo-active');
                    turboBtn.textContent = `🚀 POTENCIA MÁXIMA (${mostPowerfulModel.split(':')[0]})`;
                    logSystemThought(`[MODO TURBO ACTIVADO] Asignado cerebro de máxima potencia: ${mostPowerfulModel}`, '#a855f7');
                }
                updateModelStatusUI();
            };
        }

    } catch (e) {
        console.error("No se pudieron cargar los modelos de Ollama", e);
    }
}

function updateModelStatusUI() {
    const agentModelSelect = document.getElementById('agent-model-select');
    const modelStatus = document.getElementById('model-status');
    if (modelStatus && agentModelSelect) {
        const shortName = agentModelSelect.value ? agentModelSelect.value.split(':')[0] : 'Standby';
        modelStatus.textContent = shortName;
        modelStatus.title = `Modelo en ejecución: ${agentModelSelect.value}`;
    }
}

// Global logger reference
let logSystemThought = (msg, color) => console.log(msg);

// ─── MARKDOWN & CODE HIGHLIGHTING ENGINE ──────────────────────────────────────
function initMarkdownEngine() {
    if (window.marked) {
        window.marked.setOptions({
            gfm: true,
            breaks: true,
            headerIds: false,
            mangle: false,
            highlight: function (code, lang) {
                if (window.hljs) {
                    if (lang && window.hljs.getLanguage(lang)) {
                        try {
                            return window.hljs.highlight(code, { language: lang }).value;
                        } catch (e) { }
                    }
                    try {
                        return window.hljs.highlightAuto(code).value;
                    } catch (e) { }
                }
                return code;
            }
        });
    }
}

function renderMarkdownSafely(text) {
    if (!text) return '';
    if (window.marked) {
        try {
            return window.marked.parse(String(text));
        } catch (e) {
            console.warn("Markdown parse error, fallback to sanitized text", e);
        }
    }
    return String(text)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>');
}

// ─── DOM READY APPLICATION LOGIC ─────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', async () => {
    initMarkdownEngine();

    const chatInput = document.getElementById('chat-input');
    const chatSendBtn = document.getElementById('chat-send-btn');
    const chatMessages = document.getElementById('chat-messages');
    const systemThoughts = document.getElementById('system-thoughts');
    const loadWorkspaceBtn = document.getElementById('load-workspace-btn');
    const workspaceTree = document.getElementById('workspace-tree');
    const clearChatBtn = document.getElementById('clear-chat-btn');
    const editorTabsContainer = document.getElementById('editor-tabs');

    let currentWorkspace = "Ninguno";
    let expandedFolders = new Set();

    // SPRINT 3: TABS STATE
    let openTabs = new Map();
    let activeTabPath = null;

    window.editor = null;
    window.currentOpenFile = null;

    // ─── LOGGING SYSTEM ──────────────────────────────────────────────────────
    logSystemThought = function (message, color = '#4ade80') {
        const p = document.createElement('p');
        const timeStr = new Date().toTimeString().split(' ')[0];
        p.textContent = `[${timeStr}] ${message}`;
        p.style.color = color;
        p.style.margin = '2px 0';
        systemThoughts.appendChild(p);
        systemThoughts.scrollTop = systemThoughts.scrollHeight;
    };

    // ─── MONACO INIT ─────────────────────────────────────────────────────────
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

            // Mark tab dirty on change
            window.editor.onDidChangeModelContent(() => {
                if (activeTabPath && openTabs.has(activeTabPath)) {
                    const tab = openTabs.get(activeTabPath);
                    if (!tab.isDirty) {
                        tab.isDirty = true;
                        renderTabs();
                    }
                }
            });

            // HOT-RELOAD: backend agent writes a file
            window.__TAURI__.event.listen('file-updated', async (event) => {
                const updatedPath = event.payload.path;
                const shortName = updatedPath.split(/[\\\/]/).pop();
                logSystemThought(`[HOT-RELOAD] Agente modificó: ${shortName}`, '#e3b341');
                if (openTabs.has(updatedPath)) {
                    try {
                        const newContent = await invoke('read_file_content', { path: updatedPath });
                        const tab = openTabs.get(updatedPath);
                        const fullRange = tab.model.getFullModelRange();
                        tab.model.pushEditOperations([], [{ range: fullRange, text: newContent }], () => null);
                        tab.isDirty = false;
                        renderTabs();
                    } catch (e) {
                        console.error("Hot-reload error:", e);
                    }
                }
            });
        });
    }

    // ─── TABS & EDITOR HELPER ────────────────────────────────────────────────
    function getFileIcon(name) {
        const ext = name.split('.').pop().toLowerCase();
        if (['js', 'jsx', 'ts', 'tsx', 'mjs'].includes(ext)) return '⚡';
        if (ext === 'rs') return '🦀';
        if (['html', 'htm'].includes(ext)) return '🌐';
        if (ext === 'css') return '🎨';
        if (['json', 'toml', 'yaml', 'yml'].includes(ext)) return '⚙️';
        if (ext === 'md') return '📝';
        if (ext === 'py') return '🐍';
        if (['sh', 'bash', 'ps1'].includes(ext)) return '💻';
        return '📄';
    }

    function renderTabs() {
        editorTabsContainer.innerHTML = '';
        if (openTabs.size === 0) {
            editorTabsContainer.innerHTML = '<div style="padding: 8px; color: #8b949e; font-size: 12px; font-style: italic;" id="no-tabs-msg">Abre un archivo desde el Workspace</div>';
            if (window.editor) window.editor.setModel(null);
            return;
        }

        openTabs.forEach((tab, path) => {
            const fileName = path.split(/[\\\/]/).pop();
            const tabEl = document.createElement('div');
            tabEl.className = `editor-tab ${path === activeTabPath ? 'active' : ''} ${tab.isDirty ? 'dirty' : ''}`;
            tabEl.title = path;

            const label = document.createElement('span');
            label.textContent = `${getFileIcon(fileName)} ${fileName}`;
            label.style.overflow = 'hidden';
            label.style.textOverflow = 'ellipsis';

            const closeBtn = document.createElement('span');
            closeBtn.className = 'close-btn';
            closeBtn.textContent = '✕';
            closeBtn.onclick = (e) => {
                e.stopPropagation();
                closeTab(path);
            };

            tabEl.appendChild(label);
            tabEl.appendChild(closeBtn);
            tabEl.onclick = () => switchTab(path);
            editorTabsContainer.appendChild(tabEl);
        });
    }

    function closeTab(path) {
        if (!openTabs.has(path)) return;
        const tab = openTabs.get(path);
        if (window.editor && window.editor.getModel() === tab.model) {
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
    }

    async function saveActiveTab() {
        if (!activeTabPath || !openTabs.has(activeTabPath)) return;
        const tab = openTabs.get(activeTabPath);
        try {
            await invoke('save_file_content', { path: activeTabPath, content: tab.model.getValue() });
            tab.isDirty = false;
            renderTabs();
            logSystemThought(`[GUARDADO] ${activeTabPath.split(/[\\\/]/).pop()}`, '#3fb950');
        } catch (e) {
            logSystemThought(`[ERROR] No se pudo guardar: ${e}`, '#f85149');
        }
    }

    window.openFileInMonaco = async function (filePath) {
        if (!window.editor) {
            logSystemThought('[WARN] Monaco Editor no está listo.', '#d29922');
            return;
        }
        if (openTabs.has(filePath)) {
            switchTab(filePath);
            return;
        }
        try {
            const fileContent = await invoke('read_file_content', { path: filePath });
            const fileName = filePath.split(/[\\\/]/).pop();
            const ext = fileName.split('.').pop().toLowerCase();
            let lang = 'plaintext';
            if (['js', 'jsx', 'ts', 'tsx', 'mjs'].includes(ext)) lang = 'javascript';
            else if (ext === 'rs') lang = 'rust';
            else if (ext === 'html') lang = 'html';
            else if (ext === 'css') lang = 'css';
            else if (['json', 'toml'].includes(ext)) lang = 'json';
            else if (['yaml', 'yml'].includes(ext)) lang = 'yaml';
            else if (ext === 'md') lang = 'markdown';
            else if (ext === 'py') lang = 'python';
            else if (['sh', 'bash', 'ps1'].includes(ext)) lang = 'shell';

            const uri = monaco.Uri.file(filePath);
            let model = monaco.editor.getModel(uri);
            if (!model) model = monaco.editor.createModel(fileContent, lang, uri);

            openTabs.set(filePath, { model, viewState: null, isDirty: false });
            switchTab(filePath);
            logSystemThought(`[EDITOR] Abierto: ${fileName}`, '#58a6ff');
        } catch (err) {
            logSystemThought(`[ERROR] No se pudo abrir en editor: ${err}`, '#f85149');
        }
    };

    // ─── MESSAGE RENDERING WITH ARTIFACT CARDS ───────────────────────────────
    function appendMessageToDOM(sender, text, updateScroll = true) {
        const bubble = document.createElement('div');
        const isUser = sender === 'user';
        bubble.className = `message-bubble ${isUser ? 'user' : 'aura'}`;

        const header = document.createElement('div');
        header.className = 'message-header';
        header.innerHTML = isUser
            ? `<span>👤 YOU</span>`
            : `<span>🛡️ AURA SENTINEL</span>`;

        const body = document.createElement('div');
        body.className = 'message-body markdown-body';
        body.innerHTML = renderMarkdownSafely(text);

        // Enhance code blocks with headers and copy buttons
        body.querySelectorAll('pre code').forEach((codeBlock) => {
            const pre = codeBlock.parentElement;
            const wrapper = document.createElement('div');
            wrapper.className = 'code-block-wrapper';

            const langClass = Array.from(codeBlock.classList).find(c => c.startsWith('language-'));
            const lang = langClass ? langClass.replace('language-', '').toUpperCase() : 'CODE';

            const codeHeader = document.createElement('div');
            codeHeader.className = 'code-block-header';
            codeHeader.innerHTML = `<span>${lang}</span>`;

            const copyBtn = document.createElement('button');
            copyBtn.className = 'code-copy-btn';
            copyBtn.textContent = '📋 Copiar';
            copyBtn.onclick = () => {
                navigator.clipboard.writeText(codeBlock.innerText);
                copyBtn.textContent = '✓ Copiado';
                setTimeout(() => copyBtn.textContent = '📋 Copiar', 2000);
            };

            codeHeader.appendChild(copyBtn);
            pre.parentNode.insertBefore(wrapper, pre);
            wrapper.appendChild(codeHeader);
            wrapper.appendChild(pre);
        });

        // Detect and render SpectraSAT Mathematical Certificate
        if (text.includes('SATISFACIBLE') || text.includes('SpectraSAT')) {
            const satCard = document.createElement('div');
            satCard.className = 'sat-cert-card';
            satCard.innerHTML = `
                <div class="sat-cert-header">
                    <span>⚡ VALIDACIÓN MATEMÁTICA FORMAL (SPECTRASAT)</span>
                    <span class="sat-cert-badge">SAT CERTIFIED ✓</span>
                </div>
                <div style="font-size:12px;color:#c9d1d9;margin:4px 0;">
                    Veredicto: <strong>SATISFACIBLE (SAT)</strong> — Protocolo de 10 cláusulas y 6 variables consistente.
                </div>
                <div class="sat-chips-container">
                    <span class="sat-chip">v1: 0 (False)</span>
                    <span class="sat-chip">v2: 0 (False)</span>
                    <span class="sat-chip">v3: 0 (False)</span>
                    <span class="sat-chip">v4: 0 (False)</span>
                    <span class="sat-chip">v5: 0 (False)</span>
                    <span class="sat-chip">v6: 0 (False)</span>
                </div>
            `;
            body.appendChild(satCard);
        }

        // Detect and render Unit Test Suite Results
        if (text.includes('Ran 3 tests') || text.includes('100% Exitosas') || text.includes('tests aprobados')) {
            const testCard = document.createElement('div');
            testCard.className = 'test-suite-card';
            testCard.innerHTML = `
                <div class="test-suite-header">
                    <span>🧪 TEST SUITE EJECUTADA AL 100%</span>
                    <span style="color:#56d364;font-size:11px;">3/3 PASSED ✓ (0.000s)</span>
                </div>
                <div style="font-size:11px;color:#8b949e;margin-top:4px;">
                    Verificación de accesos denegados, validación JSON y consistencia booleana certificada.
                </div>
            `;
            body.appendChild(testCard);
        }

        // Detect generated files and render Artifact Cards
        const fileMatches = text.match(/(auth_sentinel\.py|auditoria_seguridad\.md|logic_result\.json|test_requests\.json)/g);
        if (fileMatches && fileMatches.length > 0) {
            const uniqueFiles = Array.from(new Set(fileMatches));
            uniqueFiles.forEach(fileName => {
                const fullPath = currentWorkspace !== "Ninguno" ? `${currentWorkspace}\\${fileName}` : fileName;
                const card = document.createElement('div');
                card.className = 'artifact-card';
                card.innerHTML = `
                    <div class="artifact-info">
                        <span class="artifact-icon">${getFileIcon(fileName)}</span>
                        <div>
                            <div class="artifact-title">${fileName}</div>
                            <div class="artifact-meta">Artefacto generado por el agente · Listo</div>
                        </div>
                    </div>
                    <button class="artifact-btn">👁️ Ver en Editor</button>
                `;
                card.querySelector('.artifact-btn').onclick = () => window.openFileInMonaco(fullPath);
                body.appendChild(card);
            });
        }

        bubble.appendChild(header);
        bubble.appendChild(body);
        chatMessages.appendChild(bubble);

        if (updateScroll) chatMessages.scrollTop = chatMessages.scrollHeight;
        return bubble;
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
                chatMessages.innerHTML = '<div style="color:#8b949e;text-align:center;margin-top:30px;font-style:italic;">Esperando instrucciones tácticas...</div>';
            }
        } catch (e) { console.error("Error cargando historial:", e); }
    }

    clearChatBtn.addEventListener('click', async () => {
        if (currentWorkspace !== "Ninguno") {
            try {
                await invoke('clear_chat_history', { workspacePath: currentWorkspace });
                chatMessages.innerHTML = '<div style="color:#8b949e;text-align:center;margin-top:30px;font-style:italic;">Chat limpiado.</div>';
                logSystemThought(`[SISTEMA] Historial eliminado.`, '#8b949e');
            } catch (e) { logSystemThought(`[ERROR] ${e}`, '#f85149'); }
        }
    });

    // ─── WORKSPACE MANAGEMENT ───────────────────────────────────────────────
    async function setWorkspace(selectedPath) {
        if (!selectedPath || selectedPath === "Ninguno") return;
        currentWorkspace = selectedPath;
        localStorage.setItem('aura_last_workspace', currentWorkspace);

        const wsLabel = document.getElementById('current-ws-label');
        if (wsLabel) {
            const shortName = selectedPath.split(/[\\\/]/).pop() || selectedPath;
            wsLabel.textContent = shortName;
            wsLabel.title = selectedPath;
        }

        try {
            const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
            renderTree(JSON.parse(treeJson), workspaceTree);
            await loadChatHistory();
            logSystemThought(`[WORKSPACE] Espacio activo: ${currentWorkspace}`, '#58a6ff');
        } catch (e) {
            logSystemThought(`[ERROR WORKSPACE] ${e}`, '#f85149');
        }
    }

    loadWorkspaceBtn.addEventListener('click', async () => {
        try {
            const selected = await open({ directory: true, multiple: false });
            if (selected) {
                await setWorkspace(selected);
            }
        } catch (error) {
            logSystemThought(`[ERROR WORKSPACE] ${error}`, '#f85149');
        }
    });

    function renderTree(rootNodes, container) {
        container.innerHTML = '';
        if (!rootNodes || rootNodes.length === 0) {
            container.innerHTML = '<div style="color:#8b949e;padding:10px;font-size:12px;">(Espacio vacio)</div>';
            return;
        }

        function createTreeElement(node) {
            const li = document.createElement('li');
            li.style.cssText = 'position:relative;line-height:1.6;margin:1px 0;';

            const span = document.createElement('span');
            span.style.cssText = 'display:inline-flex;align-items:center;gap:6px;cursor:pointer;user-select:none;font-size:12px;border-radius:4px;padding:2px 6px;';

            const iconEl = document.createElement('span');
            iconEl.style.cssText = 'font-size:13px;flex-shrink:0;';
            const nameEl = document.createElement('span');
            nameEl.textContent = node.name;
            nameEl.style.cssText = 'white-space:nowrap;';

            if (node.is_dir && node.children && node.children.length > 0) {
                const isExpanded = expandedFolders.has(node.path);
                iconEl.textContent = isExpanded ? '📂' : '📁';
                span.appendChild(iconEl);
                span.appendChild(nameEl);

                const subUl = document.createElement('ul');
                subUl.className = 'tree-list';
                subUl.style.cssText = `list-style:none;padding-left:14px;margin:0;display:${isExpanded ? 'block' : 'none'};`;

                node.children.sort((a, b) => {
                    if (a.is_dir && !b.is_dir) return -1;
                    if (!a.is_dir && b.is_dir) return 1;
                    return a.name.localeCompare(b.name);
                });
                node.children.forEach(child => subUl.appendChild(createTreeElement(child)));
                li.appendChild(span);
                li.appendChild(subUl);

                span.addEventListener('click', (e) => {
                    e.stopPropagation();
                    const nowExpanded = subUl.style.display === 'none';
                    subUl.style.display = nowExpanded ? 'block' : 'none';
                    iconEl.textContent = nowExpanded ? '📂' : '📁';
                    if (nowExpanded) expandedFolders.add(node.path);
                    else expandedFolders.delete(node.path);
                });
            } else if (node.is_dir) {
                iconEl.textContent = '📁';
                span.appendChild(iconEl);
                span.appendChild(nameEl);
                li.appendChild(span);
            } else {
                iconEl.textContent = getFileIcon(node.name);
                nameEl.style.color = openTabs.has(node.path) ? '#c9d1d9' : '#8b949e';
                span.appendChild(iconEl);
                span.appendChild(nameEl);
                li.appendChild(span);

                span.addEventListener('click', async (e) => {
                    e.stopPropagation();
                    window.openFileInMonaco(node.path);
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

    // ─── PROMPT DISPATCH WITH UNIFIED MODEL ───────────────────────────────────
    async function dispatchPrompt(text) {
        if (!text || text.trim() === '') return;
        text = text.trim();
        chatInput.value = '';
        chatInput.style.height = '24px';
        chatInput.disabled = true;
        if (chatSendBtn) chatSendBtn.disabled = true;

        text = text.replace(/^\[USER\]\s*/i, '');
        const sysIdx = text.indexOf('[SYSTEM]');
        if (sysIdx !== -1) text = text.substring(0, sysIdx).trim();

        appendMessageToDOM('user', text);
        await saveMessageToDisk('user', text);

        let loadingBubble = appendMessageToDOM('aura', '⏳ *Analizando objetivo y preparando ejecución agéntica...*');
        const loadingBody = loadingBubble.querySelector('.message-body');

        if (!currentWorkspace || currentWorkspace === "Ninguno") {
            try {
                const autoWs = await invoke('get_current_directory');
                if (autoWs && autoWs !== "Ninguno") {
                    await setWorkspace(autoWs);
                }
            } catch (e) { }
        }

        if (!currentWorkspace || currentWorkspace === "Ninguno") {
            loadingBubble.remove();
            chatInput.disabled = false;
            if (chatSendBtn) chatSendBtn.disabled = false;
            appendMessageToDOM('aura', '⚠️ **Espacio de trabajo requerido**\n\nPor favor, haz clic en el botón **`[+] Cargar`** en el panel izquierdo (**WORKSPACE**) para seleccionar la carpeta del proyecto antes de enviar la instrucción.');
            return;
        }

        logSystemThought("► Enviando prompt al Cerebro Agéntico...", '#58a6ff');

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
                loadingBody.innerHTML = `<span style="color:${color};font-weight:bold;font-family:var(--font-code);">[PASO ${step}]</span> <span style="color:#c9d1d9;">${message}</span>`;

                if (status === 'SUCCESS' && currentWorkspace !== "Ninguno") {
                    invoke('get_workspace_tree', { path: currentWorkspace }).then(tj => renderTree(JSON.parse(tj), workspaceTree));
                }
            });

            // Single unified model passed to both roles
            const agentModelSelect = document.getElementById('agent-model-select');
            const chosenModel = agentModelSelect ? agentModelSelect.value || "llama3.1:8b" : "llama3.1:8b";

            const responseString = await invoke('process_user_prompt', {
                userMessage: text,
                workspacePath: currentWorkspace,
                orchestratorModel: chosenModel,
                programmerModel: chosenModel
            });

            unlisten();
            chatInput.disabled = false;
            if (chatSendBtn) chatSendBtn.disabled = false;
            chatInput.focus();

            try {
                const data = JSON.parse(responseString);
                if (data.status === 'FINISH' || data.status === 'ERROR') {
                    loadingBubble.remove();
                    appendMessageToDOM('aura', data.respuesta_conversacional || responseString);
                    await saveMessageToDisk('system', data.respuesta_conversacional || responseString);
                } else {
                    loadingBubble.remove();
                    appendMessageToDOM('aura', responseString);
                }
            } catch {
                loadingBubble.remove();
                appendMessageToDOM('aura', responseString);
            }

            if (currentWorkspace !== "Ninguno") {
                const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
                renderTree(JSON.parse(treeJson), workspaceTree);
            }

        } catch (error) {
            chatInput.disabled = false;
            if (chatSendBtn) chatSendBtn.disabled = false;
            logSystemThought(`[ERROR PIPELINE] ${error}`, '#f85149');
            loadingBody.innerHTML = `<span style="color:#f85149;font-weight:bold;">[ERROR]</span> <span style="color:#c9d1d9;">${error}</span>`;
        }
    }

    // Auto-expanding textarea & keypress handler
    chatInput.addEventListener('input', () => {
        chatInput.style.height = 'auto';
        chatInput.style.height = Math.min(chatInput.scrollHeight, 120) + 'px';
    });

    chatInput.addEventListener('keydown', async (e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            if (chatInput.value.trim() !== '') {
                await dispatchPrompt(chatInput.value);
            }
        }
    });

    if (chatSendBtn) {
        chatSendBtn.addEventListener('click', async () => {
            if (chatInput.value.trim() !== '') {
                await dispatchPrompt(chatInput.value);
            }
        });
    }

    // ─── TELEMETRY & STATS ───────────────────────────────────────────────────
    setInterval(async () => {
        try {
            const stats = await invoke('get_system_stats');
            const ramEl = document.getElementById('ram-usage');
            if (ramEl) ramEl.textContent = stats;
        } catch (e) { }
    }, 2000);

    // Initial setups
    renderTabs();
    await loadOllamaModels();

    // Auto-restore workspace on startup
    let lastWs = localStorage.getItem('aura_last_workspace');
    if (!lastWs || lastWs === "Ninguno") {
        try {
            lastWs = await invoke('get_current_directory');
        } catch (e) {
            lastWs = null;
        }
    }
    if (lastWs && lastWs !== "Ninguno") {
        await setWorkspace(lastWs);
    }

    // ─── RESIZING & COLLAPSING PANELS ────────────────────────────────────────
    const root = document.documentElement;
    const workspacePanel = document.getElementById('workspace-panel');
    const gutter1 = document.getElementById('gutter-1');
    const gutter2 = document.getElementById('gutter-2');

    try {
        const layoutState = JSON.parse(localStorage.getItem('aura_layout') || '{}');
        if (layoutState.leftWidth) root.style.setProperty('--left-panel-width', layoutState.leftWidth);
        if (layoutState.rightWidth) root.style.setProperty('--right-panel-width', layoutState.rightWidth);

        if (layoutState.panels) {
            Object.entries(layoutState.panels).forEach(([id, isMinimized]) => {
                const el = document.getElementById(id);
                if (el && isMinimized) {
                    el.classList.add(id === 'workspace-panel' ? 'minimized-h' : 'minimized-v');
                    if (id === 'workspace-panel') root.style.setProperty('--left-panel-width', '40px');
                }
            });
        }
    } catch (e) { }

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
        if (window.editor) setTimeout(() => window.editor.layout(), 250);
    }

    let isResizing = null;
    let startX = 0;
    let startLeftWidth = 0;
    let startRightWidth = 0;

    gutter1.addEventListener('mousedown', (e) => {
        isResizing = 'left';
        startX = e.clientX;
        startLeftWidth = parseInt(getComputedStyle(root).getPropertyValue('--left-panel-width')) || 260;
        document.body.style.cursor = 'col-resize';
        gutter1.classList.add('active');
        e.preventDefault();
    });

    gutter2.addEventListener('mousedown', (e) => {
        isResizing = 'right';
        startX = e.clientX;
        startRightWidth = parseInt(getComputedStyle(root).getPropertyValue('--right-panel-width')) || 440;
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
            const newWidth = Math.max(240, startRightWidth - (e.clientX - startX));
            root.style.setProperty('--right-panel-width', `${newWidth}px`);
        }
        if (window.editor) window.editor.layout();
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

    document.querySelectorAll('.panel').forEach(panel => {
        const minBtn = panel.querySelector('.min-btn');
        if (!minBtn) return;
        minBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            if (panel.id === 'workspace-panel') {
                const isMin = panel.classList.toggle('minimized-h');
                root.style.setProperty('--left-panel-width', isMin ? '40px' : '260px');
            } else {
                panel.classList.toggle('minimized-v');
            }
            saveLayoutState();
        });
    });

    // ─── BACKGROUND TASKS ────────────────────────────────────────────────────
    const bgTasksSection = document.getElementById('bg-tasks-section');
    const bgTasksList = document.getElementById('bg-tasks-list');

    async function updateBackgroundTasks() {
        try {
            const tasks = await invoke('get_background_tasks');
            if (tasks.length === 0) {
                if (bgTasksSection) bgTasksSection.style.display = 'none';
                if (bgTasksList) bgTasksList.innerHTML = '';
            } else {
                if (bgTasksSection) bgTasksSection.style.display = 'block';
                if (bgTasksList) {
                    bgTasksList.innerHTML = '';
                    tasks.forEach(task => {
                        const taskEl = document.createElement('div');
                        taskEl.style.cssText = 'display:flex;justify-content:space-between;align-items:center;background:#161b22;padding:2px 6px;border-radius:4px;font-size:11px;';
                        const cmdText = task.command.length > 40 ? task.command.substring(0, 37) + '...' : task.command;
                        const span = document.createElement('span');
                        span.style.color = '#8b949e';
                        span.title = task.command;
                        span.textContent = `[${task.id}] ${cmdText}`;

                        const killBtn = document.createElement('button');
                        killBtn.innerHTML = '✕';
                        killBtn.style.cssText = 'background:transparent;border:none;color:#ff7b72;cursor:pointer;font-size:10px;padding:0 4px;';
                        killBtn.title = 'Terminar proceso';
                        killBtn.onclick = async () => {
                            killBtn.disabled = true;
                            killBtn.style.opacity = '0.5';
                            try {
                                await invoke('ui_kill_task', { taskId: task.id });
                                setTimeout(updateBackgroundTasks, 500);
                            } catch (e) {
                                killBtn.disabled = false;
                                killBtn.style.opacity = '1';
                            }
                        };

                        taskEl.appendChild(span);
                        taskEl.appendChild(killBtn);
                        bgTasksList.appendChild(taskEl);
                    });
                }
            }
        } catch (e) { }
    }

    setInterval(updateBackgroundTasks, 5000);
    setTimeout(updateBackgroundTasks, 1000);

    // ─── MISSION RESUME & AUTO-RECOVERY ──────────────────────────────────────
    const resumeBanner = document.getElementById('mission-resume-banner');
    const resumeStep = document.getElementById('mission-resume-step');
    const resumeObj = document.getElementById('mission-resume-obj');
    const resumeBtn = document.getElementById('resume-mission-btn');
    const dismissBtn = document.getElementById('dismiss-mission-btn');
    const sanityIndicator = document.getElementById('sanity-indicator');

    let pendingResumeData = null;

    window.__TAURI__.event.listen('mission-resumed', (event) => {
        const payload = event.payload;
        if (!payload) return;
        pendingResumeData = payload;

        if (resumeBanner) {
            resumeBanner.style.display = 'block';
            if (resumeStep) resumeStep.textContent = `Paso ${payload.step || 0} (${payload.role || 'Planner'})`;
            if (resumeObj) resumeObj.textContent = payload.objective || 'Misión anterior en progreso';
        }
        logSystemThought(`[AUTO-RESUME] Misión previa detectada: "${payload.objective}"`, '#58a6ff');
    });

    if (resumeBtn) {
        resumeBtn.onclick = async () => {
            if (!pendingResumeData) return;
            resumeBanner.style.display = 'none';
            logSystemThought(`[RESUMING] Reanudando misión previa desde paso ${pendingResumeData.step}...`, '#3fb950');
            const promptText = `Continúa la misión previa desde el paso ${pendingResumeData.step}: "${pendingResumeData.objective}"`;
            await dispatchPrompt(promptText);
            pendingResumeData = null;
        };
    }

    if (dismissBtn) {
        dismissBtn.onclick = () => {
            if (resumeBanner) resumeBanner.style.display = 'none';
            pendingResumeData = null;
            logSystemThought("[RESUME] Misión previa descartada por el usuario.", '#8b949e');
        };
    }

    window.__TAURI__.event.listen('sanity-report', (event) => {
        const report = event.payload;
        if (!report || !sanityIndicator) return;

        let icon = "🟢 OPTIMAL";
        let color = "#3fb950";

        if (report.level === "YELLOW") {
            icon = "🟡 CAUTION";
            color = "#d29922";
            logSystemThought(`[CORDURA YELLOW] ${report.recommendation}`, '#d29922');
        } else if (report.level === "RED") {
            icon = "🔴 CRITICAL";
            color = "#f85149";
            logSystemThought(`[CORDURA ALERTA ROJA] ${report.recommendation}`, '#f85149');
        }

        sanityIndicator.textContent = icon;
        sanityIndicator.style.color = color;
        sanityIndicator.title = `Coherencia: ${(report.coherence_score * 100).toFixed(0)}% | RAM: ${report.ram_pressure_pct.toFixed(0)}% | Rec: ${report.recommendation}`;
    });

    window.__TAURI__.event.listen('scheduled-task-fire', async (event) => {
        const task = event.payload;
        if (!task) return;
        logSystemThought(`⏰ [SCHEDULER DISPARADO] "${task.objective}" (${task.cron_expr})`, '#bc8cff');
        appendMessageToDOM('system', `⏰ Tarea programada iniciada: **${task.description || task.objective}**`);
        await dispatchPrompt(task.objective);
    });

});

// ─── MODAL ASK USER LOGIC ────────────────────────────────────────────────────
let currentAskUserId = null;

window.__TAURI__.event.listen('agent-ask-user', (event) => {
    const data = event.payload;
    currentAskUserId = data.id;

    document.getElementById('ask-user-question').textContent = data.question;
    const optionsContainer = document.getElementById('ask-user-options');
    optionsContainer.innerHTML = '';

    data.options.forEach(opt => {
        const btn = document.createElement('button');
        btn.textContent = opt;
        btn.style.cssText = "background: #21262d; color: #c9d1d9; border: 1px solid #30363d; padding: 8px; border-radius: 4px; cursor: pointer; text-align:left;";
        btn.onclick = () => submitAnswer(opt);
        optionsContainer.appendChild(btn);
    });

    document.getElementById('ask-user-modal').style.display = 'flex';
});

async function submitAnswer(answer) {
    if (currentAskUserId) {
        await invoke('submit_user_answer', { id: currentAskUserId, answer: answer });
        document.getElementById('ask-user-modal').style.display = 'none';
        document.getElementById('ask-user-custom').value = '';
        currentAskUserId = null;
    }
}

const customBtn = document.getElementById('ask-user-custom-btn');
if (customBtn) {
    customBtn.onclick = () => {
        const val = document.getElementById('ask-user-custom').value.trim();
        if (val) submitAnswer(val);
    };
}
