use xcap::Monitor;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;

pub async fn evaluate_vision(prompt: &str, use_advanced_model: bool) -> Result<String, String> {
    let base64_img = {
        let monitors = Monitor::all().map_err(|e| format!("Failed to get monitors: {}", e))?;
        let primary = monitors.first().ok_or("No monitors found")?;
        
        let image = primary.capture_image().map_err(|e| format!("Failed to capture screen: {}", e))?;
        
        // Save to memory buffer
        let mut buffer = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buffer);
        image.write_to(&mut cursor, image::ImageFormat::Png).map_err(|e| format!("Failed to encode png: {}", e))?;
        
        STANDARD.encode(&buffer)
    };

    let model_name = if use_advanced_model { "llama3.2-vision" } else { "moondream" };

    // 2. Ask Ollama
    let client = reqwest::Client::new();
    let payload = json!({
        "model": model_name,
        "prompt": prompt,
        "images": [base64_img],
        "stream": false
    });

    let res = client.post("http://127.0.0.1:11434/api/generate")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {}", e))?;

    if res.status().is_success() {
        let response_text = res.text().await.map_err(|e| format!("Failed to read text: {}", e))?;
        let json_val: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| format!("Invalid JSON from Ollama: {}", e))?;
        if let Some(resp) = json_val["response"].as_str() {
            Ok(format!("[VISION QA - {}]: {}", model_name, resp))
        } else {
            Err("Missing 'response' field in Ollama output".to_string())
        }
    } else {
        Err(format!("Ollama API returned {}", res.status()))
    }
}
