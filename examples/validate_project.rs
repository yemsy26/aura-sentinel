use std::env;
use aura_sentinel::core::auto_validator::AutoValidator;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let workspace = if std::env::args().len() > 1 {
        std::env::args().nth(1).unwrap()
    } else {
        env::current_dir().unwrap().to_string_lossy().to_string()
    };

    println!("🔍 Auto-validando proyecto en: {}", workspace);

    let validator = aura_sentinel::core::auto_validator::AutoValidator::new(&workspace);
    let result = validator.validate_and_fix().await;
    
    println!("{}", format_report(&result));
    
    if result.has_errors() {
        println!("\n❌ VALIDACIÓN FALLÓ");
        std::process::exit(1);
    } else {
        println!("\n✅ VALIDACIÓN EXITOSA");
    }
}

fn format_report(result: &aura_sentinel::core::auto_validator::ValidationResult) -> String {
    let mut out = String::new();
    out.push_str("📊 REPORTE DE AUTO-VALIDACIÓN\n");
    out.push_str("==========================\n");
    out.push_str(&format!("Estado: {}\n", if result.passed { "✅ ÉXITO" } else { "❌ FALLÓ" }));
    out.push_str(&format!("Errores: {}\n", result.issues.iter().filter(|i| i.severity == aura_sentinel::core::auto_validator::Severity::Error).count()));
    out.push_str(&format!("Advertencias: {}\n", result.issues.iter().filter(|i| i.severity == aura_sentinel::core::auto_validator::Severity::Warning).count()));
    out.push_str(&format!("Auto-fixes aplicados: {}\n\n", result.auto_fixed.len()));
    
    for issue in &result.issues {
        let icon = match issue.severity {
            aura_sentinel::core::auto_validator::Severity::Error => "❌",
            aura_sentinel::core::auto_validator::Severity::Warning => "⚠️",
            aura_sentinel::core::auto_validator::Severity::Info => "ℹ️",
        };
        out.push_str(&format!("{} [{}] {} (línea: {:?})\n", 
            icon, issue.file, issue.message, issue.line));
        if let Some(fix) = &issue.suggested_fix {
            out.push_str(&format!("   🔧 Fix: {}\n", fix));
        }
    }
    
    if !result.auto_fixed.is_empty() {
        out.push_str("\n🔧 AUTO-FIXES APLICADOS:\n");
        for fix in &result.auto_fixed {
            out.push_str(&format!("  ✅ {}\n", fix));
        }
    }
    
    out
}