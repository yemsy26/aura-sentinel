fn main() {
    let msg = "Aura, ncesito q me agas un dashbord web sensillo (index.html, styles.css, script.js) q muestre un grafico de barras falso usando chart.js desde un cdn. el fondo de ve ser oscuo y moderno. DEBES ejeuctar el TOOL_VISION_EVALUATOR abrirlo en el navegardor antes d terminar para varificar q la UI se vea vien, y el TOOL_TESTER pa chequear.\n[SYSTEM] La tarea fue interrumpida tras 50 pasos sin converger. Los archivos creados hasta ahora están en el workspace. Por favor revisa manualmente el resultado y reintenta con instrucciones más simples.\nEnter command...".to_lowercase();
    let analysis = ["analiza", "analisa", "analice", "analisis", "que hay", "qu\u{01F8} hay", "que sistema", "qu\u{01F8} sistema", "describe", "explica", "mu\u{01F8}strame", "muestrame", "que tiene", "qu\u{01F8} tiene", "que contiene", "qu\u{01F8} contiene", "inspect", "analyze", "show me", "que es", "qu\u{01F8} es", "que tipo", "qu\u{01F8} tipo", "que hace", "qu\u{01F8} hace", "analisa este", "analiza este", "revisa este"];
    for w in analysis {
        if msg.contains(w) {
            println!("Matched: '{}'", w);
        }
    }
}
