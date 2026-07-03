use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::ensure_admin;
use crate::AppState;

/// List all examples, optionally by datasource.
pub async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AiExample>>, (StatusCode, String)> {
    let examples = sqlx::query_as::<_, AiExample>(
        "SELECT * FROM ai_examples ORDER BY created_at DESC"
    )
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(examples))
}

/// List examples for a specific datasource.
pub async fn list_by_datasource(
    State(state): State<Arc<AppState>>,
    Path(ds_id): Path<i32>,
) -> Result<Json<Vec<AiExample>>, (StatusCode, String)> {
    let examples = sqlx::query_as::<_, AiExample>(
        "SELECT * FROM ai_examples WHERE datasource_id = ? ORDER BY created_at DESC"
    )
    .bind(ds_id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(examples))
}

/// Create a new example (thumbs-up from conversation).
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateAiExample>,
) -> Result<(StatusCode, Json<AiExample>), (StatusCode, String)> {
    ensure_admin(&user)?;
    let category = payload.category.as_deref().unwrap_or("sql");

    let result = sqlx::query(
        "INSERT INTO ai_examples (datasource_id, question, answer, category) VALUES (?, ?, ?, ?)"
    )
    .bind(payload.datasource_id)
    .bind(&payload.question)
    .bind(&payload.answer)
    .bind(category)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let entry = sqlx::query_as::<_, AiExample>("SELECT * FROM ai_examples WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(entry)))
}

/// Delete an example.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_admin(&user)?;
    sqlx::query("DELETE FROM ai_examples WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}
