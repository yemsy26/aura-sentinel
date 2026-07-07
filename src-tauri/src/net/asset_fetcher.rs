use std::fs;
use std::path::Path;
use urlencoding::encode;

pub async fn download_asset(query: &str, output_path: &str) -> Result<String, String> {
    let output_path = Path::new(output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
        }
    }

    // Using Pollinations AI for free, instant, no-key image generation
    let enhanced_query = format!("{}, simple video game asset, 2d, plain white background", query);
    let url = format!("https://image.pollinations.ai/prompt/{}?width=256&height=256&nologo=true", encode(&enhanced_query));
    
    let response = reqwest::get(&url).await.map_err(|e| format!("Request failed: {}", e))?;
    
    if response.status().is_success() {
        let bytes = response.bytes().await.map_err(|e| format!("Failed to read bytes: {}", e))?;
        fs::write(output_path, bytes).map_err(|e| format!("Failed to write file: {}", e))?;
        Ok(format!("Asset successfully generated and saved to {:?}", output_path))
    } else {
        Err(format!("Failed to download asset. Status: {}", response.status()))
    }
}
