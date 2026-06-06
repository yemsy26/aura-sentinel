use scraper::Html;
use reqwest::header::USER_AGENT;
use std::time::Duration;
use scraper::node::Node;
use ego_tree::NodeRef;

fn extract_text_recursive(node: NodeRef<Node>) -> String {
    let mut text = String::new();
    for child in node.children() {
        match child.value() {
            Node::Text(t) => {
                let trimmed = t.trim();
                if !trimmed.is_empty() {
                    text.push_str(trimmed);
                    text.push(' ');
                }
            }
            Node::Element(e) => {
                let tag = e.name();
                if tag != "script" && tag != "style" && tag != "noscript" && tag != "svg" && tag != "nav" && tag != "footer" {
                    let child_text = extract_text_recursive(child);
                    if !child_text.is_empty() {
                        text.push_str(&child_text);
                        if tag == "p" || tag == "div" || tag == "br" || tag == "li" || tag.starts_with('h') {
                            text.push('\n');
                        } else {
                            text.push(' ');
                        }
                    }
                }
            }
            _ => {}
        }
    }
    text
}

pub async fn fetch_url_text(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Error creando cliente HTTP: {}", e))?;

    let res = client.get(url)
        .header(USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36 Aura-Sentinel/1.0")
        .send()
        .await
        .map_err(|e| format!("Error conectando a la URL: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("Error HTTP: {}", res.status()));
    }

    let html_content = res.text().await.map_err(|e| format!("Error leyendo contenido: {}", e))?;
    
    let document = Html::parse_document(&html_content);
    let mut extracted_text = extract_text_recursive(document.tree.root());

    // Clean up multiple spaces and newlines
    let regex = regex::Regex::new(r" +").unwrap();
    extracted_text = regex.replace_all(&extracted_text, " ").to_string();
    let regex2 = regex::Regex::new(r"\n+").unwrap();
    extracted_text = regex2.replace_all(&extracted_text, "\n").to_string();

    if extracted_text.len() > 6000 {
        extracted_text.truncate(6000);
        extracted_text.push_str("\n...[TEXTO TRUNCADO POR LÍMITE DE VRAM]...");
    }

    if extracted_text.trim().is_empty() {
        // Fallback or raw html truncated
        let raw_html = html_content.chars().take(6000).collect::<String>();
        Ok(format!("Página sin texto renderizable. HTML crudo parcial:\n{}", raw_html))
    } else {
        Ok(extracted_text)
    }
}
