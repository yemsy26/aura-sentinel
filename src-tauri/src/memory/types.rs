use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FenixMemoryLog {
    pub task_id: String,
    pub timestamp: String,
    pub file_path: String,
    pub summary: String, // Max 3 lines
    pub previous_hash: String,
    pub compilation_status: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatMessage {
    pub sender: String,
    pub text: String,
    pub timestamp: String,
}
