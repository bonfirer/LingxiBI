use serde::Serialize;

/// Our public API surface re-exports
pub mod client;
pub mod prompts;

/// A single message in a chat completion request.
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

/// A chunk emitted mid-stream (one SSE delta).
#[derive(Debug, Clone)]
pub enum StreamChunk {
    Reasoning(String),
    Content(String),
    Done,
}

/// Full response collected after streaming finishes.
#[derive(Debug, Clone, Default)]
pub struct FullResponse {
    pub content: String,
    pub reasoning_content: String,
}

/// LLM client wrapping an HTTP base URL and auth key.
#[derive(Clone)]
pub struct LlmClient {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl LlmClient {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        // The stored key is encrypted at rest; decrypt it here so every caller
        // gets a ready-to-use plaintext key without touching credentials.
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: crate::crypto::decrypt(&api_key),
            model,
        }
    }
}
