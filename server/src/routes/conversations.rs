use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::{ensure_owner, internal_error};
use crate::AppState;

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<Conversation>>, (StatusCode, String)> {
    let convs = sqlx::query_as::<_, Conversation>(
        "SELECT * FROM conversations WHERE (owner_user_id = ? OR ? = 1) ORDER BY updated_at DESC",
    )
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(convs))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<(StatusCode, Json<Conversation>), (StatusCode, String)> {
    let result = sqlx::query("INSERT INTO conversations (title, owner_user_id) VALUES (?, ?)")
        .bind("New Conversation")
        .bind(user.id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let conv = sqlx::query_as::<_, Conversation>("SELECT * FROM conversations WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok((StatusCode::CREATED, Json(conv)))
}

/// Verify the caller owns conversation `id` (admins bypass). Returns 404 otherwise.
async fn ensure_conversation_access(
    state: &AppState,
    id: i32,
    user: &AuthUser,
) -> Result<(), (StatusCode, String)> {
    let owner: Option<(Option<i32>,)> =
        sqlx::query_as("SELECT owner_user_id FROM conversations WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(internal_error)?;
    match owner {
        Some((owner_user_id,)) => ensure_owner(user, owner_user_id),
        None => Err((StatusCode::NOT_FOUND, "Conversation not found".to_string())),
    }
}

pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<Message>>, (StatusCode, String)> {
    ensure_conversation_access(&state, id, &user).await?;

    let messages = sqlx::query_as::<_, Message>(
        "SELECT * FROM messages WHERE conversation_id = ? ORDER BY created_at ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(messages))
}

/// Lightweight status endpoint so the client can poll an in-progress
/// async generation after navigating away and back.
pub async fn get_status(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let row = sqlx::query_as::<_, (Option<String>, Option<String>, Option<i32>)>(
        "SELECT generation_status, generation_error, owner_user_id FROM conversations WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Conversation not found".to_string()))?;

    ensure_owner(&user, row.2)?;

    Ok(Json(serde_json::json!({
        "generation_status": row.0.unwrap_or_else(|| "idle".into()),
        "generation_error": row.1,
    })))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_conversation_access(&state, id, &user).await?;

    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}
