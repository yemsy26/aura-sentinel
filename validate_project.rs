use std::path::Path;
use std::env;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let workspace = if args.len() > 1 {
        args[1].clone()
    } else {
        env::current_dir().unwrap().to_string_lossy().to_string()
    };

    println!("🔍 Auto-validando proyecto en: {}", workspace);

    let report = aura_core::auto_validator::WebProjectValidator::validate_project(&workspace).await;
    let fixes = aura_core::auto_validator::AutoFixer::fix_project(&workspace, &report).await;

    println!("{}", report.summary());
    
    if !fixes.is_empty() {
        println!("\n🔧 FIXES APLICADOS:");
        for fix in fixes {
            println!("  ✅ {}", fix);
        }
    }
    
    if report.has_errors() {
        println!("\n❌ VALIDACIÓN FALLÓ");
        std::process::exit(1);
    } else {
        println!("\n✅ VALIDACIÓN EXITOSA");
    }
}