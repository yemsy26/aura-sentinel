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
    try {
        logSystemThought("Obteniendo directorio de ejecución actual...");
        const cwd = await invoke('get_current_directory');
        
        currentWorkspace = cwd;
        logSystemThought(`Workspace fijado automáticamente en: ${currentWorkspace}`);
        
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
                
                // Limpiar consola para independizar contextos
                systemThoughts.innerHTML = '';
                logSystemThought(`[SISTEMA] Workspace cambiado a: ${currentWorkspace}`);
                
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
            span.textContent = (node.is_dir ? '📁 ' : '📄 ') + node.name;
            li.appendChild(span);

            if (node.is_dir && node.children.length > 0) {
                const ul = document.createElement('ul');
                const isRoot = rootNodes.includes(node);
                ul.style.display = isRoot ? 'block' : 'none';
                ul.style.paddingLeft = '15px';
                ul.style.listStyleType = 'none';
                
                if (isRoot) {
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
                    ul.style.display = ul.style.display === 'none' ? 'block' : 'none';
                    span.textContent = (ul.style.display === 'none' ? '📁 ' : '📂 ') + node.name;
                });
            } else if (node.is_dir) {
                span.style.cursor = 'pointer';
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
            const text = chatInput.value.trim();
            chatInput.value = '';
            
            // Reemplazamos appendMessage por las nuevas funciones
            appendMessageToDOM('user', text);
            await saveMessageToDisk('user', text);
            
            // El mensaje de espera no se guarda en disco, es efimero
            let loadingMsg = appendMessageToDOM('system', '⏳ Analizando ruta de acción...');
            
            logSystemThought("Paso 1: Solicitando plan al Orquestador...");

            try {
                const responseString = await invoke('process_user_prompt', { 
                    userMessage: text, 
                    workspacePath: currentWorkspace 
                });
                
                try {
                    const data = JSON.parse(responseString);
                    
                    if (data.orquestador) {
                        const orch = data.orquestador;
                        logSystemThought(`[ORQUESTADOR]: ${orch.pensamiento}`);
                        
                        if (orch.respuesta_conversacional) {
                            loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> ${orch.respuesta_conversacional}`;
                            await saveMessageToDisk('system', orch.respuesta_conversacional);
                        } else {
                            loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> Respuesta procesada.`;
                            await saveMessageToDisk('system', "Respuesta procesada.");
                        }

                        if (orch.intencion === "CHAT" || !data.programador || Object.keys(data.programador).length === 0) {
                            logSystemThought(`[MODO CHAT]: Conversación completada sin operaciones en disco.`, '#a5d6ff');
                        } else {
                            const prog = data.programador;
                            
                            if (orch.archivos_a_analizar && orch.archivos_a_analizar.length > 0) {
                                logSystemThought(`[LECTURA SEGURA]: Extrayendo contenidos de: \n  - ${orch.archivos_a_analizar.join('\n  - ')}`);
                            }
                            
                            logSystemThought(`Paso 2: Delegando tarea a ${orch.modelo_sugerido || "qwen2.5-coder:7b"}...`);
                            
                            if (prog.explicacion_tecnica && !orch.respuesta_conversacional) {
                                loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> ${prog.explicacion_tecnica}`;
                                await saveMessageToDisk('system', prog.explicacion_tecnica);
                            }

                            if (prog.cambios) {
                                prog.cambios.forEach(cambio => {
                                    logSystemThought(`\n--- PROPUESTA PARA: ${cambio.archivo} ---`);
                                    logSystemThought(`BUSCAR:\n${cambio.buscar}`);
                                    logSystemThought(`REEMPLAZAR:\n${cambio.reemplazar}`);
                                    logSystemThought(`-----------------------------------------`);
                                });
                            }

                            if (data.operacion_fisica) {
                                logSystemThought(`[OPERACIÓN FÍSICA] ${data.operacion_fisica}`, '#00ffff');
                            }

                            if (data.eventos_validacion && data.eventos_validacion.length > 0) {
                                data.eventos_validacion.forEach(evento => {
                                    let color = '#00ff00';
                                    if (evento.includes('[ERROR DETECTADO]') || evento.includes('[FATAL]')) {
                                        color = '#ff7b72';
                                    } else if (evento.includes('[ÉXITO]')) {
                                        color = '#3fb950';
                                    } else if (evento.includes('[VALIDACIÓN]')) {
                                        color = '#a5d6ff';
                                    }
                                    logSystemThought(evento, color);
                                });
                            }
                        }

                    } else {
                        loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> JSON no reconocido.`;
                    }

                } catch (parseError) {
                    logSystemThought(`[ADVERTENCIA]: La respuesta del backend no es un JSON válido.`, '#d29922');
                    loadingMsg.innerHTML = `<span style="color: #a5d6ff">[SYSTEM]</span> Error parseando respuesta del orquestador.`;
                }
                
            } catch (error) {
                logSystemThought(`[ERROR PIPELINE]: ${error}`, '#f85149');
                loadingMsg.innerHTML = `<span style="color: #f85149">[ERROR]</span> Falla en la comunicación con Ollama: ${error}`;
            }
        }
    });

    setInterval(() => {
        document.getElementById('ram-usage').textContent = (Math.random() * 5 + 15).toFixed(1) + ' MB';
    }, 2000);
});
