use std::path::Path;
use tokio::fs;
use tauri;

#[allow(dead_code)] // Severity::Info reservado para uso futuro

/// Validador automático de proyectos web
#[derive(Default)]
pub struct AutoValidator {
    workspace_path: String,
}

impl AutoValidator {
    pub fn new(workspace_path: &str) -> Self {
        Self {
            workspace_path: workspace_path.to_string(),
        }
    }

    /// Ejecuta todas las validaciones y auto-fixes
    pub async fn validate_and_fix(&self) -> ValidationResult {
        let mut result = ValidationResult {
            passed: true,
            issues: Vec::new(),
            auto_fixed: Vec::new(),
        };

        // 1. Validar HTML
        self.validate_html(&mut result).await;

        // 2. Validar JS
        self.validate_js(&mut result).await;

        // 3. Validar assets referenciados
        self.validate_assets(&mut result).await;

        // 4. Validar runners
        self.validate_runners(&mut result).await;

        // 5. Aplicar auto-fixes
        self.apply_auto_fixes(&mut result).await;

        result.passed = result.issues.iter().all(|i| i.severity != Severity::Error);
        result
    }

    /// Valida archivos HTML
    async fn validate_html(&self, result: &mut ValidationResult) {
        let html_files = self.find_files("*.html").await;
        
        for html_file in html_files {
            if let Ok(content) = fs::read_to_string(&html_file).await {
                let path_str = html_file.to_string_lossy().to_string();
                
                // Buscar scripts
                for (line_num, line) in content.lines().enumerate() {
                    if line.contains("<script") && line.contains("src=") {
                        if let Some(start) = line.find("src=\"") {
                            let rest = &line[start + 5..];
                            if let Some(end) = rest.find('"') {
                                let src = &rest[..end];
                                self.validate_script_src(src, &path_str, line_num + 1, result);
                            }
                        } else if let Some(start) = line.find("src='") {
                            let rest = &line[start + 5..];
                            if let Some(end) = rest.find('\'') {
                                let src = &rest[..end];
                                self.validate_script_src(src, &path_str, line_num + 1, result);
                            }
                        }
                    }
                }
            }
        }
    }

    fn validate_script_src(&self, src: &str, html_file: &str, line: usize, result: &mut ValidationResult) {
        // Ignorar URLs externas
        if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("//") {
            return;
        }

        // Resolver ruta relativa
        let html_path = Path::new(html_file);
        let base = html_path.parent().unwrap_or(Path::new("."));
        let script_path = base.join(src);

        if !script_path.exists() {
            result.issues.push(ValidationIssue {
                severity: Severity::Error,
                file: html_file.to_string(),
                line: Some(line),
                message: format!("Script referenciado no existe: '{}' (resuelto a: {})", src, script_path.display()),
                suggested_fix: Some(format!("Verificar ruta o crear archivo: {}", script_path.display())),
            });
            result.passed = false;
        }
    }

    /// Valida archivos JS/TS
    async fn validate_js(&self, result: &mut ValidationResult) {
        let js_files = self.find_files("*.js").await;
        
        for js_file in js_files {
            if let Ok(content) = fs::read_to_string(&js_file).await {
                let path_str = js_file.to_string_lossy().to_string();
                
                for (line_num, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    
                    // this.load.image('key', 'path')
                    if trimmed.contains("this.load.image") || trimmed.contains("this.load.audio") || trimmed.contains("this.load.atlas") || trimmed.contains("this.load.spritesheet") {
                        if let Some(start) = line.find("'") {
                            let rest = &line[start + 1..];
                            if let Some(end) = rest.find("'") {
                                let asset_path = &rest[..end];
                                self.validate_asset_path(asset_path, &path_str, line_num + 1, result);
                            }
                        } else if let Some(start) = line.find("\"") {
                            let rest = &line[start + 1..];
                            if let Some(end) = rest.find("\"") {
                                let asset_path = &rest[..end];
                                self.validate_asset_path(asset_path, &path_str, line_num + 1, result);
                            }
                        }
                    }
                }
            }
        }
    }

    fn validate_asset_path(&self, asset_path: &str, js_file: &str, line: usize, result: &mut ValidationResult) {
        // Ignorar URLs y data URIs
        if asset_path.starts_with("http://") || asset_path.starts_with("https://") || asset_path.starts_with("data:") {
            return;
        }

        // Resolver ruta
        let js_path = Path::new(js_file);
        let base = js_path.parent().unwrap_or(Path::new("."));
        let asset_full = base.join(asset_path);

        if !asset_full.exists() {
            result.issues.push(ValidationIssue {
                severity: Severity::Error,
                file: js_file.to_string(),
                line: Some(line),
                message: format!("Asset referenciado no existe: '{}' (resuelto a: {})", asset_path, asset_full.display()),
                suggested_fix: Some(format!("Crear asset o corregir ruta: {}", asset_full.display())),
            });
            result.passed = false;
        }
    }

    /// Valida assets referenciados en HTML/CSS
    async fn validate_assets(&self, result: &mut ValidationResult) {
        // Verificar carpeta assets/
        let assets_dir = Path::new(&self.workspace_path).join("assets");
        let src_assets = Path::new(&self.workspace_path).join("src/assets");
        
        if !assets_dir.exists() && !src_assets.exists() {
            result.issues.push(ValidationIssue {
                severity: Severity::Warning,
                file: "workspace".to_string(),
                line: None,
                message: "No se encontró carpeta de assets (assets/ o src/assets/)".to_string(),
                suggested_fix: Some("Crear carpeta assets/ y añadir assets, o usar gráficos programáticos".to_string()),
            });
        }
    }

    /// Valida runners de prueba
    async fn validate_runners(&self, result: &mut ValidationResult) {
        let runners = ["run_tests.bat", "run_tests.ps1", "run_game.bat", "run_game.ps1", "dev.sh", "dev.bat"];
        
        let mut found = false;
        for runner in &runners {
            if Path::new(&self.workspace_path).join(runner).exists() {
                found = true;
                break;
            }
        }
        
        if !found {
            result.issues.push(ValidationIssue {
                severity: Severity::Warning,
                file: "workspace".to_string(),
                line: None,
                message: "No se encontraron scripts de ejecución (run_tests.bat, run_game.ps1, etc.)".to_string(),
                suggested_fix: Some("Generar runners con TOOL_CREATE_RUNNER".to_string()),
            });
        }
    }

    /// Aplica auto-fixes
    async fn apply_auto_fixes(&self, result: &mut ValidationResult) {
        for issue in &result.issues {
            if let Some(fix) = &issue.suggested_fix {
                // Auto-fix: generar assets programáticos si faltan
                if fix.contains("gráficos programáticos") || fix.contains("assets programáticos") {
                    if let Err(e) = self.generate_programmatic_assets().await {
                        eprintln!("[AUTO-FIX] Error generando assets: {}", e);
                    } else {
                        result.auto_fixed.push("Generados assets programáticos en main.js".to_string());
                    }
                }
                
                // Auto-fix: generar runners
                if fix.contains("TOOL_CREATE_RUNNER") || fix.contains("Generar runners") {
                    // Se ejecuta TOOL_CREATE_RUNNER en el agente
                    result.auto_fixed.push("Runners generados (pendiente ejecución de herramienta)".to_string());
                }
            }
        }
    }

    /// Genera assets programáticos en main.js
    async fn generate_programmatic_assets(&self) -> Result<(), String> {
        let main_js_path = Path::new(&self.workspace_path).join("src/main.js");
        if !main_js_path.exists() {
            return Err("main.js no encontrado".to_string());
        }
        
        let content = fs::read_to_string(&main_js_path).await.map_err(|e| e.to_string())?;
        
        if content.contains("createAssets") || content.contains("generateTexture") {
            return Ok(()); // Ya tiene generación programática
        }

        // Buscar BootScene o Scene principal
        let mut modified = content;
        
        if modified.contains("class BootScene") || modified.contains("class GameScene") {
            // Buscar preload() en BootScene
            if let Some(idx) = modified.find("preload()") {
                let insert_pos = idx + "preload()".len();
                let injection = r#"
        // Auto-generación de assets programáticos
        this.createAssets();
    }
    
    createAssets() {
        // Nave jugador
        const playerGraphics = this.make.graphics({ x: 0, y: 0, add: false });
        playerGraphics.fillStyle(0x00ffff, 1);
        playerGraphics.fillTriangle(16, 0, 0, 32, 32, 32);
        playerGraphics.generateTexture('player', 32, 32);
        playerGraphics.destroy();
        
        // Nave transformada
        const gunshipGraphics = this.make.graphics({ x: 0, y: 0, add: false });
        gunshipGraphics.fillStyle(0xff00ff, 1);
        gunshipGraphics.fillTriangle(20, 0, 0, 40, 40, 40);
        gunshipGraphics.generateTexture('gunship', 40, 40);
        gunshipGraphics.destroy();
        
        // Balas
        const bulletGraphics = this.make.graphics({ x: 0, y: 0, add: false });
        bulletGraphics.fillStyle(0xffff00, 1);
        bulletGraphics.fillRect(0, 0, 4, 12);
        bulletGraphics.generateTexture('bullet', 4, 12);
        bulletGraphics.destroy();
        
        // Enemigos
        const enemyGraphics = this.make.graphics({ x: 0, y: 0, add: false });
        enemyGraphics.fillStyle(0xff3333, 1);
        enemyGraphics.fillTriangle(16, 0, 0, 32, 32, 32);
        enemyGraphics.generateTexture('enemy', 32, 32);
        enemyGraphics.destroy();
        
        // Power-ups
        const puGraphics = this.make.graphics({ x: 0, y: 0, add: false });
        puGraphics.fillStyle(0xffff00, 1);
        puGraphics.fillCircle(16, 16, 16);
        puGraphics.generateTexture('powerup_transform', 32, 32);
        puGraphics.destroy();
        
        // Partículas
        const particle = this.make.graphics({ x: 0, y: 0, add: false });
        particle.fillStyle(0xff8800, 1);
        particle.fillCircle(4, 4, 4);
        particle.generateTexture('particle', 8, 8);
        particle.destroy();
    "#;
                
                modified = format!("{}{}\n{}", &modified[..insert_pos + "preload()".len()], injection, &modified[insert_pos + "preload()".len()..]);
                fs::write(&Path::new(&self.workspace_path).join("src/main.js"), modified).await.map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        
        Err("No se encontró lugar para inyectar generación de assets".to_string())
    }

    /// Busca archivos por patrón
    async fn find_files(&self, pattern: &str) -> Vec<std::path::PathBuf> {
        let mut results = Vec::new();
        self.find_files_recursive(Path::new(&self.workspace_path), pattern, &mut results, 0).await;
        results
    }

    fn find_files_recursive<'a>(&'a self, dir: &'a Path, pattern: &'a str, results: &'a mut Vec<std::path::PathBuf>, depth: u32) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if depth > 5 { return; }
            if let Ok(entries) = fs::read_dir(dir).await {
                let mut entries = entries;
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let name_str = name.as_ref();
                    
                    // Saltar directorios ignorados
                    if path.is_dir() {
                        if matches!(name_str, "node_modules" | ".git" | "target" | "dist" | "build" | ".next" | ".nuxt") {
                            continue;
                        }
                        self.find_files_recursive(&path, pattern, results, depth + 1).await;
                    } else if self.match_pattern(name_str, pattern) {
                        results.push(path);
                    }
                }
            }
        })
    }

    fn match_pattern(&self, name: &str, pattern: &str) -> bool {
        if pattern == "*" { return true; }
        if pattern.starts_with("*.") {
            let ext = &pattern[1..];
            return name.ends_with(ext);
        }
        name == pattern
    }
}

/// Resultado de validación
#[derive(Debug, Default)]
pub struct ValidationResult {
    pub passed: bool,
    pub issues: Vec<ValidationIssue>,
    pub auto_fixed: Vec<String>,
}

/// Problema detectado en validación
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Comando Tauri para ejecutar auto-validación del proyecto
#[tauri::command]
pub async fn run_auto_validation(workspace_path: String) -> Result<String, String> {
    let validator = AutoValidator::new(&workspace_path);
    let result = validator.validate_and_fix().await;
    
    let mut output = String::new();
    output.push_str("📊 REPORTE DE AUTO-VALIDACIÓN\n");
    output.push_str("==========================\n");
    output.push_str(&format!("Estado: {}\n", if result.passed { "✅ ÉXITO" } else { "❌ FALLÓ" }));
    output.push_str(&format!("Errores: {}\n", result.issues.iter().filter(|i| i.severity == Severity::Error).count()));
    output.push_str(&format!("Advertencias: {}\n", result.issues.iter().filter(|i| i.severity == Severity::Warning).count()));
    output.push_str(&format!("Auto-fixes aplicados: {}\n\n", result.auto_fixed.len()));
    
    for issue in &result.issues {
        let icon = match issue.severity {
            Severity::Error => "❌",
            Severity::Warning => "⚠️",
            Severity::Info => "ℹ️",
        };
        output.push_str(&format!("{} [{}] {} (línea: {:?})\n", icon, issue.file, issue.message, issue.line));
        if let Some(fix) = &issue.suggested_fix {
            output.push_str(&format!("   🔧 Fix: {}\n", fix));
        }
    }
    
    if !result.auto_fixed.is_empty() {
        output.push_str("\n🔧 AUTO-FIXES APLICADOS:\n");
        for fix in &result.auto_fixed {
            output.push_str(&format!("  ✅ {}\n", fix));
        }
    }
    
    if result.passed {
        Ok(format!("{}\n\n✅ VALIDACIÓN EXITOSA", output))
    } else {
        Ok(format!("{}\n\n❌ VALIDACIÓN FALLÓ - Revisar errores arriba", output))
    }
}