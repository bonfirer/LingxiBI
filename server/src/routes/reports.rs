use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;
use std::collections::HashMap;

use crate::llm::LlmClient;
use crate::llm::prompts;
use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::{ensure_owner, internal_error};
use crate::AppState;

/// Fetch a report by id and verify the caller owns it (admins bypass).
/// Returns 404 for missing OR non-owned reports so ownership isn't leaked.
async fn load_owned_report(
    state: &AppState,
    id: i32,
    user: &AuthUser,
) -> Result<Report, (StatusCode, String)> {
    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Report not found".to_string()))?;
    ensure_owner(user, report.owner_user_id)?;
    Ok(report)
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<Report>>, (StatusCode, String)> {
    // Exclude the heavy LONGTEXT columns from the list. The sidebar polls this
    // frequently, so sending full HTML for every report is wasteful.
    // We return a tiny marker ('1') for html_content/published_html so the frontend
    // can still tell whether content exists (used for status dots) without the payload.
    // Scope to the caller's own reports; admins (?=1) see everything.
    let reports = sqlx::query_as::<_, Report>(
        "SELECT id, title, description, group_id, pool_ids, config, data_cache, status, \
         share_token, share_public, layout_config, \
         CASE WHEN html_content IS NOT NULL THEN '1' END AS html_content, \
         CASE WHEN published_html IS NOT NULL THEN '1' END AS published_html, \
         refresh_interval, generation_status, generation_error, style_key, design_score, \
         report_filters, owner_user_id, created_at, updated_at \
         FROM reports WHERE (owner_user_id = ? OR ? = 1) ORDER BY updated_at DESC",
    )
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(reports))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateReport>,
) -> Result<(StatusCode, Json<Report>), (StatusCode, String)> {
    // Build default visualization config based on pool count
    let vis_types = vec!["kpi", "bar", "line"];
    let visualizations: Vec<serde_json::Value> = payload
        .pool_ids
        .iter()
        .enumerate()
        .map(|(i, pid)| {
            serde_json::json!({
                "type": vis_types[i % vis_types.len()],
                "title": payload.visualization_intent
                    .as_deref()
                    .unwrap_or(&format!("Chart {}", i + 1))
                    .to_string(),
                "data_pool_id": pid,
                "config": {}
            })
        })
        .collect();

    let config = serde_json::json!({
        "visualizations": visualizations,
        "layout": "grid"
    });

    let pool_ids_json = serde_json::to_value(&payload.pool_ids)
        .map_err(crate::routes::internal_error)?;

    let result = sqlx::query(
        "INSERT INTO reports (title, description, pool_ids, config, group_id, owner_user_id) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&payload.title)
    .bind(&payload.description)
    .bind(&pool_ids_json)
    .bind(&config)
    .bind(payload.group_id)
    .bind(user.id)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(report)))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<Report>, (StatusCode, String)> {
    let report = load_owned_report(&state, id, &user).await?;
    Ok(Json(report))
}

pub async fn render(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(body): Json<RenderRequest>,
) -> Result<Json<Report>, (StatusCode, String)> {
    // Validate report exists and is owned by the caller.
    let report = load_owned_report(&state, id, &user).await?;

    if let Some(prompt) = body.prompt.clone() {
        // Surface the metrics' declared parameters as report-level filter
        // controls, so the generated dashboard gets interactive conditions wired
        // to the `{{param}}` placeholders. Only adds missing ones — never
        // clobbers controls the user configured.
        sync_param_filters(&state, id).await;

        // Optionally load a saved theme to generate in (full row incl. sample_html).
        let theme = if let Some(theme_id) = body.theme_id {
            let t = sqlx::query_as::<_, ReportTheme>(
                "SELECT id, name, description, style_prompt, sample_html, emoji, \
                 source_report_id, owner_user_id, created_at, updated_at FROM report_themes WHERE id = ?",
            )
            .bind(theme_id)
            .fetch_optional(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
            // You can only generate with a theme you own.
            if let Some(t) = &t {
                ensure_owner(&user, t.owner_user_id)?;
            }
            t
        } else {
            None
        };

        // Mark as generating and kick off a background task.
        // This allows the client to navigate away without interrupting generation.
        sqlx::query("UPDATE reports SET generation_status = 'generating', generation_error = NULL WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;

        let state_clone = Arc::clone(&state);
        let report_clone = report.clone();
        tokio::spawn(async move {
            match generate_html_dashboard(&state_clone, &report_clone, &prompt, theme.as_ref()).await {
                Ok(html) => {
                    let _ = sqlx::query(
                        "UPDATE reports SET html_content = ?, generation_status = 'done', generation_error = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
                    )
                    .bind(&html)
                    .bind(id)
                    .execute(&state_clone.db)
                    .await;

                    // Save version snapshot
                    let next_version: (i64,) = sqlx::query_as(
                        "SELECT COALESCE(MAX(version), 0) + 1 FROM report_versions WHERE report_id = ?"
                    ).bind(id).fetch_one(&state_clone.db).await.unwrap_or((1,));
                    let _ = sqlx::query(
                        "INSERT INTO report_versions (report_id, version, html_content, prompt, style_key) VALUES (?, ?, ?, ?, ?)"
                    )
                    .bind(id)
                    .bind(next_version.0 as i32)
                    .bind(&html)
                    .bind(&prompt)
                    .bind(report_clone.style_key.as_deref())
                    .execute(&state_clone.db)
                    .await;

                    // Score the design asynchronously
                    let db = state_clone.db.clone();
                    let html_clone = html.clone();
                    let achievements_user = report_clone.owner_user_id.unwrap_or(1);
                    tokio::spawn(async move {
                        score_report_design(&db, id, &html_clone).await;
                        crate::routes::achievements::check_achievements(&db, achievements_user).await;
                    });
                }
                Err((_, err_msg)) => {
                    let _ = sqlx::query(
                        "UPDATE reports SET generation_status = 'failed', generation_error = ? WHERE id = ?"
                    )
                    .bind(&err_msg)
                    .bind(id)
                    .execute(&state_clone.db)
                    .await;
                }
            }
        });
    } else if let Some(config) = &body.config {
        // Manual config update (legacy)
        let new_config = serde_json::to_value(config)
            .map_err(crate::routes::internal_error)?;
        sqlx::query("UPDATE reports SET config = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(&new_config)
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
    }

    let updated = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(updated))
}

/// Get the generation status of a report (for async polling).
pub async fn get_status(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let row: Option<(Option<String>, Option<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT generation_status, generation_error, updated_at FROM reports WHERE id = ? AND (owner_user_id = ? OR ? = 1)"
    )
    .bind(id)
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    match row {
        Some((status, error, updated_at)) => Ok(Json(serde_json::json!({
            "status": status.unwrap_or_else(|| "idle".to_string()),
            "error": error,
            "updated_at": updated_at,
        }))),
        None => Err((StatusCode::NOT_FOUND, "Report not found".to_string())),
    }
}

/// Generate a complete HTML dashboard page using LLM.
async fn generate_html_dashboard(
    state: &AppState,
    report: &Report,
    prompt: &str,
    theme: Option<&ReportTheme>,
) -> Result<String, (StatusCode, String)> {
    // Load LLM config
    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "LLM not configured".to_string()))?;

    if llm_cfg.api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "API key not configured".to_string()));
    }

    let client = LlmClient::new(llm_cfg.base_url, llm_cfg.api_key, llm_cfg.model);

    // Build data context from report datasources
    let report_ds: Vec<ReportDataSource> = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE report_id = ?"
    )
    .bind(report.id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let mut data_context = String::new();
    data_context.push_str(&format!("Report ID: {}\n\n", report.id));
    for ds in &report_ds {
        data_context.push_str(&format!("### {} (id={})\n", ds.name, ds.id));
        data_context.push_str(&format!("SQL: {}\n", ds.sql_query));
        if let Some(cache) = &ds.result_cache {
            if let Some(rows) = cache.as_array() {
                data_context.push_str(&format!("Rows: {}\n", rows.len()));
                // Include all data (JSON) for the AI to embed
                let json_str = serde_json::to_string_pretty(cache).unwrap_or_default();
                // Limit to avoid token overflow
                if json_str.len() < 8000 {
                    data_context.push_str(&format!("Data (JSON):\n{}\n\n", json_str));
                } else {
                    // Truncate to first 50 rows
                    let truncated: Vec<&serde_json::Value> = rows.iter().take(50).collect();
                    let trunc_str = serde_json::to_string_pretty(&truncated).unwrap_or_default();
                    data_context.push_str(&format!("Data (first 50 rows):\n{}\n\n", trunc_str));
                }
            }
        }
    }

    // If no datasources, try loading from data_pools or metrics (same as before)
    if data_context.is_empty() {
        // Try metrics
        let metrics_data: Vec<MetricPool> = sqlx::query_as::<_, MetricPool>(
            "SELECT * FROM metric_pools ORDER BY group_id, id LIMIT 10"
        )
        .fetch_all(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

        for m in &metrics_data {
            data_context.push_str(&format!("### {} (metric_id={})\n", m.name, m.id));
            data_context.push_str(&format!("SQL: {}\n", m.sql_query));
            if let Some(cache) = &m.result_cache {
                let json_str = serde_json::to_string_pretty(cache).unwrap_or_default();
                if json_str.len() < 5000 {
                    data_context.push_str(&format!("Data:\n{}\n\n", json_str));
                }
            }
        }
    }

    if data_context.is_empty() {
        // Last resort: generate from schema
        let schema_context = crate::routes::chat::build_kg_context(state, None, "").await;
        if schema_context.contains("No schema") {
            return Err((StatusCode::BAD_REQUEST, "No data available. Add data sources or metrics first.".to_string()));
        }
        data_context = format!("No pre-computed data available. Here is the database schema — generate sample/mock data for the visualization:\n{}", schema_context);
    }

    // Choose prompt: a selected theme takes precedence (generates in that theme);
    // otherwise refine an existing dashboard, or create a fresh one.
    let system = if let Some(theme) = theme {
        prompts::html_theme_prompt(
            &data_context,
            theme.style_prompt.as_deref(),
            theme.sample_html.as_deref(),
        )
    } else if let Some(existing_html) = &report.html_content {
        if !existing_html.is_empty() {
            prompts::html_refine_prompt(existing_html, &data_context)
        } else {
            prompts::html_dashboard_prompt(&data_context)
        }
    } else {
        prompts::html_dashboard_prompt(&data_context)
    };

    use crate::llm::ChatMessage;
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: prompt.to_string(),
        reasoning_content: None,
    }];

    // Call LLM — get raw text response (not JSON)
    let start = std::time::Instant::now();
    let result = client
        .chat_oneshot(&messages, &system, 65536, llm_cfg.temperature)
        .await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match &result {
        Ok(content) => {
            crate::ai_log::log_ai_request(
                &state.db, "html_generation", &client.model,
                duration_ms, "success", None,
                Some(&format!("report_id={}, prompt={}", report.id, prompt)),
                Some(&system),
                Some(content),
            ).await;
            tracing::info!("AI html_generation OK: {}ms, {} chars", duration_ms, content.len());
        }
        Err(e) => {
            crate::ai_log::log_ai_request(
                &state.db, "html_generation", &client.model,
                duration_ms, "failed", Some(e),
                Some(&format!("report_id={}, prompt={}", report.id, prompt)),
                Some(&system),
                None,
            ).await;
            tracing::error!("AI html_generation FAILED: {}ms, {}", duration_ms, e);
        }
    }

    let full = result.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM failed: {}", e)))?;

    // Extract HTML from response (strip markdown fences if present)
    let html = extract_html(&full);

    if html.is_empty() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "AI returned empty HTML".to_string()));
    }

    Ok(html)
}

/// Extract HTML content from LLM response, stripping markdown fences and reasoning text.
fn extract_html(text: &str) -> String {
    let text = text.trim();
    // Strip ```html ... ``` fences
    let text = if text.starts_with("```html") {
        let inner = &text[7..];
        if let Some(end) = inner.rfind("```") {
            inner[..end].trim()
        } else {
            inner.trim()
        }
    } else if text.starts_with("```") {
        let inner = &text[3..];
        if let Some(end) = inner.rfind("```") {
            inner[..end].trim()
        } else {
            inner.trim()
        }
    } else {
        text
    };

    // Find the start of HTML (<!DOCTYPE or <html)
    let start = text.find("<!DOCTYPE")
        .or_else(|| text.find("<!doctype"))
        .or_else(|| text.find("<html"))
        .unwrap_or(0);

    let html = &text[start..];

    // Find the end of HTML (</html>)
    if let Some(end_pos) = html.rfind("</html>") {
        return html[..end_pos + 7].to_string();
    }
    if let Some(end_pos) = html.rfind("</HTML>") {
        return html[..end_pos + 7].to_string();
    }

    html.to_string()
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;

    let result = sqlx::query("DELETE FROM reports WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Report not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Publish or unpublish a report.
pub async fn publish(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<PublishReport>,
) -> Result<Json<Report>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;
    let status = if payload.status == "published" { "published" } else { "draft" };

    if status == "published" {
        // Copy current html_content to published_html (snapshot the current version)
        sqlx::query("UPDATE reports SET status = 'published', published_html = html_content, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
    } else {
        sqlx::query("UPDATE reports SET status = 'draft', updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
    }

    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(report))
}

/// Rollback html_content to the last published version.
pub async fn rollback(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<Report>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;
    // Copy published_html back to html_content
    sqlx::query("UPDATE reports SET html_content = published_html, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(report))
}

/// Generate or update share link for a report.
pub async fn share(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<ShareReport>,
) -> Result<Json<ShareInfo>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;
    // Check if token already exists
    let existing: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT share_token FROM reports WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let token = match existing {
        Some((Some(t),)) if !t.is_empty() => t,
        _ => uuid::Uuid::new_v4().to_string().replace("-", ""),
    };

    // Update the share link. Enabling a public link also publishes the report
    // (publishing is the public-visibility gate, so a freshly-shared link is
    // live right away). We snapshot into published_html only if there isn't one
    // yet, so an existing approved snapshot is preserved — use the "Publish"
    // button to push later edits. Turning sharing OFF leaves publish state alone.
    if payload.public {
        sqlx::query(
            "UPDATE reports SET share_token = ?, share_public = 1, \
             status = 'published', \
             published_html = COALESCE(published_html, html_content), \
             updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(&token)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;
    } else {
        sqlx::query("UPDATE reports SET share_token = ?, share_public = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(&token)
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
    }

    Ok(Json(ShareInfo {
        url: format!("/share/{}", token),
        share_token: token,
        public: payload.public,
    }))
}

/// View a shared report (public access).
pub async fn view_shared(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<Json<Report>, (StatusCode, String)> {
    let report = sqlx::query_as::<_, Report>(
        "SELECT * FROM reports WHERE share_token = ? AND share_public = 1",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Report not found or not public".to_string()))?;

    Ok(Json(report))
}

/// Serve the raw HTML content of a report (for iframe embedding).
/// Injects the persisted refresh interval into the page.
/// With `?preview=1`, injects a guard that prevents live-data fetching and polling —
/// the page renders only its embedded (static) data. Used for list-page thumbnails
/// so opening the reports list doesn't re-execute every report's SQL queries.
/// Rewrite a stored report's HTML so it renders fast in mainland China without
/// slow/blocked external resources: serve ECharts from our own origin and route
/// Google Fonts through Google's official China endpoints
/// (fonts.googleapis.cn / fonts.gstatic.cn).
fn localize_report_html(html: &str) -> String {
    use std::sync::OnceLock;
    static ECHARTS: OnceLock<regex::Regex> = OnceLock::new();

    let echarts = ECHARTS.get_or_init(|| {
        regex::Regex::new(r#"https?://[^"'\s>]*?echarts[^"'\s>]*?\.min\.js"#).unwrap()
    });

    let s = echarts.replace_all(html, "/vendor/echarts.min.js").into_owned();
    // Google Fonts -> official China endpoints (keeps any requested font family working).
    s.replace("fonts.googleapis.com", "fonts.googleapis.cn")
        .replace("fonts.gstatic.com", "fonts.gstatic.cn")
}

/// Neutral page shown at a public share link when the report is not currently
/// published (never published, or taken offline via "unpublish").
fn shared_offline_page() -> String {
    r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Report unavailable</title></head>
<body style="margin:0;background:#0b0b11;color:#9aa0ab;display:flex;align-items:center;justify-content:center;min-height:100vh;font-family:Inter,system-ui,sans-serif">
<div style="text-align:center;max-width:420px;padding:32px">
  <div style="font-size:40px;margin-bottom:16px">📊</div>
  <h1 style="font-size:16px;font-weight:600;color:#e8e8ec;margin:0 0 8px">报表当前不可用 · Report unavailable</h1>
  <p style="font-size:13px;line-height:1.6;margin:0">该报表尚未发布或已下线。<br>This report is not currently published.</p>
</div></body></html>"#
        .to_string()
}

pub async fn get_html(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<axum::response::Html<String>, (StatusCode, String)> {
    let preview = params.get("preview").map(|v| v == "1" || v == "true").unwrap_or(false);

    let report = load_owned_report(&state, id, &user).await?;

    let mut html = report.html_content.unwrap_or_else(|| {
        "<html><body style='background:#0d0d14;color:#9ca3af;display:flex;align-items:center;justify-content:center;height:100vh;font-family:system-ui'><p>No HTML content generated yet. Use AI to generate a dashboard.</p></body></html>".to_string()
    });
    html = localize_report_html(&html);

    if preview {
        // Preview mode: neutralize live-data fetching so the thumbnail renders
        // only the embedded data — no SQL re-execution, no polling.
        // Override fetch early (in <head>) so the report's refreshData() becomes a no-op.
        let guard = r#"<script>(function(){var _f=window.fetch;window.fetch=function(u){if(typeof u==='string'&&u.indexOf('/data')!==-1){return Promise.reject(new Error('preview'));}return _f.apply(this,arguments);};var _si=window.setInterval;window.setInterval=function(fn,ms){if(ms>=1000)return 0;return _si.apply(this,arguments);};})();</script>"#;
        if let Some(pos) = html.find("<head>") {
            html.insert_str(pos + "<head>".len(), guard);
        } else if let Some(pos) = html.find("<html>") {
            html.insert_str(pos + "<html>".len(), guard);
        } else {
            html.insert_str(0, guard);
        }
        return Ok(axum::response::Html(html));
    }

    // Inject the persisted refresh interval (replace default 60000ms if present)
    let interval_ms = (report.refresh_interval.unwrap_or(1) as u64) * 60 * 1000;
    // Try to replace the default interval in the generated code
    html = html.replace("setInterval(refreshData, 60000)", &format!("setInterval(refreshData, {})", interval_ms));
    // Also inject a script at the end to override if the pattern doesn't match
    if let Some(pos) = html.rfind("</body>") {
        let inject = format!(
            r#"<script>if(typeof refreshTimer!=='undefined'){{clearInterval(refreshTimer);refreshTimer=setInterval(refreshData,{});}}</script>"#,
            interval_ms
        );
        html.insert_str(pos, &inject);
    }

    // Inject the interactive filter bar (from the report's global filters). The
    // head script wraps fetch so /data calls carry current control values; the
    // bar HTML goes at the top of <body>. The (LLM-generated) charts re-render
    // via their existing refreshData() when a control changes.
    if let Some((head_js, body_html)) = render_report_filter_bar(&report.report_filters) {
        if let Some(pos) = html.find("<head>") {
            html.insert_str(pos + "<head>".len(), &head_js);
        } else if let Some(pos) = html.find("<html>") {
            html.insert_str(pos + "<html>".len(), &head_js);
        } else {
            html.insert_str(0, &head_js);
        }
        // <body> may carry attributes (e.g. <body class="...">), so insert
        // right after the tag's closing '>'.
        if let Some(bstart) = html.find("<body") {
            if let Some(gt) = html[bstart..].find('>') {
                html.insert_str(bstart + gt + 1, &body_html);
            } else {
                html.insert_str(0, &body_html);
            }
        } else {
            html.insert_str(0, &body_html);
        }
    }

    // The page's live-data fetch (/api/reports/{id}/data) runs inside the iframe and
    // cannot set an Authorization header. Forward the token (passed to this endpoint
    // via ?token=) by wrapping fetch to append it to same-origin /api/ requests.
    if let Some(token) = params.get("token") {
        if !token.is_empty() {
            let token_js = token.replace('\\', "").replace('"', "");
            let wrapper = format!(
                r#"<script>(function(){{var _t="{}";var _f=window.fetch;window.fetch=function(u,o){{try{{if(typeof u==='string'&&u.indexOf('/api/')!==-1&&u.indexOf('token=')===-1){{u+=(u.indexOf('?')!==-1?'&':'?')+'token='+encodeURIComponent(_t);}}}}catch(e){{}}return _f.call(this,u,o);}};}})();</script>"#,
                token_js
            );
            if let Some(pos) = html.find("<head>") {
                html.insert_str(pos + "<head>".len(), &wrapper);
            } else if let Some(pos) = html.find("<html>") {
                html.insert_str(pos + "<html>".len(), &wrapper);
            } else {
                html.insert_str(0, &wrapper);
            }
        }
    }

    Ok(axum::response::Html(html))
}

/// Serve shared report HTML directly.
pub async fn view_shared_html(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<axum::response::Html<String>, (StatusCode, String)> {
    let report = sqlx::query_as::<_, Report>(
        "SELECT * FROM reports WHERE share_token = ? AND share_public = 1",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Report not found or not public".to_string()))?;

    // Publishing is the public-visibility gate: only a *published* report serves
    // its approved snapshot (published_html). Draft reports, or ones taken
    // offline via "unpublish", show a neutral offline notice instead of content.
    let is_published = report.status.as_deref() == Some("published");
    let source = if is_published {
        report.published_html.clone().or_else(|| report.html_content.clone())
    } else {
        None
    };
    let mut html = match source {
        Some(h) => localize_report_html(&h),
        None => return Ok(axum::response::Html(shared_offline_page())),
    };

    // Inject persisted refresh interval
    let interval_ms = (report.refresh_interval.unwrap_or(1) as u64) * 60 * 1000;
    html = html.replace("setInterval(refreshData, 60000)", &format!("setInterval(refreshData, {})", interval_ms));
    if let Some(pos) = html.rfind("</body>") {
        let inject = format!(
            r#"<script>if(typeof refreshTimer!=='undefined'){{clearInterval(refreshTimer);refreshTimer=setInterval(refreshData,{});}}</script>"#,
            interval_ms
        );
        html.insert_str(pos, &inject);
    }

    Ok(axum::response::Html(html))
}

/// HTML-attribute escaping for injected filter control values.
fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render a stored filter value as a plain string for prefilling a control.
fn filter_value_display(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Array(a) => a
            .iter()
            .map(filter_value_display)
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

/// Build the interactive filter bar for a rendered dashboard from the report's
/// global filter controls. Returns `(head_js, body_html)`:
/// - `head_js` wraps `window.fetch` so every `/data` request carries the current
///   control values as `f_<key>` params, and exposes apply/reset helpers.
/// - `body_html` is the sticky bar with one control per filter.
///
/// Returns None when the report has no global filters. This injection owns the
/// filter wiring, so the (LLM-generated) dashboard body doesn't need to know
/// anything about filters — it just keeps fetching `/data` as usual.
fn render_report_filter_bar(report_filters: &Option<serde_json::Value>) -> Option<(String, String)> {
    let filters: Vec<ReportFilter> = serde_json::from_value(report_filters.clone()?).ok()?;
    if filters.is_empty() {
        return None;
    }

    let input_style = "background:rgba(255,255,255,0.06);border:1px solid rgba(255,255,255,0.14);border-radius:6px;padding:3px 7px;font:12px system-ui,sans-serif;color:#e5e7eb;min-width:90px";
    let btn_style = "background:rgba(245,158,11,0.15);border:1px solid rgba(245,158,11,0.4);color:#f59e0b;border-radius:6px;padding:3px 12px;font:12px system-ui,sans-serif;cursor:pointer";
    let ghost_btn = "background:transparent;border:1px solid rgba(255,255,255,0.14);color:#9ca3af;border-radius:6px;padding:3px 10px;font:12px system-ui,sans-serif;cursor:pointer";

    let mut items = String::new();
    for f in &filters {
        let op = f.op.trim().to_uppercase();
        let label = esc_attr(f.label.as_deref().unwrap_or(&f.key));
        let key = esc_attr(&f.key);
        let op_attr = esc_attr(&op);

        let control = if op == "BETWEEN" {
            let (from, to) = match &f.value {
                serde_json::Value::Array(a) if a.len() == 2 => {
                    (filter_value_display(&a[0]), filter_value_display(&a[1]))
                }
                _ => (String::new(), String::new()),
            };
            format!(
                r#"<input class="f-from" type="text" value="{}" placeholder="起" onchange="__applyReportFilters__()" style="{}"/>
<span style="color:#6b7280">~</span>
<input class="f-to" type="text" value="{}" placeholder="止" onchange="__applyReportFilters__()" style="{}"/>"#,
                esc_attr(&from), input_style, esc_attr(&to), input_style
            )
        } else {
            let placeholder = if op == "IN" { "逗号分隔" } else { "" };
            format!(
                r#"<input class="f-val" type="text" value="{}" placeholder="{}" onchange="__applyReportFilters__()" style="{}"/>"#,
                esc_attr(&filter_value_display(&f.value)),
                placeholder,
                input_style
            )
        };

        items.push_str(&format!(
            r#"<div class="rf-item" data-filter-key="{}" data-filter-op="{}" style="display:flex;align-items:center;gap:5px">
<label style="color:#9ca3af;font-size:11px">{}</label>{}</div>"#,
            key, op_attr, label, control
        ));
    }

    let body_html = format!(
        r#"<div id="__report_filter_bar__" style="position:sticky;top:0;z-index:99999;display:flex;flex-wrap:wrap;gap:12px;align-items:center;padding:8px 14px;background:rgba(13,13,20,0.92);backdrop-filter:blur(6px);border-bottom:1px solid rgba(255,255,255,0.08)">
<span style="font-weight:600;color:#f59e0b;font:12px system-ui,sans-serif">筛选</span>
{items}
<button onclick="__applyReportFilters__()" style="{btn_style}">应用</button>
<button onclick="__resetReportFilters__()" style="{ghost_btn}">重置</button>
</div>"#
    );

    let head_js = r#"<script>(function(){
function collect(){
  var qs=[];
  var items=document.querySelectorAll('#__report_filter_bar__ .rf-item');
  items.forEach(function(el){
    var key=el.getAttribute('data-filter-key');
    var op=el.getAttribute('data-filter-op');
    var val='';
    if(op==='BETWEEN'){
      var a=el.querySelector('.f-from'); var b=el.querySelector('.f-to');
      var f=a?a.value.trim():''; var t=b?b.value.trim():'';
      val=(f&&t)?(f+','+t):'';
    } else {
      var i=el.querySelector('.f-val'); val=i?i.value.trim():'';
    }
    qs.push('f_'+encodeURIComponent(key)+'='+encodeURIComponent(val));
  });
  return qs.join('&');
}
var _f=window.fetch;
window.fetch=function(u,o){
  try{
    if(typeof u==='string' && u.indexOf('/data')!==-1){
      var extra=collect();
      if(extra) u+=(u.indexOf('?')!==-1?'&':'?')+extra;
    }
  }catch(e){}
  return _f.call(this,u,o);
};
window.__applyReportFilters__=function(){ if(typeof refreshData==='function'){try{refreshData();}catch(e){}} };
window.__resetReportFilters__=function(){
  document.querySelectorAll('#__report_filter_bar__ input').forEach(function(i){i.value='';});
  window.__applyReportFilters__();
};
})();</script>"#.to_string();

    Some((head_js, body_html))
}

/// Coerce a raw string token to a JSON value: number when numeric, else string.
/// Mirrors the client's coercion so numeric comparisons stay numeric.
fn coerce_runtime_token(raw: &str) -> serde_json::Value {
    let s = raw.trim();
    if s.is_empty() {
        return serde_json::Value::String(String::new());
    }
    if let Ok(i) = s.parse::<i64>() {
        return serde_json::Value::Number(i.into());
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return serde_json::Value::Number(n);
        }
    }
    serde_json::Value::String(s.to_string())
}

/// Build the runtime value for a control from its `f_<key>` request param,
/// shaped for the control's operator (array for IN, [min,max] for BETWEEN,
/// scalar otherwise). Comma-separates multi-value inputs.
fn runtime_value_for_op(op: &str, raw: &str) -> serde_json::Value {
    match op.trim().to_uppercase().as_str() {
        "IN" => serde_json::Value::Array(
            raw.split(',')
                .map(|p| coerce_runtime_token(p))
                .filter(|v| !matches!(v, serde_json::Value::String(s) if s.is_empty()))
                .collect(),
        ),
        "BETWEEN" => {
            let parts: Vec<&str> = raw.split(',').collect();
            if parts.len() == 2 {
                serde_json::Value::Array(vec![
                    coerce_runtime_token(parts[0]),
                    coerce_runtime_token(parts[1]),
                ])
            } else {
                // Not a complete range → blank so the control is treated inactive.
                serde_json::Value::String(String::new())
            }
        }
        _ => coerce_runtime_token(raw),
    }
}

/// Produce a report_filters JSON where each control's `value` is replaced by the
/// matching `f_<key>` request param, when present. Controls with no param keep
/// their stored value. Returns None when there are no stored filters.
fn apply_runtime_filter_values(
    stored: &Option<serde_json::Value>,
    params: &HashMap<String, String>,
) -> Option<serde_json::Value> {
    let stored = stored.as_ref()?;
    let mut filters: Vec<ReportFilter> = serde_json::from_value(stored.clone()).ok()?;
    for f in &mut filters {
        if let Some(raw) = params.get(&format!("f_{}", f.key)) {
            f.value = runtime_value_for_op(&f.op, raw);
        }
    }
    serde_json::to_value(&filters).ok()
}

/// Ensure the report has a global filter control for each parameter declared by
/// its datasets' metrics. Existing controls (by key) are preserved; only missing
/// param controls are appended. Param controls carry no column `targets` — their
/// value flows into the metric's `{{param}}` placeholder by name at query time.
async fn sync_param_filters(state: &AppState, report_id: i32) {
    // Collect distinct metric ids used by the report's datasets.
    let metric_ids: Vec<(Option<i32>,)> = sqlx::query_as(
        "SELECT DISTINCT metric_id FROM report_datasources WHERE report_id = ? AND metric_id IS NOT NULL",
    )
    .bind(report_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Gather declared params across those metrics.
    let mut params: Vec<MetricParam> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (mid,) in metric_ids.into_iter().flat_map(|(m,)| m.map(|x| (x,))) {
        let row: Option<(Option<serde_json::Value>,)> =
            sqlx::query_as("SELECT params FROM metric_pools WHERE id = ?")
                .bind(mid)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();
        if let Some((Some(v),)) = row {
            if let Ok(defs) = serde_json::from_value::<Vec<MetricParam>>(v) {
                for d in defs {
                    if seen.insert(d.name.clone()) {
                        params.push(d);
                    }
                }
            }
        }
    }
    if params.is_empty() {
        return;
    }

    // Load existing controls and index by key.
    let existing_row: Option<(Option<serde_json::Value>,)> =
        sqlx::query_as("SELECT report_filters FROM reports WHERE id = ?")
            .bind(report_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let mut controls: Vec<ReportFilter> = existing_row
        .and_then(|(v,)| v)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let have: std::collections::HashSet<String> = controls.iter().map(|c| c.key.clone()).collect();

    let mut changed = false;
    for p in params {
        if have.contains(&p.name) {
            continue;
        }
        controls.push(ReportFilter {
            key: p.name.clone(),
            label: Some(p.label.unwrap_or_else(|| p.name.clone())),
            // Scalar control: its value fills the metric's {{param}} placeholder.
            op: "=".to_string(),
            value: p.default.unwrap_or(serde_json::Value::String(String::new())),
            targets: Vec::new(),
        });
        changed = true;
    }

    if changed {
        if let Ok(v) = serde_json::to_value(&controls) {
            let _ = sqlx::query("UPDATE reports SET report_filters = ? WHERE id = ?")
                .bind(&v)
                .bind(report_id)
                .execute(&state.db)
                .await;
        }
    }
}

/// Default parameter values for a report dataset's linked metric (if any), used
/// to resolve `{{param}}` placeholders when no runtime value is supplied.
async fn metric_param_defaults(
    state: &AppState,
    metric_id: Option<i32>,
) -> HashMap<String, serde_json::Value> {
    let Some(mid) = metric_id else { return HashMap::new() };
    let row: Option<(Option<serde_json::Value>,)> =
        sqlx::query_as("SELECT params FROM metric_pools WHERE id = ?")
            .bind(mid)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    crate::routes::query::param_defaults(&row.and_then(|(p,)| p))
}

/// Return live data for a report's datasources (re-executes SQL queries).
/// This endpoint is called by the HTML page inside the iframe to get fresh data.
pub async fn get_live_data(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let report = load_owned_report(&state, id, &user).await?;

    let report_ds: Vec<ReportDataSource> = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE report_id = ?"
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    // Runtime filter values from the request override the stored global-filter
    // values (interactive controls in the rendered dashboard pass `f_<key>`).
    // Passing None/empty leaves the stored values in effect.
    let effective_report_filters = apply_runtime_filter_values(&report.report_filters, &params);

    // The same `f_<name>` request params also feed metric SQL placeholders
    // ({{name}} / [[ ]] blocks). Coerce each to a scalar value keyed by name.
    let request_param_values: HashMap<String, serde_json::Value> = params
        .iter()
        .filter_map(|(k, v)| k.strip_prefix("f_").map(|name| (name.to_string(), coerce_runtime_token(v))))
        .collect();

    let mut results = Vec::new();

    for ds in &report_ds {
        // Re-execute the SQL query to get fresh data
        let ds_info = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
            .bind(ds.datasource_id)
            .fetch_optional(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;

        let fresh_data = if let Some(source) = ds_info {
            // Combine the dataset's own filters with report-level global
            // controls that target it. No effective filters => metric SQL runs
            // unchanged.
            let ds_filters: Vec<FilterCondition> = ds
                .filters
                .as_ref()
                .and_then(|v| serde_json::from_value::<Vec<FilterCondition>>(v.clone()).ok())
                .unwrap_or_default();
            let filters = crate::routes::query::combined_filters(
                ds_filters,
                &effective_report_filters,
                ds.datasource_id,
            );
            // Metric parameter values: the metric's defaults overlaid with any
            // runtime request values (by param name).
            let mut param_values = metric_param_defaults(&state, ds.metric_id).await;
            for (k, v) in &request_param_values {
                param_values.insert(k.clone(), v.clone());
            }
            match crate::routes::query::execute_metric_sql(&state, &source, &ds.sql_query, &param_values, &filters).await {
                Ok(result) => serde_json::to_value(&result.rows).unwrap_or(serde_json::Value::Array(vec![])),
                Err(_) => ds.result_cache.clone().unwrap_or(serde_json::Value::Array(vec![])),
            }
        } else {
            ds.result_cache.clone().unwrap_or(serde_json::Value::Array(vec![]))
        };

        results.push(serde_json::json!({
            "id": ds.id,
            "name": ds.name,
            "data": fresh_data,
        }));
    }

    Ok(Json(results))
}

/// Set (or clear) the report's global filter controls, then refresh each
/// dataset's cache so previews/thumbnails reflect the new selection. An empty
/// `filters` array clears them and reverts to per-dataset behavior.
pub async fn set_report_filters(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<SetReportFilters>,
) -> Result<Json<Report>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;

    let filters_value: Option<serde_json::Value> = if payload.filters.is_empty() {
        None
    } else {
        serde_json::to_value(&payload.filters).ok()
    };

    sqlx::query("UPDATE reports SET report_filters = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&filters_value)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    // Refresh each dataset's cached result with the combined (dataset + global)
    // filters. Failures are non-fatal — the live-data path still recomputes on
    // view, so a datasource that errors under the new filter just keeps its old
    // cache rather than blocking the whole apply.
    let report_ds: Vec<ReportDataSource> = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE report_id = ?",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    for ds in &report_ds {
        let source = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
            .bind(ds.datasource_id)
            .fetch_optional(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
        let Some(source) = source else { continue };

        let ds_filters: Vec<FilterCondition> = ds
            .filters
            .as_ref()
            .and_then(|v| serde_json::from_value::<Vec<FilterCondition>>(v.clone()).ok())
            .unwrap_or_default();
        let filters = crate::routes::query::combined_filters(
            ds_filters,
            &filters_value,
            ds.datasource_id,
        );

        let param_values = metric_param_defaults(&state, ds.metric_id).await;
        if let Ok(qr) =
            crate::routes::query::execute_metric_sql(&state, &source, &ds.sql_query, &param_values, &filters).await
        {
            let cache = serde_json::to_value(&qr.rows).ok();
            let _ = sqlx::query("UPDATE report_datasources SET result_cache=?, row_count=? WHERE id=?")
                .bind(&cache)
                .bind(qr.row_count as i32)
                .bind(ds.id)
                .execute(&state.db)
                .await;
        }
    }

    let updated = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(updated))
}

/// Debug trace of the SQL a report runs: for each dataset, the actual executed
/// query (base or filter-wrapped), its bound params, timing, row count, and any
/// error. Owner-only; called on demand when the debug panel is opened, so no
/// SQL is executed for this purpose unless the user asks for it.
pub async fn debug_sql(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<ReportDebugEntry>>, (StatusCode, String)> {
    let report = load_owned_report(&state, id, &user).await?;

    let report_ds: Vec<ReportDataSource> = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE report_id = ? ORDER BY created_at ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let mut entries: Vec<ReportDebugEntry> = Vec::new();

    for ds in &report_ds {
        let source = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
            .bind(ds.datasource_id)
            .fetch_optional(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;

        let Some(source) = source else {
            entries.push(ReportDebugEntry {
                dataset_id: ds.id,
                name: ds.name.clone(),
                datasource_id: ds.datasource_id,
                db_type: "unknown".to_string(),
                base_sql: ds.sql_query.clone(),
                effective_sql: ds.sql_query.clone(),
                params: vec![],
                filter_count: 0,
                row_count: ds.row_count.map(|n| n as usize),
                duration_ms: 0,
                executed_at: chrono::Utc::now(),
                error: Some("Data source not found".to_string()),
            });
            continue;
        };

        // Combine dataset filters (方案 B) with report-level controls (方案 C).
        let ds_filters: Vec<FilterCondition> = ds
            .filters
            .as_ref()
            .and_then(|v| serde_json::from_value::<Vec<FilterCondition>>(v.clone()).ok())
            .unwrap_or_default();
        let filters = crate::routes::query::combined_filters(
            ds_filters,
            &report.report_filters,
            ds.datasource_id,
        );
        let filter_count = filters.len();

        // Resolve the exact SQL + binds that will run (param rendering + filter
        // wrap), so the debug view matches execution.
        let param_values = metric_param_defaults(&state, ds.metric_id).await;
        let mut idx = 1usize;
        let (effective_sql, params, build_err) = match crate::routes::query::render_parameterized_sql(
            &ds.sql_query,
            &param_values,
            &source.db_type,
            &mut idx,
        ) {
            Err(e) => (ds.sql_query.clone(), vec![], Some(e)),
            Ok((rendered, mut binds)) => {
                match crate::routes::query::build_filtered_sql_from(&rendered, &filters, &source.db_type, &mut idx) {
                    Ok(Some((wrapped, mut fb))) => {
                        binds.append(&mut fb);
                        (wrapped, binds, None)
                    }
                    Ok(None) => (rendered, binds, None),
                    Err(e) => (ds.sql_query.clone(), vec![], Some(e)),
                }
            }
        };

        // If the filter build failed, report it without executing.
        if let Some(err) = build_err {
            entries.push(ReportDebugEntry {
                dataset_id: ds.id,
                name: ds.name.clone(),
                datasource_id: ds.datasource_id,
                db_type: source.db_type.clone(),
                base_sql: ds.sql_query.clone(),
                effective_sql,
                params,
                filter_count,
                row_count: None,
                duration_ms: 0,
                executed_at: chrono::Utc::now(),
                error: Some(err),
            });
            continue;
        }

        let executed_at = chrono::Utc::now();
        let start = std::time::Instant::now();
        let result = crate::routes::query::execute_metric_sql(
            &state,
            &source,
            &ds.sql_query,
            &param_values,
            &filters,
        )
        .await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let (row_count, error) = match result {
            Ok(qr) => (Some(qr.row_count), None),
            Err(e) => (None, Some(e)),
        };

        entries.push(ReportDebugEntry {
            dataset_id: ds.id,
            name: ds.name.clone(),
            datasource_id: ds.datasource_id,
            db_type: source.db_type.clone(),
            base_sql: ds.sql_query.clone(),
            effective_sql,
            params,
            filter_count,
            row_count,
            duration_ms,
            executed_at,
            error,
        });
    }

    Ok(Json(entries))
}

/// Update the refresh interval for a report.
pub async fn update_refresh_interval(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Report>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;
    let interval = body.get("refresh_interval")
        .and_then(|v| v.as_i64())
        .unwrap_or(1) as i32;

    sqlx::query("UPDATE reports SET refresh_interval = ? WHERE id = ?")
        .bind(interval)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(report))
}

/// Update the style_key for a report.
pub async fn update_style(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Report>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;
    let style_key = body.get("style_key").and_then(|v| v.as_str());

    sqlx::query("UPDATE reports SET style_key = ? WHERE id = ?")
        .bind(style_key)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(report))
}

/// Score the design quality of a generated report HTML.
async fn score_report_design(db: &sqlx::MySqlPool, report_id: i32, html: &str) {
    // Simple heuristic scoring — no LLM call needed, instant
    let mut layout = 5;
    let mut color = 5;
    let mut typography = 5;
    let mut responsiveness = 5;
    let mut data_viz = 5;

    // Layout: check for grid/flexbox usage
    if html.contains("display:grid") || html.contains("display: grid") || html.contains("grid-template") { layout += 2; }
    if html.contains("display:flex") || html.contains("display: flex") { layout += 1; }
    if html.contains("gap:") || html.contains("gap :") { layout += 1; }
    if html.contains("padding") && html.contains("margin") { layout += 1; }

    // Color: check for gradients, multiple colors, proper contrast
    if html.contains("linear-gradient") || html.contains("radial-gradient") { color += 2; }
    if html.contains("rgba") { color += 1; }
    if html.contains("box-shadow") { color += 1; }
    if html.contains("text-shadow") { color += 1; }

    // Typography: check font imports, weight variety, sizing
    if html.contains("font-family") { typography += 1; }
    if html.contains("font-weight:") && (html.contains("300") || html.contains("700") || html.contains("800")) { typography += 2; }
    if html.contains("letter-spacing") { typography += 1; }
    if html.contains("line-height") { typography += 1; }

    // Responsiveness: check for media queries, viewport meta, relative units
    if html.contains("@media") { responsiveness += 3; }
    if html.contains("viewport") { responsiveness += 1; }
    if html.contains("vw") || html.contains("vh") || html.contains("rem") { responsiveness += 1; }

    // Data visualization: check for ECharts config quality
    if html.contains("tooltip") { data_viz += 1; }
    if html.contains("legend") { data_viz += 1; }
    if html.contains("animation") { data_viz += 1; }
    if html.contains("axisLabel") || html.contains("xAxis") { data_viz += 1; }
    if html.contains("setOption") { data_viz += 1; }

    // Cap at 10
    let cap = |v: i32| v.min(10);
    let scores = serde_json::json!({
        "layout": cap(layout),
        "color": cap(color),
        "typography": cap(typography),
        "responsiveness": cap(responsiveness),
        "data_viz": cap(data_viz),
        "total": ((cap(layout) + cap(color) + cap(typography) + cap(responsiveness) + cap(data_viz)) as f32 / 5.0 * 10.0).round() as i32,
    });

    let _ = sqlx::query("UPDATE reports SET design_score = ? WHERE id = ?")
        .bind(&scores)
        .bind(report_id)
        .execute(db)
        .await;
}

// ── Version History ──

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ReportVersion {
    pub id: i32,
    pub report_id: i32,
    pub version: i32,
    pub prompt: Option<String>,
    pub style_key: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// List versions (without HTML content — just metadata).
pub async fn list_versions(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<ReportVersion>>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;
    let versions = sqlx::query_as::<_, ReportVersion>(
        "SELECT id, report_id, version, prompt, style_key, created_at FROM report_versions WHERE report_id = ? ORDER BY version DESC"
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(versions))
}

/// Get HTML content of a specific version (for preview).
pub async fn get_version_html(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((report_id, version_id)): Path<(i32, i32)>,
) -> Result<axum::response::Html<String>, (StatusCode, String)> {
    load_owned_report(&state, report_id, &user).await?;
    let html: Option<(String,)> = sqlx::query_as(
        "SELECT html_content FROM report_versions WHERE report_id = ? AND id = ?"
    )
    .bind(report_id)
    .bind(version_id)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    match html {
        Some((content,)) => Ok(axum::response::Html(localize_report_html(&content))),
        None => Err((StatusCode::NOT_FOUND, "Version not found".to_string())),
    }
}

/// Delete a specific version.
pub async fn delete_version(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((report_id, version_id)): Path<(i32, i32)>,
) -> Result<StatusCode, (StatusCode, String)> {
    load_owned_report(&state, report_id, &user).await?;
    let result = sqlx::query(
        "DELETE FROM report_versions WHERE report_id = ? AND id = ?"
    )
    .bind(report_id)
    .bind(version_id)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Version not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Restore a specific version — copy its HTML to the report's html_content.
pub async fn restore_version(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((report_id, version_id)): Path<(i32, i32)>,
) -> Result<Json<Report>, (StatusCode, String)> {
    load_owned_report(&state, report_id, &user).await?;
    let html: Option<(String,)> = sqlx::query_as(
        "SELECT html_content FROM report_versions WHERE report_id = ? AND id = ?"
    )
    .bind(report_id)
    .bind(version_id)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let content = html.ok_or((StatusCode::NOT_FOUND, "Version not found".to_string()))?.0;

    sqlx::query("UPDATE reports SET html_content = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&content)
        .bind(report_id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(report_id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(report))
}

// ── AI Data Summary ──

/// Build the grounding context for a report summary: the report's data rows
/// (truncated), recent metric snapshots (for trend), and relevant knowledge-base
/// entries (for business context).
async fn build_summary_context(
    state: &AppState,
    report: &Report,
) -> Result<String, (StatusCode, String)> {
    let report_ds: Vec<ReportDataSource> = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE report_id = ?",
    )
    .bind(report.id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    if report_ds.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "This report has no data sources to analyze yet.".to_string(),
        ));
    }

    let mut ctx = format!("Report title: {}\n\n", report.title);
    let mut ds_ids: Vec<i32> = Vec::new();

    for ds in &report_ds {
        ctx.push_str(&format!("### Dataset: {}\nSQL: {}\n", ds.name, ds.sql_query));
        if let Some(cache) = &ds.result_cache {
            if let Some(rows) = cache.as_array() {
                ctx.push_str(&format!("Row count: {}\n", rows.len()));
                let json_str = serde_json::to_string(cache).unwrap_or_default();
                if json_str.len() < 6000 {
                    ctx.push_str(&format!("Data (JSON):\n{}\n", json_str));
                } else {
                    let trunc: Vec<&serde_json::Value> = rows.iter().take(50).collect();
                    ctx.push_str(&format!(
                        "Data (first 50 rows):\n{}\n",
                        serde_json::to_string(&trunc).unwrap_or_default()
                    ));
                }
            }
        }
        if !ds_ids.contains(&ds.datasource_id) {
            ds_ids.push(ds.datasource_id);
        }

        // Trend context: recent snapshots for metric-backed datasources.
        if let Some(metric_id) = ds.metric_id {
            let snaps: Vec<(String, String, Option<serde_json::Value>)> = sqlx::query_as(
                "SELECT period_key, DATE_FORMAT(snapshot_at, '%Y-%m-%d %H:%i') AS at, result_data \
                 FROM metric_snapshots WHERE metric_pool_id = ? ORDER BY snapshot_at DESC LIMIT 6",
            )
            .bind(metric_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

            if !snaps.is_empty() {
                ctx.push_str("Recent snapshots (newest first — use these for trend/YoY/MoM):\n");
                for (period_key, at, data) in &snaps {
                    let mut compact = data
                        .as_ref()
                        .map(|d| serde_json::to_string(d).unwrap_or_default())
                        .unwrap_or_default();
                    if compact.len() > 400 {
                        compact = compact.chars().take(400).collect::<String>() + "…";
                    }
                    ctx.push_str(&format!("- [{} @ {}] {}\n", period_key, at, compact));
                }
            }
        }
        ctx.push('\n');
    }

    // Business context: knowledge-base entries for the involved data sources.
    let mut kb_lines: Vec<String> = Vec::new();
    for ds_id in &ds_ids {
        let entries: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT category, title, content FROM knowledge_base WHERE datasource_id = ? ORDER BY category LIMIT 10",
        )
        .bind(ds_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        for (cat, title, content) in entries {
            kb_lines.push(format!("- [{}] {}: {}", cat, title, content));
            if kb_lines.len() >= 15 {
                break;
            }
        }
        if kb_lines.len() >= 15 {
            break;
        }
    }
    if !kb_lines.is_empty() {
        ctx.push_str("### Business knowledge (definitions & rules to respect)\n");
        ctx.push_str(&kb_lines.join("\n"));
        ctx.push('\n');
    }

    Ok(ctx)
}

/// GET the cached AI summary for a report (if any).
pub async fn get_summary(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    load_owned_report(&state, id, &user).await?;
    let row: Option<(serde_json::Value, String, String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            "SELECT summary, model, lang, updated_at FROM report_summaries WHERE report_id = ?",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    match row {
        Some((summary, model, lang, updated_at)) => Ok(Json(serde_json::json!({
            "summary": summary,
            "model": model,
            "lang": lang,
            "updated_at": updated_at,
        }))),
        None => Ok(Json(serde_json::json!({ "summary": null }))),
    }
}

/// POST — (re)generate the AI data summary for a report and cache it.
pub async fn generate_summary(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(body): Json<GenerateSummaryRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let report = load_owned_report(&state, id, &user).await?;

    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "LLM not configured".to_string()))?;
    if llm_cfg.api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "LLM API key not configured".to_string()));
    }

    let lang = body.lang.unwrap_or_else(|| "zh".to_string());
    let data_context = build_summary_context(&state, &report).await?;
    let system = prompts::data_summary_prompt(&data_context, &lang);

    let client = LlmClient::new(llm_cfg.base_url, llm_cfg.api_key, llm_cfg.model);
    use crate::llm::ChatMessage;
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: "Generate the data analysis summary now.".to_string(),
        reasoning_content: None,
    }];

    let start = std::time::Instant::now();
    let result = client
        .generate_json::<DataSummary>(&messages, &system, llm_cfg.max_tokens.max(2048), llm_cfg.temperature)
        .await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(summary) => {
            let summary_json = serde_json::to_value(&summary).unwrap_or(serde_json::json!({}));
            crate::ai_log::log_ai_request(
                &state.db, "data_summary", &client.model,
                duration_ms, "success", None,
                Some(&format!("report_id={}", id)),
                Some(&system),
                Some(&serde_json::to_string(&summary).unwrap_or_default()),
            ).await;

            let _ = sqlx::query(
                "INSERT INTO report_summaries (report_id, summary, model, lang) VALUES (?, ?, ?, ?) \
                 ON DUPLICATE KEY UPDATE summary = VALUES(summary), model = VALUES(model), \
                 lang = VALUES(lang), updated_at = CURRENT_TIMESTAMP",
            )
            .bind(id)
            .bind(&summary_json)
            .bind(&client.model)
            .bind(&lang)
            .execute(&state.db)
            .await;

            Ok(Json(serde_json::json!({
                "summary": summary_json,
                "model": client.model,
                "lang": lang,
                "updated_at": chrono::Utc::now(),
            })))
        }
        Err(e) => {
            crate::ai_log::log_ai_request(
                &state.db, "data_summary", &client.model,
                duration_ms, "failed", Some(&e),
                Some(&format!("report_id={}", id)),
                Some(&system),
                None,
            ).await;
            Err((StatusCode::BAD_GATEWAY, format!("AI summary failed: {}", e)))
        }
    }
}

/// POST — answer a grounded question about a report's data (Q&A over the report).
pub async fn ask_report(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(body): Json<ReportQaRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let question = body.question.trim();
    if question.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Question is empty".to_string()));
    }

    let report = load_owned_report(&state, id, &user).await?;

    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "LLM not configured".to_string()))?;
    if llm_cfg.api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "LLM API key not configured".to_string()));
    }

    let lang = body.lang.clone().unwrap_or_else(|| "zh".to_string());
    let data_context = build_summary_context(&state, &report).await?;
    let system = prompts::report_qa_prompt(&data_context, &lang);

    let client = LlmClient::new(llm_cfg.base_url, llm_cfg.api_key, llm_cfg.model);
    use crate::llm::ChatMessage;

    // Carry the recent conversation (capped) so follow-ups have context.
    let mut messages: Vec<ChatMessage> = Vec::new();
    let history_start = body.history.len().saturating_sub(8);
    for m in &body.history[history_start..] {
        let role = if m.role == "assistant" { "assistant" } else { "user" };
        messages.push(ChatMessage {
            role: role.to_string(),
            content: m.content.clone(),
            reasoning_content: None,
        });
    }
    messages.push(ChatMessage {
        role: "user".into(),
        content: question.to_string(),
        reasoning_content: None,
    });

    let start = std::time::Instant::now();
    let result = client
        .chat_oneshot(&messages, &system, llm_cfg.max_tokens.max(2048), llm_cfg.temperature)
        .await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(answer) => {
            crate::ai_log::log_ai_request(
                &state.db, "report_qa", &client.model,
                duration_ms, "success", None,
                Some(&format!("report_id={}, q={}", id, question)),
                Some(&system),
                Some(&answer),
            ).await;
            Ok(Json(serde_json::json!({ "answer": answer })))
        }
        Err(e) => {
            crate::ai_log::log_ai_request(
                &state.db, "report_qa", &client.model,
                duration_ms, "failed", Some(&e),
                Some(&format!("report_id={}, q={}", id, question)),
                Some(&system),
                None,
            ).await;
            Err((StatusCode::BAD_GATEWAY, format!("AI Q&A failed: {}", e)))
        }
    }
}

#[cfg(test)]
mod runtime_filter_tests {
    use super::{apply_runtime_filter_values, runtime_value_for_op};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn scalar_coercion() {
        assert_eq!(runtime_value_for_op("=", "华东"), json!("华东"));
        assert_eq!(runtime_value_for_op(">=", "100"), json!(100));
        assert_eq!(runtime_value_for_op("=", "3.5"), json!(3.5));
        // Blank stays a blank string (treated as inactive downstream).
        assert_eq!(runtime_value_for_op("=", "  "), json!(""));
    }

    #[test]
    fn in_splits_and_drops_blanks() {
        assert_eq!(runtime_value_for_op("IN", "A, B ,C"), json!(["A", "B", "C"]));
        assert_eq!(runtime_value_for_op("IN", "1,2,3"), json!([1, 2, 3]));
    }

    #[test]
    fn between_requires_two_parts() {
        assert_eq!(
            runtime_value_for_op("BETWEEN", "2024-01-01,2024-12-31"),
            json!(["2024-01-01", "2024-12-31"])
        );
        // Incomplete range => blank (inactive).
        assert_eq!(runtime_value_for_op("BETWEEN", "2024-01-01"), json!(""));
    }

    #[test]
    fn override_replaces_only_matching_keys() {
        let stored = json!([
            {"key": "region", "op": "=", "value": "华北", "targets": [{"datasource_id": 1, "column": "region"}]},
            {"key": "d", "op": "BETWEEN", "value": ["2024-01-01", "2024-12-31"], "targets": [{"datasource_id": 1, "column": "created_at"}]}
        ]);
        let mut params = HashMap::new();
        params.insert("f_region".to_string(), "华东".to_string());
        // 'd' has no param => keeps its stored value.

        let out = apply_runtime_filter_values(&Some(stored), &params).unwrap();
        let arr = out.as_array().unwrap();
        assert_eq!(arr[0]["value"], json!("华东"));
        assert_eq!(arr[1]["value"], json!(["2024-01-01", "2024-12-31"]));
    }

    #[test]
    fn override_with_no_stored_filters_is_none() {
        let params = HashMap::new();
        assert!(apply_runtime_filter_values(&None, &params).is_none());
    }

    #[test]
    fn empty_param_clears_control() {
        let stored = json!([
            {"key": "region", "op": "=", "value": "华北", "targets": [{"datasource_id": 1, "column": "region"}]}
        ]);
        let mut params = HashMap::new();
        params.insert("f_region".to_string(), "".to_string());
        let out = apply_runtime_filter_values(&Some(stored), &params).unwrap();
        assert_eq!(out.as_array().unwrap()[0]["value"], json!(""));
    }
}
