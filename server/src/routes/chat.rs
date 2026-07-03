use axum::{
    extract::{ws::{Message as WsMessage, WebSocket}, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;

use crate::llm::{ChatMessage, LlmClient, StreamChunk};
use crate::llm::prompts;
use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::query;
use crate::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // Browsers can't set Authorization headers on WebSocket connections, so the
    // token is passed as a query param (?token=...). Validate before upgrading
    // and capture the caller's identity for per-conversation ownership checks.
    let auth = params
        .get("token")
        .and_then(|t| crate::routes::auth::validate_session(t).ok());

    let Some((user_id, is_admin)) = auth else {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    };

    ws.on_upgrade(move |socket| handle_socket(socket, state, AuthUser { id: user_id, is_admin }))
}

/// Verify the caller owns a conversation (admins bypass). Returns false for
/// missing conversations too, so callers treat both as "no access".
async fn owns_conversation(state: &AppState, cid: i32, user: &AuthUser) -> bool {
    if user.is_admin {
        // Admins may act on any existing conversation.
        return sqlx::query_scalar::<_, i32>("SELECT id FROM conversations WHERE id = ?")
            .bind(cid)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .is_some();
    }
    sqlx::query_scalar::<_, i32>(
        "SELECT id FROM conversations WHERE id = ? AND owner_user_id = ?",
    )
    .bind(cid)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .is_some()
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>, user: AuthUser) {
    let (mut sender, mut receiver) = socket.split();

    let _ = sender
        .send(WsMessage::Text(
            serde_json::json!({"type": "connected", "message": "AI assistant ready"}).to_string().into(),
        ))
        .await;

    while let Some(Ok(msg)) = receiver.next().await {
        if let WsMessage::Text(text) = msg {
            let user_msg: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let action = user_msg.get("action").and_then(|v| v.as_str()).unwrap_or("chat");

            match action {
                "chat" => {
                    let query = user_msg
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let conversation_id = user_msg
                        .get("conversation_id")
                        .and_then(|v| v.as_i64())
                        .map(|v| v as i32);
                    let datasource_id = user_msg
                        .get("datasource_id")
                        .and_then(|v| v.as_i64())
                        .map(|v| v as i32);
                    let lang = user_msg
                        .get("lang")
                        .and_then(|v| v.as_str())
                        .unwrap_or("zh")
                        .to_string();

                    // Enforce ownership: a caller may only chat within their own
                    // conversation (admins bypass). Reject foreign conversation ids
                    // so nobody can read/inject into someone else's thread.
                    if let Some(cid) = conversation_id {
                        if !owns_conversation(&state, cid, &user).await {
                            let _ = sender
                                .send(WsMessage::Text(
                                    serde_json::json!({"type": "error", "message": "Conversation not found"}).to_string().into(),
                                ))
                                .await;
                            continue;
                        }
                    }

                    // Enforce datasource access: chat generates + runs SQL against
                    // the chosen datasource, so members may only target ones granted
                    // to them (admins bypass).
                    if let Some(dsid) = datasource_id {
                        if crate::routes::datasources::ensure_access(&state, dsid, &user).await.is_err() {
                            let _ = sender
                                .send(WsMessage::Text(
                                    serde_json::json!({"type": "error", "message": "Data source not found"}).to_string().into(),
                                ))
                                .await;
                            continue;
                        }
                    }

                    // Run the chat handler while concurrently listening for a "stop"
                    // action so the user can interrupt mid-generation.
                    enum Outcome {
                        Completed(Result<(), String>),
                        Interrupted,
                    }

                    let outcome = {
                        let chat_fut = handle_chat(&state, &mut sender, query, conversation_id, datasource_id, &lang);
                        tokio::pin!(chat_fut);

                        let result;
                        // Once the client goes away we stop reading the socket and
                        // just drive the generation to completion so it still gets
                        // persisted (async generation — survives navigation).
                        let mut client_gone = false;
                        loop {
                            if client_gone {
                                result = Outcome::Completed((&mut chat_fut).await);
                                break;
                            }
                            tokio::select! {
                                res = &mut chat_fut => {
                                    result = Outcome::Completed(res);
                                    break;
                                }
                                incoming = receiver.next() => {
                                    match incoming {
                                        Some(Ok(WsMessage::Text(t))) => {
                                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                                                if v.get("action").and_then(|a| a.as_str()) == Some("stop") {
                                                    result = Outcome::Interrupted;
                                                    break;
                                                }
                                            }
                                            // Ignore any other messages while busy
                                        }
                                        Some(Ok(WsMessage::Close(_))) | None => {
                                            // Don't abort: let the generation finish and
                                            // persist its result. Stop touching the socket.
                                            client_gone = true;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        result
                    };

                    match outcome {
                        Outcome::Completed(Err(e)) => {
                            let _ = sender
                                .send(WsMessage::Text(
                                    serde_json::json!({"type": "error", "message": e}).to_string().into(),
                                ))
                                .await;
                        }
                        Outcome::Interrupted => {
                            // Generation aborted by user — clear the "generating" flag.
                            if let Some(cid) = conversation_id {
                                let _ = sqlx::query(
                                    "UPDATE conversations SET generation_status = 'idle' WHERE id = ?",
                                )
                                .bind(cid)
                                .execute(&state.db)
                                .await;
                            }
                            let _ = sender
                                .send(WsMessage::Text(
                                    serde_json::json!({"type": "interrupted", "message": "Generation stopped by user"}).to_string().into(),
                                ))
                                .await;
                        }
                        Outcome::Completed(Ok(())) => {}
                    }
                }
                "stop" => {
                    // No generation in progress — acknowledge harmlessly
                    let _ = sender
                        .send(WsMessage::Text(
                            serde_json::json!({"type": "interrupted", "message": "Nothing to stop"}).to_string().into(),
                        ))
                        .await;
                }
                "ping" => {
                    let _ = sender.send(WsMessage::Text(r#"{"type":"pong"}"#.to_string().into())).await;
                }
                _ => {
                    let _ = sender
                        .send(WsMessage::Text(
                            serde_json::json!({"type": "error", "message": "Unknown action"}).to_string().into(),
                        ))
                        .await;
                }
            }
        }
    }
}

/// Wrapper that makes chat generation "async": it persists the user message and
/// marks the conversation as generating up front, runs the actual generation,
/// and records a terminal status (done/failed). Because the caller drives this
/// future to completion even after the client disconnects, the result is always
/// persisted and can be picked up when the user returns to the conversation.
async fn handle_chat(
    state: &AppState,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    query: &str,
    conversation_id: Option<i32>,
    datasource_id: Option<i32>,
    lang: &str,
) -> Result<(), String> {
    if query.is_empty() {
        return Err("Empty query".into());
    }

    // Persist the user message and flag generation up front so a client that
    // reconnects can see the pending turn and poll for the result.
    if let Some(cid) = conversation_id {
        let _ = sqlx::query(
            "UPDATE conversations SET generation_status = 'generating', generation_error = NULL WHERE id = ?",
        )
        .bind(cid)
        .execute(&state.db)
        .await;

        let _ = sqlx::query("INSERT INTO messages (conversation_id, role, content) VALUES (?, 'user', ?)")
            .bind(cid)
            .bind(query)
            .execute(&state.db)
            .await;

        let title: String = query.chars().take(80).collect();
        let _ = sqlx::query("UPDATE conversations SET title = ? WHERE id = ? AND title = 'New Conversation'")
            .bind(&title)
            .bind(cid)
            .execute(&state.db)
            .await;
    }

    let result = handle_chat_inner(state, sender, query, conversation_id, datasource_id, lang).await;

    // Record terminal status.
    if let Some(cid) = conversation_id {
        match &result {
            Ok(()) => {
                let _ = sqlx::query("UPDATE conversations SET generation_status = 'done' WHERE id = ?")
                    .bind(cid)
                    .execute(&state.db)
                    .await;
            }
            Err(e) => {
                let _ = sqlx::query(
                    "UPDATE conversations SET generation_status = 'failed', generation_error = ? WHERE id = ?",
                )
                .bind(e)
                .bind(cid)
                .execute(&state.db)
                .await;
            }
        }
    }

    result
}

async fn handle_chat_inner(
    state: &AppState,
    sender: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    query: &str,
    conversation_id: Option<i32>,
    datasource_id: Option<i32>,
    lang: &str,
) -> Result<(), String> {
    if query.is_empty() {
        return Err("Empty query".into());
    }

    // 1. Load LLM config
    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("LLM config not initialized. Please set it in Settings.")?;

    if llm_cfg.api_key.is_empty() {
        return Err("API key not configured. Please set it in Settings.".into());
    }

    let client = LlmClient::new(llm_cfg.base_url.clone(), llm_cfg.api_key.clone(), llm_cfg.model.clone());

    // 2. Load conversation history
    let history = if let Some(cid) = conversation_id {
        sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE conversation_id = ? ORDER BY created_at ASC",
        )
        .bind(cid)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
    } else {
        vec![]
    };

    // 3. Build knowledge graph context
    let kg_context = build_kg_context(state, datasource_id, query).await;

    // 4. Build system prompt
    let system = prompts::chat_system_prompt(&kg_context, lang);

    // 5. Build messages for LLM, injecting reasoning_content
    let mut messages: Vec<ChatMessage> = Vec::new();
    for msg in &history {
        let mut cm = ChatMessage {
            role: msg.role.clone(),
            content: msg.content.clone(),
            reasoning_content: None,
        };
        if msg.role == "assistant" {
            cm.reasoning_content = msg.reasoning_content.clone();
        }
        messages.push(cm);
    }
    messages.push(ChatMessage {
        role: "user".into(),
        content: query.to_string(),
        reasoning_content: None,
    });

    // 6. Stream LLM response via channel
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamChunk>();

    let llm_client = client.clone();
    let llm_messages = messages.clone();
    let llm_system = system.clone();
    let max_tokens = llm_cfg.max_tokens;
    let temperature = llm_cfg.temperature;

    let chat_start = std::time::Instant::now();
    let llm_handle = tokio::spawn(async move {
        llm_client.chat_stream(&llm_messages, &llm_system, max_tokens, temperature, tx).await
    });

    // Abort the spawned LLM task if this future is dropped (e.g. user interrupts).
    // This prevents the underlying HTTP request from continuing in the background.
    struct AbortGuard(tokio::task::AbortHandle);
    impl Drop for AbortGuard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _abort_guard = AbortGuard(llm_handle.abort_handle());

    // Forward chunks to WebSocket
    while let Some(chunk) = rx.recv().await {
        let (msg_type, content) = match chunk {
            StreamChunk::Reasoning(text) => ("reasoning", text),
            StreamChunk::Content(text) => ("content", text),
            StreamChunk::Done => continue,
        };
        let _ = sender
            .send(WsMessage::Text(
                serde_json::json!({ "type": msg_type, "content": content }).to_string().into(),
            ))
            .await;
    }

    let chat_duration = chat_start.elapsed().as_millis() as u64;

    let full = match llm_handle.await {
        Ok(Ok(f)) => {
            crate::ai_log::log_ai_request(
                &state.db, "chat", &llm_cfg.model,
                chat_duration, "success", None,
                Some(&format!("query={}", &query[..query.len().min(200)])),
                Some(&format!("user: {}", query)),
                Some(&f.content),
            ).await;
            tracing::info!("AI chat OK: {}ms", chat_duration);
            f
        }
        Ok(Err(e)) => {
            crate::ai_log::log_ai_request(
                &state.db, "chat", &llm_cfg.model,
                chat_duration, "failed", Some(&e),
                Some(&format!("query={}", &query[..query.len().min(200)])),
                Some(&format!("user: {}", query)),
                None,
            ).await;
            tracing::error!("AI chat FAILED: {}ms, {}", chat_duration, e);
            return Err(format!("LLM error: {}", e));
        }
        Err(e) => {
            crate::ai_log::log_ai_request(
                &state.db, "chat", &llm_cfg.model,
                chat_duration, "failed", Some(&e.to_string()),
                Some(&format!("query={}", &query[..query.len().min(200)])),
                Some(&format!("user: {}", query)),
                None,
            ).await;
            return Err(format!("LLM task failed: {}", e));
        }
    };

    // 7. Parse JSON from response
    let llm_json: serde_json::Value = serde_json::from_str(&full.content)
        .map_err(|e| format!("Failed to parse LLM response as JSON: {} — raw: {}", e, &full.content[..full.content.len().min(200)]))?;

    let explanation = llm_json
        .get("explanation")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let _ = sender
        .send(WsMessage::Text(
            serde_json::json!({"type": "explanation", "content": explanation}).to_string().into(),
        ))
        .await;

    // 8. Execute SQL queries and create data pools
    let queries = llm_json.get("queries").and_then(|v| v.as_array());
    let mut pool_ids: Vec<i32> = Vec::new();

    if let Some(qs) = queries {
        for q in qs {
            let sql = q.get("sql").and_then(|v| v.as_str()).unwrap_or("");
            let ds_id = q.get("datasource_id").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
            let label = q.get("label").and_then(|v| v.as_str()).unwrap_or("query");

            if sql.is_empty() {
                continue;
            }

            if let Err(e) = query::validate_sql(sql) {
                let _ = sender
                    .send(WsMessage::Text(
                        serde_json::json!({"type": "query_error", "sql": sql, "message": e})
                            .to_string().into(),
                    ))
                    .await;
                continue;
            }

            let ds = match sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
                .bind(ds_id)
                .fetch_optional(&state.db)
                .await
            {
                Ok(Some(ds)) => ds,
                _ => {
                    let _ = sender
                        .send(WsMessage::Text(
                            serde_json::json!({"type": "query_error", "sql": sql, "message": "Data source not found"})
                                .to_string().into(),
                        ))
                        .await;
                    continue;
                }
            };

            // Execute with auto-retry on failure (up to 2 retries via LLM fix)
            let mut current_sql = sql.to_string();
            let mut last_error = String::new();
            let mut success = false;

            for attempt in 0..3 {
                if attempt > 0 {
                    // Ask LLM to fix the SQL
                    let _ = sender
                        .send(WsMessage::Text(
                            serde_json::json!({"type": "content", "content": format!("\n🔄 SQL 执行失败，正在自动修正 (尝试 {}/2)...", attempt)})
                                .to_string().into(),
                        ))
                        .await;

                    match fix_sql_with_llm(&client, &system, &current_sql, &last_error, &llm_cfg).await {
                        Ok(fixed) => {
                            if let Err(e) = query::validate_sql(&fixed) {
                                last_error = format!("Fixed SQL validation failed: {}", e);
                                continue;
                            }
                            current_sql = fixed;
                        }
                        Err(_) => break,
                    }
                }

                // Execute with shared safety guards (validation + timeout + row cap).
                let result = query::execute_validated(state, &ds, &current_sql).await;

                match result {
                    Ok(qr) => {
                        let cache = serde_json::to_value(&qr.rows).ok();
                        let pool_name = label.to_string();
                        let final_sql = current_sql.clone();

                        let pool_result = sqlx::query(
                            "INSERT INTO data_pools (conversation_id, name, sql_query, datasource_id, result_cache, row_count) VALUES (?, ?, ?, ?, ?, ?)",
                        )
                        .bind(conversation_id)
                        .bind(&pool_name)
                        .bind(&final_sql)
                        .bind(ds_id)
                        .bind(&cache)
                        .bind(qr.row_count as i32)
                        .execute(&state.db)
                        .await;

                        match pool_result {
                            Ok(r) => {
                                let pid = r.last_insert_id() as i32;
                                pool_ids.push(pid);
                                let _ = sender
                                    .send(WsMessage::Text(
                                        serde_json::json!({
                                            "type": "query_result",
                                            "pool_id": pid,
                                            "label": label,
                                            "sql": final_sql,
                                            "datasource_id": ds_id,
                                            "columns": qr.columns,
                                            "row_count": qr.row_count,
                                            "retries": attempt,
                                        })
                                        .to_string().into(),
                                    ))
                                    .await;
                            }
                            Err(e) => {
                                let _ = sender
                                    .send(WsMessage::Text(
                                        serde_json::json!({"type": "query_error", "sql": final_sql, "message": format!("DB save error: {}", e)})
                                            .to_string().into(),
                                    ))
                                    .await;
                            }
                        }
                        success = true;
                        break;
                    }
                    Err(e) => {
                        last_error = e;
                    }
                }
            }

            if !success {
                let _ = sender
                    .send(WsMessage::Text(
                        serde_json::json!({"type": "query_error", "sql": current_sql, "message": last_error})
                            .to_string().into(),
                    ))
                    .await;
            }
        }
    }

    // 9. Save the assistant message to DB. (The user message and title were
    //    persisted up front by the handle_chat wrapper.)
    if let Some(cid) = conversation_id {
        let metadata = if !pool_ids.is_empty() {
            Some(serde_json::json!({ "pool_ids": pool_ids }))
        } else {
            None
        };

        let _ = sqlx::query(
            "INSERT INTO messages (conversation_id, role, content, reasoning_content, metadata) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(cid)
        .bind("assistant")
        .bind(&full.content)
        .bind(if full.reasoning_content.is_empty() { None } else { Some(&full.reasoning_content) })
        .bind(&metadata)
        .execute(&state.db)
        .await;
    }

    // 10. Send final done event
    let _ = sender
        .send(WsMessage::Text(
            serde_json::json!({
                "type": "done",
                "message": explanation,
                "pool_ids": pool_ids,
            })
            .to_string().into(),
        ))
        .await;

    // 11. Background: extract business knowledge from this conversation
    if let Some(ds_id) = datasource_id {
        let state_clone = state.db.clone();
        let llm_cfg_clone = llm_cfg.clone();
        let query_owned = query.to_string();
        let explanation_owned = explanation.to_string();
        let kg_context_clone = kg_context.clone();
        tokio::spawn(async move {
            let _ = extract_knowledge(
                &state_clone, &llm_cfg_clone, ds_id,
                &query_owned, &explanation_owned, &kg_context_clone,
            ).await;
        });
    }

    Ok(())
}

/// Extract business knowledge from a conversation turn and save to knowledge_base.
async fn extract_knowledge(
    db: &sqlx::MySqlPool,
    llm_cfg: &LLMConfig,
    datasource_id: i32,
    user_query: &str,
    ai_response: &str,
    schema_context: &str,
) -> Result<(), String> {
    if llm_cfg.api_key.is_empty() { return Ok(()); }

    let client = LlmClient::new(llm_cfg.base_url.clone(), llm_cfg.api_key.clone(), llm_cfg.model.clone());

    // Load existing knowledge for this datasource so the LLM can avoid duplicates
    let existing_entries: Vec<(i32, String, String, String, String)> = sqlx::query_as(
        "SELECT id, category, title, content, confidence FROM knowledge_base WHERE datasource_id = ? ORDER BY category"
    )
    .bind(datasource_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let existing_str = existing_entries
        .iter()
        .map(|(_, cat, title, content, _)| format!("- [{}] {}: {}", cat, title, content))
        .collect::<Vec<_>>()
        .join("\n");

    let system = prompts::knowledge_extraction_prompt(schema_context, &existing_str);

    let conversation_summary = format!(
        "User asked: {}\nAI responded: {}",
        user_query, ai_response
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: conversation_summary,
        reasoning_content: None,
    }];

    // Use generate_json to get structured output
    #[derive(serde::Deserialize, serde::Serialize)]
    struct KnowledgeItem {
        category: Option<String>,
        title: String,
        content: String,
        confidence: Option<String>,
    }

    let kb_start = std::time::Instant::now();
    let items: Vec<KnowledgeItem> = match client
        .generate_json::<Vec<KnowledgeItem>>(&messages, &system, 4096, llm_cfg.temperature)
        .await
    {
        Ok(items) => {
            let dur = kb_start.elapsed().as_millis() as u64;
            let output_str = serde_json::to_string(&items).unwrap_or_default();
            crate::ai_log::log_ai_request(
                db, "knowledge_extraction", &llm_cfg.model,
                dur, "success", None,
                Some(&format!("ds_id={}, items={}", datasource_id, items.len())),
                Some(&format!("conversation: {} | {}", user_query, ai_response)),
                Some(&output_str),
            ).await;
            items
        }
        Err(e) => {
            let dur = kb_start.elapsed().as_millis() as u64;
            crate::ai_log::log_ai_request(
                db, "knowledge_extraction", &llm_cfg.model,
                dur, "failed", Some(&e),
                Some(&format!("ds_id={}", datasource_id)),
                Some(&format!("conversation: {} | {}", user_query, ai_response)),
                None,
            ).await;
            return Ok(());
        }
    };

    // Save new knowledge entries with semantic dedup against existing entries
    for item in &items {
        if item.title.is_empty() || item.content.is_empty() { continue; }

        let new_conf = item.confidence.as_deref().unwrap_or("medium");

        // Find the best-matching existing entry by combined title+content similarity
        let mut best_match: Option<(i32, f64, &str)> = None;
        for (eid, _ecat, etitle, econtent, econf) in &existing_entries {
            let title_sim = token_similarity(&item.title, etitle);
            let content_sim = token_similarity(&item.content, econtent);
            // Weight title higher than content
            let combined = title_sim * 0.6 + content_sim * 0.4;

            if best_match.map_or(true, |(_, s, _)| combined > s) {
                best_match = Some((*eid, combined, econf.as_str()));
            }
        }

        // If a sufficiently similar entry exists, merge/update instead of inserting
        if let Some((eid, sim, existing_conf)) = best_match {
            if sim >= 0.55 {
                // Considered semantically duplicate.
                // Update the existing entry if the new one has higher confidence
                // or is more detailed (longer content).
                if confidence_rank(new_conf) > confidence_rank(existing_conf) {
                    let _ = sqlx::query(
                        "UPDATE knowledge_base SET content = ?, confidence = ?, category = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
                    )
                    .bind(&item.content)
                    .bind(new_conf)
                    .bind(item.category.as_deref().unwrap_or("relation"))
                    .bind(eid)
                    .execute(db)
                    .await;
                }
                continue; // Don't insert a duplicate
            }
        }

        let _ = sqlx::query(
            "INSERT INTO knowledge_base (datasource_id, category, title, content, source, confidence) VALUES (?, ?, ?, ?, 'ai', ?)"
        )
        .bind(datasource_id)
        .bind(item.category.as_deref().unwrap_or("relation"))
        .bind(&item.title)
        .bind(&item.content)
        .bind(new_conf)
        .execute(db)
        .await;
    }

    Ok(())
}

/// Ask LLM to fix a failed SQL query based on the error message.
async fn fix_sql_with_llm(
    client: &LlmClient,
    system_prompt: &str,
    failed_sql: &str,
    error: &str,
    llm_cfg: &LLMConfig,
) -> Result<String, String> {
    let fix_prompt = format!(
        r#"The following SQL query failed with an error. Fix the SQL and return ONLY the corrected SQL query, nothing else.

## Failed SQL
```sql
{}
```

## Error Message
{}

## Rules
1. Return ONLY the fixed SQL — no explanations, no markdown fences, no JSON wrapper.
2. The fix must still be a SELECT/SHOW/DESCRIBE query (read-only).
3. Fix the specific error while preserving the original intent."#,
        failed_sql, error
    );

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: fix_prompt,
        reasoning_content: None,
    }];

    let result = client
        .chat_oneshot(&messages, system_prompt, llm_cfg.max_tokens, llm_cfg.temperature)
        .await?;

    // Clean up the result — remove markdown fences if present
    let sql = result
        .trim()
        .trim_start_matches("```sql")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string();

    if sql.is_empty() {
        return Err("LLM returned empty fix".to_string());
    }

    Ok(sql)
}

/// Build knowledge graph context string from datasource schemas.
/// If `datasource_id` is provided, only include that datasource's schema.
/// Otherwise, include all schemas.
/// `last_user_query` is used to rank few-shot examples by relevance.
pub async fn build_kg_context(state: &AppState, datasource_id: Option<i32>, last_user_query: &str) -> String {
    let schemas = if let Some(ds_id) = datasource_id {
        sqlx::query_as::<_, (i32, serde_json::Value)>(
            "SELECT s.datasource_id, s.schema_data FROM `schemas` s WHERE s.datasource_id = ?",
        )
        .bind(ds_id)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<_, (i32, serde_json::Value)>(
            "SELECT s.datasource_id, s.schema_data FROM `schemas` s
             JOIN datasources d ON d.id = s.datasource_id
             ORDER BY s.created_at DESC",
        )
        .fetch_all(&state.db)
        .await
    };

    match schemas {
        Ok(rows) if !rows.is_empty() => {
            let mut ctx = String::new();
            for (ds_id, schema_data) in &rows {
                // Get datasource name
                let ds_name = sqlx::query_as::<_, (String,)>(
                    "SELECT name FROM datasources WHERE id = ?",
                )
                .bind(ds_id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
                .map(|(n,)| n)
                .unwrap_or_else(|| format!("datasource_{}", ds_id));

                ctx.push_str(&format!("\n### Data Source: {} (id={})\n", ds_name, ds_id));

                // Load user-provided table descriptions for this datasource
                let table_descs: std::collections::HashMap<String, String> =
                    sqlx::query_as::<_, (String, String)>(
                        "SELECT table_name, description FROM table_descriptions WHERE datasource_id = ?",
                    )
                    .bind(ds_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .collect();

                // Load user-provided column descriptions, keyed by "table|column"
                let col_descs: std::collections::HashMap<String, String> =
                    sqlx::query_as::<_, (String, String, String)>(
                        "SELECT table_name, column_name, description FROM column_descriptions WHERE datasource_id = ?",
                    )
                    .bind(ds_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(t, c, d)| (format!("{}|{}", t, c), d))
                    .collect();

                if let Some(tables) = schema_data.get("tables").and_then(|v| v.as_array()) {
                    for table in tables {
                        let name = table.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let table_comment = table.get("comment").and_then(|v| v.as_str()).unwrap_or("");
                        if table_comment.is_empty() {
                            ctx.push_str(&format!("Table: {}\n", name));
                        } else {
                            ctx.push_str(&format!("Table: {} -- {}\n", name, table_comment));
                        }
                        // Inject user-provided table description (business notes for the AI)
                        if let Some(desc) = table_descs.get(name) {
                            ctx.push_str(&format!("  # Note: {}\n", desc));
                        }
                        if let Some(cols) = table.get("columns").and_then(|v| v.as_array()) {
                            for col in cols {
                                let col_name = col.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let col_type = col.get("data_type").and_then(|v| v.as_str()).unwrap_or("?");
                                let col_comment = col.get("comment").and_then(|v| v.as_str()).unwrap_or("");
                                let pk = col.get("is_primary_key").and_then(|v| v.as_bool()).unwrap_or(false);
                                let fk = col.get("is_foreign_key").and_then(|v| v.as_bool()).unwrap_or(false);
                                let mut flags = String::new();
                                if pk { flags.push_str(" PK"); }
                                if fk { flags.push_str(" FK"); }

                                // Prefer the user-provided column description; fall back to the DB comment.
                                let user_desc = col_descs.get(&format!("{}|{}", name, col_name));
                                let effective_comment = match (user_desc, col_comment.is_empty()) {
                                    (Some(d), _) => d.as_str(),
                                    (None, false) => col_comment,
                                    (None, true) => "",
                                };

                                if !effective_comment.is_empty() {
                                    ctx.push_str(&format!("  - {} ({}){} -- {}\n", col_name, col_type, flags, effective_comment));
                                } else {
                                    ctx.push_str(&format!("  - {} ({}){}\n", col_name, col_type, flags));
                                }
                            }
                        }
                    }
                }
                if let Some(rels) = schema_data.get("relationships").and_then(|v| v.as_array()) {
                    if !rels.is_empty() {
                        ctx.push_str("Relationships:\n");
                        for rel in rels {
                            let src = rel.get("source_table").and_then(|v| v.as_str()).unwrap_or("?");
                            let sc = rel.get("source_column").and_then(|v| v.as_str()).unwrap_or("?");
                            let tgt = rel.get("target_table").and_then(|v| v.as_str()).unwrap_or("?");
                            let tc = rel.get("target_column").and_then(|v| v.as_str()).unwrap_or("?");
                            ctx.push_str(&format!("  {}.{} → {}.{}\n", src, sc, tgt, tc));
                        }
                    }
                }
            }
            if ctx.trim().is_empty() {
                "No schema has been introspected yet. Ask the user to scan a data source first.".into()
            } else {
                // Append AI knowledge base entries (ranked by relevance + confidence,
                // clamped to a budget so the learned context stays focused).
                let knowledge = if let Some(ds_id) = datasource_id {
                    sqlx::query_as::<_, (String, String, String, String)>(
                        "SELECT category, title, content, confidence FROM knowledge_base WHERE datasource_id = ? ORDER BY category"
                    )
                    .bind(ds_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                } else {
                    sqlx::query_as::<_, (String, String, String, String)>(
                        "SELECT category, title, content, confidence FROM knowledge_base ORDER BY datasource_id, category"
                    )
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                };

                let knowledge = rank_knowledge(knowledge, last_user_query);
                if !knowledge.is_empty() {
                    ctx.push_str("\n### AI Knowledge Base (learned from previous conversations)\n");
                    for (category, title, content, _conf) in &knowledge {
                        ctx.push_str(&format!("- [{}] {}: {}\n", category, title, content));
                    }
                }

                // Metrics Library: curated, user-validated SQL with business-meaningful
                // names. This is high-value knowledge — the AI should reuse these proven
                // queries (and their definitions of business terms) when relevant.
                let metrics: Vec<(String, Option<String>, String)> = if let Some(ds_id) = datasource_id {
                    sqlx::query_as(
                        "SELECT name, description, sql_query FROM metric_pools WHERE datasource_id = ? ORDER BY updated_at DESC LIMIT 50",
                    )
                    .bind(ds_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                } else {
                    sqlx::query_as(
                        "SELECT name, description, sql_query FROM metric_pools ORDER BY updated_at DESC LIMIT 50",
                    )
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                };

                if !metrics.is_empty() {
                    // Rank by relevance to the user's query, reusing the example ranker.
                    let metric_pairs: Vec<(String, String)> = metrics
                        .iter()
                        .map(|(name, desc, sql)| {
                            let label = match desc {
                                Some(d) if !d.is_empty() => format!("{} — {}", name, d),
                                _ => name.clone(),
                            };
                            (label, sql.clone())
                        })
                        .collect();
                    let top = rank_examples_by_relevance(metric_pairs, last_user_query);

                    ctx.push_str("\n### Metrics Library (curated, validated metrics — prefer reusing these definitions and SQL)\n");
                    for (label, sql) in &top {
                        ctx.push_str(&format!("- {}\n  SQL: {}\n", label, sql.replace('\n', " ")));
                    }
                }

                // Load few-shot examples for this datasource — match by relevance to last user query
                let examples = if let Some(ds_id) = datasource_id {
                    // Get all examples for this datasource, then rank by keyword overlap
                    let all_examples: Vec<(String, String)> = sqlx::query_as(
                        "SELECT question, answer FROM ai_examples WHERE datasource_id = ? ORDER BY created_at DESC LIMIT 30"
                    )
                    .bind(ds_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default();
                    rank_examples_by_relevance(all_examples, last_user_query)
                } else {
                    let all_examples: Vec<(String, String)> = sqlx::query_as(
                        "SELECT question, answer FROM ai_examples ORDER BY created_at DESC LIMIT 30"
                    )
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default();
                    rank_examples_by_relevance(all_examples, last_user_query)
                };

                if !examples.is_empty() {
                    ctx.push_str("\n### Good Examples (follow these patterns)\n");
                    for (question, answer) in &examples {
                        ctx.push_str(&format!("Q: {}\nA: {}\n\n", question, answer));
                    }
                }

                // Append column profiles (sample values, enums, ranges)
                if let Some(ds_id) = datasource_id {
                    let profile_ctx = crate::column_profiler::build_profile_context(state, ds_id).await;
                    if !profile_ctx.is_empty() {
                        ctx.push_str(&profile_ctx);
                    }
                }

                ctx
            }
        }
        _ => "No schema available. Please run introspection on a data source first.".into(),
    }
}

// ── Relevance retrieval ──────────────────────────────────────────────────
//
// The learned context (knowledge base, few-shot examples, metrics) is ranked
// by relevance to the user's current query and clamped to a token/char budget
// so it stays focused and can't balloon the prompt as the knowledge grows.
//
// Scoring is currently lexical (keyword overlap). The pure scoring/selection
// functions below are deliberately isolated so an embedding-based scorer can be
// dropped in later without touching the retrieval call sites.

/// Max few-shot examples (and metrics) injected.
const EXAMPLE_MAX: usize = 5;
/// Max knowledge-base entries injected, and the char budget for that block.
const KNOWLEDGE_MAX_ENTRIES: usize = 25;
const KNOWLEDGE_CHAR_BUDGET: usize = 6000;

/// Tokenize a string into lowercased keywords longer than 2 chars.
fn keywords(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 2)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Lexical relevance of `text` against precomputed query keywords:
/// +2 per matched keyword, +5 when one string contains the other (near-match).
fn lexical_score(text: &str, query_keywords: &[String], query_lower: &str) -> usize {
    if query_keywords.is_empty() {
        return 0;
    }
    let text_lower = text.to_lowercase();
    let mut score = 0usize;
    for kw in query_keywords {
        if text_lower.contains(kw) {
            score += 2;
        }
    }
    if !query_lower.is_empty()
        && (text_lower.contains(query_lower) || query_lower.contains(&text_lower))
    {
        score += 5;
    }
    score
}

/// Rank few-shot examples by relevance to the user's current query.
/// Returns the top `EXAMPLE_MAX` most relevant examples.
fn rank_examples_by_relevance(
    examples: Vec<(String, String)>,
    user_query: &str,
) -> Vec<(String, String)> {
    let kws = keywords(user_query);
    if examples.is_empty() || kws.is_empty() {
        return examples.into_iter().take(EXAMPLE_MAX).collect();
    }
    let query_lower = user_query.to_lowercase();

    let mut scored: Vec<(usize, (String, String))> = examples
        .into_iter()
        .map(|(q, a)| {
            let combined = format!("{} {}", q, a);
            // Question matches count double (weighted by adding the question score).
            let score = lexical_score(&combined, &kws, &query_lower)
                + lexical_score(&q, &kws, &query_lower);
            (score, (q, a))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(EXAMPLE_MAX).map(|(_, ex)| ex).collect()
}

/// Rank knowledge entries `(category, title, content, confidence)` by relevance
/// to the user query, boosted by stored confidence, then keep the highest-scored
/// entries that fit within `KNOWLEDGE_CHAR_BUDGET` (and `KNOWLEDGE_MAX_ENTRIES`).
fn rank_knowledge(
    entries: Vec<(String, String, String, String)>,
    user_query: &str,
) -> Vec<(String, String, String, String)> {
    if entries.is_empty() {
        return entries;
    }
    let kws = keywords(user_query);
    let query_lower = user_query.to_lowercase();

    let mut scored: Vec<(i64, (String, String, String, String))> = entries
        .into_iter()
        .map(|e| {
            let combined = format!("{} {}", e.1, e.2); // title + content
            let mut score = lexical_score(&combined, &kws, &query_lower) as i64;
            // Confidence acts as a tie-breaker / mild prior when nothing matches.
            score += match e.3.as_str() {
                "high" => 3,
                "medium" => 1,
                _ => 0,
            };
            (score, e)
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    let mut out = Vec::new();
    let mut used = 0usize;
    for (_, e) in scored.into_iter().take(KNOWLEDGE_MAX_ENTRIES) {
        let line_len = e.0.len() + e.1.len() + e.2.len() + 8;
        // Always keep at least one entry; otherwise stop once the budget is hit.
        if !out.is_empty() && used + line_len > KNOWLEDGE_CHAR_BUDGET {
            break;
        }
        used += line_len;
        out.push(e);
    }
    out
}

/// Compute Jaccard token similarity between two strings (0.0 ~ 1.0).
/// Used for semantic-ish deduplication of knowledge base entries.
fn token_similarity(a: &str, b: &str) -> f64 {
    let tokens_a: std::collections::HashSet<String> = a
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 1)
        .map(|w| w.to_string())
        .collect();

    let tokens_b: std::collections::HashSet<String> = b
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 1)
        .map(|w| w.to_string())
        .collect();

    if tokens_a.is_empty() && tokens_b.is_empty() {
        return 1.0;
    }
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }

    let intersection = tokens_a.intersection(&tokens_b).count() as f64;
    let union = tokens_a.union(&tokens_b).count() as f64;

    intersection / union
}

/// Rank confidence levels for comparison: high > medium > low.
fn confidence_rank(confidence: &str) -> u8 {
    match confidence.to_lowercase().as_str() {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod retrieval_tests {
    use super::*;

    #[test]
    fn keywords_lowercases_and_drops_short_tokens() {
        let kws = keywords("Top Revenue by Region!");
        assert!(kws.contains(&"top".to_string()));
        assert!(kws.contains(&"revenue".to_string()));
        assert!(kws.contains(&"region".to_string()));
        // "by" is <= 2 chars and dropped.
        assert!(!kws.contains(&"by".to_string()));
    }

    #[test]
    fn lexical_score_rewards_keyword_and_substring_matches() {
        let kws = keywords("monthly revenue");
        let q = "monthly revenue".to_lowercase();
        // Two keyword matches (+4) plus full-substring containment (+5).
        assert_eq!(lexical_score("monthly revenue report", &kws, &q), 9);
        // No overlap → 0.
        assert_eq!(lexical_score("user login latency", &kws, &q), 0);
    }

    #[test]
    fn rank_examples_orders_by_relevance_and_caps_at_max() {
        let examples = vec![
            ("How many users?".to_string(), "SELECT count(*) FROM users".to_string()),
            ("Monthly revenue trend".to_string(), "SELECT sum(total) FROM orders".to_string()),
            ("a".to_string(), "b".to_string()),
            ("c".to_string(), "d".to_string()),
            ("e".to_string(), "f".to_string()),
            ("g".to_string(), "h".to_string()),
        ];
        let ranked = rank_examples_by_relevance(examples, "monthly revenue");
        assert_eq!(ranked.len(), EXAMPLE_MAX);
        // The revenue example should rank first.
        assert_eq!(ranked[0].0, "Monthly revenue trend");
    }

    #[test]
    fn rank_knowledge_prefers_relevant_then_confident() {
        let entries = vec![
            ("field".into(), "user region".into(), "region maps to sales territory".into(), "low".into()),
            ("business".into(), "revenue rule".into(), "monthly revenue excludes cancelled orders".into(), "high".into()),
        ];
        let ranked = rank_knowledge(entries, "monthly revenue");
        // The revenue entry is both relevant and high-confidence → first.
        assert_eq!(ranked[0].1, "revenue rule");
    }

    #[test]
    fn rank_knowledge_enforces_char_budget() {
        // Build many large entries; the budget must cap how many survive.
        let big = "x".repeat(1000);
        let entries: Vec<(String, String, String, String)> = (0..50)
            .map(|i| ("business".into(), format!("t{}", i), big.clone(), "medium".into()))
            .collect();
        let ranked = rank_knowledge(entries, "unrelated query terms");
        // Each entry is ~1000 chars; the 6000-char budget keeps far fewer than 50.
        assert!(ranked.len() <= 7, "expected budget to cap entries, got {}", ranked.len());
        assert!(!ranked.is_empty());
    }
}
