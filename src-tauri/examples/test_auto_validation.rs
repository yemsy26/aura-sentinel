use app_lib::core::auto_validator::AutoValidator;
use app_lib::core::auto_validator::Severity;

#[tokio::main]
async fn main() {
    let workspace = r"C:\Users\yemsy\OneDrive\Documents\carpeta de preuba de aurasentinel\prueba 2";
    
    println!("🔍 Ejecutando auto-validación en: {}", workspace);
    
    let validator = AutoValidator::new(workspace);
    let result = validator.validate_and_fix().await;
    
    println!("\n📊 REPORTE DE AUTO-VALIDACIÓN");
    println!("==========================");
    println!("Estado: {}", if result.passed { "✅ ÉXITO" } else { "❌ FALLÓ" });
    println!("Errores: {}", result.issues.iter().filter(|i| i.severity == Severity::Error).count());
    println!("Advertencias: {}", result.issues.iter().filter(|i| i.severity == Severity::Warning).count());
    println!("Auto-fixes aplicados: {}", result.auto_fixed.len());
    
    for issue in &result.issues {
        let icon = match issue.severity {
            Severity::Error => "❌",
            Severity::Warning => "⚠️",
            Severity::Info => "ℹ️",
        };
        println!("{} [{}] {} (línea: {:?})", 
            icon, issue.file, issue.message, issue.line);
        if let Some(fix) = &issue.suggested_fix {
            println!("   🔧 Fix: {}", fix);
        }
    }
    
    if !result.auto_fixed.is_empty() {
        println!("\n🔧 AUTO-FIXES APLICADOS:");
        for fix in &result.auto_fixed {
            println!("  ✅ {}", fix);
        }
    }
}