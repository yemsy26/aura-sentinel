Write-Host "🛡️ Bienvenido a la configuracion de Aura-Sentinel" -ForegroundColor Cyan
Write-Host "===============================================" -ForegroundColor Cyan

# Verificando Node.js
try {
    $nodeVersion = node -v
    Write-Host "✅ Node.js detectado: $nodeVersion" -ForegroundColor Green
} catch {
    Write-Host "❌ Node.js no esta instalado. Por favor instala Node.js (https://nodejs.org/) y reintenta." -ForegroundColor Red
    exit 1
}

# Verificando Rust
try {
    $cargoVersion = cargo -V
    Write-Host "✅ Rust/Cargo detectado: $cargoVersion" -ForegroundColor Green
} catch {
    Write-Host "❌ Rust no esta instalado. Por favor instala Rust (https://rustup.rs/) y reintenta." -ForegroundColor Red
    exit 1
}

# Verificando Ollama
try {
    $ollamaVersion = ollama -v
    Write-Host "✅ Ollama detectado. Descargando modelos requeridos (esto puede tomar varios minutos dependiendo de tu conexion)..." -ForegroundColor Green
    ollama pull llama3.1:8b
    ollama pull qwen2.5-coder:7b
    ollama pull nomic-embed-text
    Write-Host "✅ Modelos listos." -ForegroundColor Green
} catch {
    Write-Host "❌ Ollama no esta instalado. Por favor instala Ollama (https://ollama.com/) y reintenta." -ForegroundColor Red
    exit 1
}

Write-Host "===============================================" -ForegroundColor Cyan
Write-Host "Instalando dependencias del proyecto..." -ForegroundColor Yellow
npm install

Write-Host "🎉 ¡Todo listo! Puedes arrancar Aura-Sentinel ejecutando: npm run tauri dev" -ForegroundColor Green
