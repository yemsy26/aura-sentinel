#!/bin/bash
# Aura-Sentinel Setup Script for macOS/Linux

echo "🛡️ Bienvenido a la configuración de Aura-Sentinel"
echo "==============================================="

# Verificando Node.js
if ! command -v node &> /dev/null; then
    echo "❌ Node.js no está instalado. Por favor instala Node.js (https://nodejs.org/) y reintenta."
    exit 1
else
    echo "✅ Node.js detectado: $(node -v)"
fi

# Verificando Rust
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust no está instalado. Por favor instala Rust (https://rustup.rs/) usando: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
else
    echo "✅ Rust/Cargo detectado: $(cargo -V)"
fi

# Verificando Ollama
if ! command -v ollama &> /dev/null; then
    echo "❌ Ollama no está instalado. Por favor instala Ollama (https://ollama.com/) y reintenta."
    exit 1
else
    echo "✅ Ollama detectado. Descargando modelos requeridos (esto puede tomar varios minutos dependiendo de tu conexión)..."
    ollama pull llama3.1:8b
    ollama pull qwen2.5-coder:7b
    ollama pull nomic-embed-text
    echo "✅ Modelos listos."
fi

echo "==============================================="
echo "Instalando dependencias del proyecto..."
npm install

echo "🎉 ¡Todo listo! Puedes arrancar Aura-Sentinel ejecutando: npm run tauri dev"
