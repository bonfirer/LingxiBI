use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;
use crate::routes::auth::AuthUser;
use crate::routes::ensure_admin;
use crate::AppState;

#[derive(sqlx::FromRow, serde::Serialize)]
pub struct AiLogEntry {
    pub id: i32,
    pub request_type: String,
    pub model: Option<String>,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub duration_ms: Option<i32>,
    pub status: Option<String>,
    pub error_message: Option<String>,
    pub context: Option<String>,
    pub input_params: Option<String>,
    pub output_result: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(serde::Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(serde::Serialize)]
pub struct PaginatedLogs {
    pub data: Vec<AiLogEntry>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
    pub total_pages: u32,
}

/// GET /api/ai-logs?page=1&page_size=20 — list AI request logs with pagination.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<PaginatedLogs>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(20).min(100).max(1);
    let offset = (page - 1) * page_size;

    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_logs")
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let logs = sqlx::query_as::<_, AiLogEntry>(
        "SELECT id, request_type, model, prompt_tokens, completion_tokens, duration_ms, status, error_message, context, \
         LEFT(input_params, 500) AS input_params, LEFT(output_result, 500) AS output_result, created_at \
         FROM ai_logs ORDER BY created_at DESC LIMIT ? OFFSET ?"
    )
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let total_pages = ((total as f64) / (page_size as f64)).ceil() as u32;

    Ok(Json(PaginatedLogs {
        data: logs,
        total,
        page,
        page_size,
        total_pages,
    }))
}

/// GET /api/ai-logs/:id — get full detail of a single log entry.
pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<AiLogEntry>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let log = sqlx::query_as::<_, AiLogEntry>(
        "SELECT * FROM ai_logs WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Log not found".to_string()))?;

    Ok(Json(log))
}
