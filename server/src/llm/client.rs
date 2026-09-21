use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{ChatMessage, FullResponse, LlmClient, StreamChunk};

impl LlmClient {
    /// Stream a chat completion, sending chunks into `tx`.
    /// Returns the accumulated full response (content + reasoning_content).
    pub async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        system_prompt: &str,
        max_tokens: i32,
        temperature: f64,
        tx: mpsc::UnboundedSender<StreamChunk>,
    ) -> Result<FullResponse, String> {
        let mut all_messages: Vec<ChatMessage> = vec![ChatMessage {
            role: "system".into(),
            content: system_prompt.into(),
            reasoning_content: None,
        }];
        all_messages.extend_from_slice(messages);

        let body = serde_json::json!({
            "model": self.model,
            "messages": all_messages,
            "stream": true,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "response_format": { "type": "json_object" },
            "stop": null,
        });

        let client = crate::http::streaming_client();
        let resp = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("LLM error {}: {}", status, text));
        }

        let mut stream = resp.bytes_stream();
        let mut full = FullResponse::default();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| format!("Stream error: {}", e))?;
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);

            // Process complete SSE lines
            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer.drain(..=line_end);

                if line.is_empty() {
                    continue;
                }

                // SSE data: prefix
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        let _ = tx.send(StreamChunk::Done);
                        continue;
                    }

                    if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                        if let Some(delta) = parsed
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                        {
                            // reasoning_content
                            if let Some(rc) = delta.get("reasoning_content").and_then(|v| v.as_str())
                            {
                                let rc = rc.to_string();
                                full.reasoning_content.push_str(&rc);
                                let _ = tx.send(StreamChunk::Reasoning(rc));
                            }
                            // content
                            if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                                let c = content.to_string();
                                full.content.push_str(&c);
                                let _ = tx.send(StreamChunk::Content(c));
                            }
                        }
                    }
                }
            }
        }

        // Flush remaining buffer
        let remaining = buffer.trim().to_string();
        if !remaining.is_empty() {
            if let Some(data) = remaining.strip_prefix("data: ") {
                if data != "[DONE]" {
                    if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                        if let Some(delta) = parsed
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                        {
                            if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                                full.content.push_str(content);
                                let _ = tx.send(StreamChunk::Content(content.to_string()));
                            }
                        }
                    }
                }
            }
        }

        Ok(full)
    }

    /// One-shot text generation (non-streaming, no JSON constraint).
    /// Returns the raw content string from the LLM.
    pub async fn chat_oneshot(
        &self,
        messages: &[ChatMessage],
        system_prompt: &str,
        max_tokens: i32,
        temperature: f64,
    ) -> Result<String, String> {
        let mut all_messages: Vec<ChatMessage> = vec![ChatMessage {
            role: "system".into(),
            content: system_prompt.into(),
            reasoning_content: None,
        }];
        all_messages.extend_from_slice(messages);

        let body = serde_json::json!({
            "model": self.model,
            "messages": all_messages,
            "stream": false,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        let client = crate::http::llm_client();
        let resp = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("LLM error {}: {}", status, text));
        }

        let full: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        // Try content first, then reasoning_content (some models put output there)
        let message = full
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"));

        let content = message
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !content.is_empty() {
            return Ok(content.to_string());
        }

        // Fallback: check reasoning_content
        let reasoning = message
            .and_then(|m| m.get("reasoning_content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !reasoning.is_empty() {
            return Ok(reasoning.to_string());
        }

        // Last resort: dump the full response for debugging
        Err(format!("LLM returned no content. Response: {}", serde_json::to_string(&full).unwrap_or_default().chars().take(500).collect::<String>()))
    }

    /// One-shot JSON generation (non-streaming).
    /// Uses `response_format: { type: "json_object" }` to force structured JSON.
    pub async fn generate_json<T: DeserializeOwned>(
        &self,
        messages: &[ChatMessage],
        system_prompt: &str,
        max_tokens: i32,
        temperature: f64,
    ) -> Result<T, String> {
        let mut all_messages: Vec<ChatMessage> = vec![ChatMessage {
            role: "system".into(),
            content: system_prompt.into(),
            reasoning_content: None,
        }];
        all_messages.extend_from_slice(messages);

        let body = serde_json::json!({
            "model": self.model,
            "messages": all_messages,
            "stream": false,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        let client = crate::http::llm_client();
        let resp = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("LLM error {}: {}", status, text));
        }

        let full: Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        let content = full
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "LLM returned no content".to_string())?;

        // Extract JSON — model may wrap in markdown fences
        let json_str = extract_json(content)?;
        let parsed: T = serde_json::from_str(json_str)
            .map_err(|e| format!("Failed to parse LLM JSON: {} — raw: {}", e, json_str))?;

        Ok(parsed)
    }
}

/// Extract the first JSON object/array from text that may contain markdown fences
/// or surrounding prose.
fn extract_json(text: &str) -> Result<&str, String> {
    let text = text.trim();

    // Strip markdown code fences
    let text = if text.starts_with("```json") {
        let end = text[7..].find("```").map(|i| i + 7).unwrap_or(text.len());
        text[7..end].trim()
    } else if text.starts_with("```") {
        let end = text[3..].find("```").map(|i| i + 3).unwrap_or(text.len());
        text[3..end].trim()
    } else {
        text
    };

    // Find first { or [
    let start = text
        .find(|c| c == '{' || c == '[')
        .ok_or_else(|| "No JSON object/array found in LLM response".to_string())?;

    let text = &text[start..];

    // Find matching closing brace/bracket (simple stack-based)
    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut open = '{';
    let mut first = true;

    for (byte_idx, c) in text.char_indices() {
        if first {
            open = c;
            first = false;
        }

        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && in_string {
            escape = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if c == open {
            depth += 1;
            continue;
        }
        if (c == '}' && open == '{') || (c == ']' && open == '[') {
            depth -= 1;
            if depth == 0 {
                let end_byte = byte_idx + c.len_utf8();
                return Ok(&text[..end_byte]);
            }
        }
    }

    Err("Unmatched JSON brackets in LLM response".to_string())
}
