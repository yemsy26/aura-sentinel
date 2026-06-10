const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;
document.addEventListener('DOMContentLoaded', async () => {
    const chatInput = document.getElementById('chat-input');
    const chatMessages = document.getElementById('chat-messages');
    const systemThoughts = document.getElementById('system-thoughts');
    const loadWorkspaceBtn = document.getElementById('load-workspace-btn');
    const workspaceTree = document.getElementById('workspace-tree');
    const clearChatBtn = document.getElementById('clear-chat-btn');

    let currentWorkspace = "Ninguno";
    let expandedFolders = new Set();

    function logSystemThought(message, color = '#00ff00') {
        const p = document.createElement('p');
        const now = new Date();
        const timeStr = now.toTimeString().split(' ')[0]; 
        p.textContent = `[${timeStr}] - ${message}`;
        p.style.color = color;
        systemThoughts.appendChild(p);
        systemThoughts.scrollTop = systemThoughts.scrollHeight;
    }

    function appendMessageToDOM(sender, text, updateScroll = true) {
        const div = document.createElement('div');
        div.style.marginBottom = '10px';
        div.innerHTML = `<span style="color: ${sender === 'user' ? '#79c0ff' : '#a5d6ff'}">[${sender.toUpperCase()}]</span> ${text}`;
        chatMessages.appendChild(div);
        if (updateScroll) {
            chatMessages.scrollTop = chatMessages.scrollHeight;
        }
        return div;
    }

    async function saveMessageToDisk(sender, text) {
        const timestamp = new Date().toISOString();
        try {
            await invoke('save_chat_message', { 
                workspacePath: currentWorkspace, 
                message: { sender, text, timestamp } 
            });
        } catch (e) {
            console.error("Error guardando chat", e);
        }
    }

    async function loadChatHistory() {
        chatMessages.innerHTML = '';
        try {
            const chatJson = await invoke('load_chat_history', { workspacePath: currentWorkspace });
            const chatLogs = JSON.parse(chatJson);
            
            if (chatLogs.length > 0) {
                chatLogs.forEach(msg => {
                    appendMessageToDOM(msg.sender, msg.text, false);
                });
                chatMessages.scrollTop = chatMessages.scrollHeight;
            } else {
                chatMessages.innerHTML = '<div style="color: #8b949e; text-align: center; margin-top: 20px;">Esperando instrucciones tácticas...</div>';
            }
        } catch (e) {
            console.error("Error cargando historial de chat", e);
        }
    }

    clearChatBtn.addEventListener('click', async () => {
        if (currentWorkspace !== "Ninguno") {
            try {
                await invoke('clear_chat_history', { workspacePath: currentWorkspace });
                chatMessages.innerHTML = '<div style="color: #8b949e; text-align: center; margin-top: 20px;">Chat limpiado. Esperando nuevas instrucciones tácticas...</div>';
                logSystemThought(`[SISTEMA] Historial de chat eliminado para este workspace.`, '#8b949e');
            } catch (e) {
                logSystemThought(`[ERROR] No se pudo limpiar el chat: ${e}`, '#f85149');
            }
        }
    });

    logSystemThought("AURA-SENTINEL Inicializado. Ejecutando protocolo de auto-carga...");

    // AUTO CARGA DEL WORKSPACE AL INICIAR LA VENTANA
    // Recupera el workspace que el usuario eligió antes (sobrevive reinicios de Tauri)
    try {
        logSystemThought("Obteniendo directorio de ejecución actual...");
        const defaultCwd = await invoke('get_current_directory');
        
        // Si el usuario había seleccionado un workspace antes, lo restauramos
        const savedWorkspace = localStorage.getItem('aura_workspace');
        currentWorkspace = savedWorkspace || defaultCwd;

        if (savedWorkspace) {
            logSystemThought(`[MEMORIA] Workspace restaurado: ${currentWorkspace}`, '#d29922');
        } else {
            logSystemThought(`Workspace fijado automáticamente en: ${currentWorkspace}`);
        }
        
        const initResult = await invoke('init_memory_log', { workspacePath: currentWorkspace });
        logSystemThought(`Memoria: ${initResult}`);

        logSystemThought("Escaneando directorio y estructurando el mapa...");
        const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
        const treeData = JSON.parse(treeJson);
        
        renderTree(treeData, workspaceTree);
        logSystemThought(`Escaneo completado. ${treeData.length} nodos identificados. LISTO PARA OPERAR.`);

        // FASE 10 y 11: MEMORIA VISUAL Y CHAT
        await loadChatHistory();

        const memoryJson = await invoke('read_memory_logs', { workspacePath: currentWorkspace });
        const memoryLogs = JSON.parse(memoryJson);

        if (memoryLogs.length > 0) {
            memoryLogs.forEach(log => {
                logSystemThought(`[HISTORIAL RECUPERADO] ${log.timestamp} - Archivo: ${log.file_path} | Estado: ${log.compilation_status}`, '#d29922');
            });
        } else {
            logSystemThought(`[SISTEMA] Proyecto nuevo. No hay registros de memoria anteriores.`, '#8b949e');
        }
    } catch (error) {
        logSystemThought(`[ERROR FATAL DE AUTO-CARGA]: ${error}`, '#f85149');
    }

    // CARGA MANUAL DE WORKSPACE
    loadWorkspaceBtn.addEventListener('click', async () => {
        try {
            logSystemThought("Abriendo selector de directorios de Windows...");
            const selectedPath = await open({ directory: true, multiple: false });

            if (selectedPath) {
                currentWorkspace = selectedPath;
                
                // Guardamos el workspace elegido en localStorage para que persista en reinicios
                localStorage.setItem('aura_workspace', currentWorkspace);
                
                // Limpiar consola para independizar contextos
                systemThoughts.innerHTML = '';
                logSystemThought(`[SISTEMA] Workspace cambiado a: ${currentWorkspace}`);
                logSystemThought(`[MEMORIA] Workspace guardado. Se restaurará automáticamente al reiniciar.`, '#3fb950');
                
                const initResult = await invoke('init_memory_log', { workspacePath: currentWorkspace });
                logSystemThought(`Memoria: ${initResult}`);

                logSystemThought("Escaneando directorio y estructurando el mapa...");
                const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
                const treeData = JSON.parse(treeJson);
                
                renderTree(treeData, workspaceTree);
                logSystemThought(`Escaneo completado. ${treeData.length} nodos identificados.`);

                // FASE 10 y 11: MEMORIA VISUAL Y CHAT
                await loadChatHistory();

                const memoryJson = await invoke('read_memory_logs', { workspacePath: currentWorkspace });
                const memoryLogs = JSON.parse(memoryJson);

                if (memoryLogs.length > 0) {
                    memoryLogs.forEach(log => {
                        logSystemThought(`[HISTORIAL RECUPERADO] ${log.timestamp} - Archivo: ${log.file_path} | Estado: ${log.compilation_status}`, '#d29922');
                    });
                } else {
                    logSystemThought(`[SISTEMA] Proyecto nuevo. No hay registros de memoria anteriores.`, '#8b949e');
                }
            }
        } catch (error) {
            logSystemThought(`[ERROR FATAL]: ${error}`, '#f85149');
        }
    });

    function renderTree(nodes, container) {
        // Guardamos el estado actual antes de limpiar
        const currentExpanded = new Set();
        container.querySelectorAll('ul.tree-list ul').forEach(ul => {
            if (ul.style.display === 'block') {
                const parentSpan = ul.previousElementSibling;
                if (parentSpan) {
                    currentExpanded.add(parentSpan.textContent.trim());
                }
            }
        });
        
        // Si ya teníamos carpetas expandidas en memoria, las combinamos
        expandedFolders.forEach(f => currentExpanded.add(f));
        expandedFolders = currentExpanded;

        container.innerHTML = '';
        
        const nodeMap = {};
        const rootNodes = [];
        
        nodes.forEach(node => {
            node.children = [];
            const normPath = node.path.replace(/\\/g, '/');
            nodeMap[normPath] = node;
        });
        
        nodes.forEach(node => {
            let addedToParent = false;
            if (node.parent_path) {
                const normParent = node.parent_path.replace(/\\/g, '/');
                if (nodeMap[normParent]) {
                    nodeMap[normParent].children.push(node);
                    addedToParent = true;
                }
            }
            if (!addedToParent) {
                rootNodes.push(node);
            }
        });

        function createTreeElement(node) {
            const li = document.createElement('li');
            li.className = node.is_dir ? 'tree-folder' : 'tree-file';
            li.title = node.path;
            
            const span = document.createElement('span');
            const baseText = (node.is_dir ? '📁 ' : '📄 ') + node.name;
            span.textContent = baseText;
            li.appendChild(span);

            if (node.is_dir && node.children.length > 0) {
                const ul = document.createElement('ul');
                const isRoot = rootNodes.includes(node);
                
                // Restaurar estado si estaba expandido, o si es la raíz
                const isExpanded = isRoot || expandedFolders.has('📂 ' + node.name) || expandedFolders.has('📁 ' + node.name);
                
                ul.style.display = isExpanded ? 'block' : 'none';
                ul.style.paddingLeft = '15px';
                ul.style.listStyleType = 'none';
                
                if (isExpanded) {
                    span.textContent = '📂 ' + node.name;
                }
                
                node.children.sort((a, b) => {
                    if (a.is_dir && !b.is_dir) return -1;
                    if (!a.is_dir && b.is_dir) return 1;
                    return a.name.localeCompare(b.name);
                });

                node.children.forEach(child => {
                    ul.appendChild(createTreeElement(child));
                });
                
                li.appendChild(ul);
                
                span.style.cursor = 'pointer';
                span.addEventListener('click', (e) => {
                    e.stopPropagation();
                    const willBeExpanded = ul.style.display === 'none';
                    ul.style.display = willBeExpanded ? 'block' : 'none';
                    span.textContent = (willBeExpanded ? '📂 ' : '📁 ') + node.name;
                    
                    // Actualizamos memoria global
                    if (willBeExpanded) {
                        expandedFolders.add(span.textContent.trim());
                    } else {
                        expandedFolders.delete('📂 ' + node.name);
                        expandedFolders.delete('📁 ' + node.name);
                    }
                });
            } else if (node.is_dir) {
                span.style.cursor = 'pointer';
                // Even empty directories should toggle icon on click
                const isExpanded = expandedFolders.has('📂 ' + node.name);
                if (isExpanded) span.textContent = '📂 ' + node.name;
                
                span.addEventListener('click', (e) => {
                    e.stopPropagation();
                    const willBeExpanded = span.textContent.includes('📁');
                    span.textContent = (willBeExpanded ? '📂 ' : '📁 ') + node.name;
                    if (willBeExpanded) {
                        expandedFolders.add(span.textContent.trim());
                    } else {
                        expandedFolders.delete('📂 ' + node.name);
                        expandedFolders.delete('📁 ' + node.name);
                    }
                });
            }
            
            return li;
        }

        const ul = document.createElement('ul');
        ul.className = 'tree-list';
        ul.style.listStyleType = 'none';
        ul.style.paddingLeft = '0';
        
        rootNodes.sort((a, b) => {
            if (a.is_dir && !b.is_dir) return -1;
            if (!a.is_dir && b.is_dir) return 1;
            return a.name.localeCompare(b.name);
        });

        rootNodes.forEach(root => {
            ul.appendChild(createTreeElement(root));
        });

        container.appendChild(ul);
    }

    chatInput.addEventListener('keypress', async (e) => {
        if (e.key === 'Enter' && chatInput.value.trim() !== '') {
            let text = chatInput.value.trim();
            chatInput.value = '';
            
            // Sanitizer anti-copy-paste: Si el usuario pegó el historial completo
            text = text.replace(/^\[USER\]\s*/i, '');
            const systemIndex = text.indexOf('[SYSTEM]');
            if (systemIndex !== -1) {
                text = text.substring(0, systemIndex).trim();
            }
            
            // Reemplazamos appendMessage por las nuevas funciones
            appendMessageToDOM('user', text);
            await saveMessageToDisk('user', text);
            
            // El mensaje de espera no se guarda en disco, es efimero
            let loadingMsg = appendMessageToDOM('system', '⏳ Analizando ruta de acción...');
            
            logSystemThought("Paso 1: Solicitando plan al Orquestador...");

            try {
                // Interceptamos eventos de streaming en vivo desde el Backend (El Agentic Loop)
                const unlisten = await window.__TAURI__.event.listen('agent-step', (event) => {
                    const payload = event.payload;
                    const step = payload.step;
                    const status = payload.status;
                    const message = payload.message;
                    
                    let color = '#a5d6ff'; // PLANNING default
                    if (status === 'DECISION') color = '#e3b341'; // yellow
                    if (status === 'ACTION') color = '#bc8cff'; // purple
                    if (status === 'SUCCESS') color = '#3fb950'; // green
                    if (status === 'ERROR' || status === 'FATAL') color = '#f85149'; // red
                    if (status === 'VALIDATING') color = '#1f6feb'; // blue

                    logSystemThought(`[PASO ${step}] [${status}] ${message}`, color);
                    loadingMsg.innerHTML = `<span style="color: ${color}">[PASO ${step}]</span> ${message}`;
                    
                    // Forzamos refresco del árbol en cualquier SUCCESS
                    if (status === 'SUCCESS') {
                        invoke('get_workspace_tree', { path: currentWorkspace }).then(treeJson => {
                            renderTree(JSON.parse(treeJson), workspaceTree);
                        });
                    }
                });

                const responseString = await invoke('process_user_prompt', { 
                    userMessage: text, 
                    workspacePath: currentWorkspace 
                });
                
                // Limpiamos el listener para evitar duplicados en el próximo turno
                unlisten();
                
                try {
                    const data = JSON.parse(responseString);
                    if (data.status === "FINISH" || data.status === "ERROR") {
                        loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> ${data.respuesta_conversacional}`;
                        await saveMessageToDisk('system', data.respuesta_conversacional);
                    } else {
                        loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> (Formato final desconocido): ${responseString}`;
                    }
                    
                    // Refrescar el árbol al final por si acaso
                    const treeJson = await invoke('get_workspace_tree', { path: currentWorkspace });
                    renderTree(JSON.parse(treeJson), workspaceTree);
                    
                } catch (parseError) {
                    logSystemThought(`[ADVERTENCIA]: La respuesta final del backend no es un JSON válido.`, '#d29922');
                    loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> ${responseString}`;
                }
                
            } catch (error) {
                logSystemThought(`[ERROR PIPELINE]: ${error}`, '#f85149');
                loadingMsg.innerHTML = `<span style="color: #f85149">[ERROR]</span> Falla de sistema: ${error}`;
            }
        }
    });

    setInterval(async () => {
        try {
            const stats = await invoke('get_system_stats');
            document.getElementById('ram-usage').textContent = stats;
        } catch (e) {
            console.error("Error fetching system stats", e);
        }
    }, 2000);
});
