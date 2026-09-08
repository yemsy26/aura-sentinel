#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::Path;
use chrono::Utc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub module: String,
    pub message: String,
    pub context: Option<serde_json::Value>,
}

pub struct AuraLogger {
    workspace: String,
    module: String,
}

impl AuraLogger {
    pub fn new(workspace: &str, module: &str) -> Self {
        Self {
            workspace: workspace.to_string(),
            module: module.to_string(),
        }
    }

    pub fn log(&self, level: LogLevel, message: &str, context: Option<serde_json::Value>) {
        let entry = LogEntry {
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            level,
            module: self.module.clone(),
            message: message.to_string(),
            context,
        };

        let log_path = Path::new(&self.workspace).join(".aura_logs.jsonl");
        if let Ok(json) = serde_json::to_string(&entry) {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(log_path) {
                let _ = writeln!(file, "{}", json);
            }
        }
    }

    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message, None);
    }

    pub fn warning(&self, message: &str) {
        self.log(LogLevel::Warning, message, None);
    }

    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message, None);
    }
}
