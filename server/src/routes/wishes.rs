use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::{ensure_admin, ensure_owner, internal_error};
use crate::routes::auth::AuthUser;
use crate::AppState;

/// List wishes. Admins see everything; members see only their own.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<Wish>>, (StatusCode, String)> {
    let wishes = if user.is_admin {
        sqlx::query_as::<_, Wish>(
            "SELECT * FROM wishes ORDER BY created_at DESC"
        )
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<_, Wish>(
            "SELECT * FROM wishes WHERE user_id = ? ORDER BY created_at DESC"
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await
    }
    .map_err(internal_error)?;

    Ok(Json(wishes))
}

/// Create a new wish.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateWish>,
) -> Result<(StatusCode, Json<Wish>), (StatusCode, String)> {
    let title = payload.title.trim();
    let content = payload.content.trim();

    if title.is_empty() || content.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Title and content are required".to_string()));
    }

    if title.len() > 255 {
        return Err((StatusCode::BAD_REQUEST, "Title must be 255 characters or less".to_string()));
    }

    let category = payload.category.as_deref().unwrap_or("feature").trim();
    if category.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Category cannot be empty".to_string()));
    }

    let result = sqlx::query(
        "INSERT INTO wishes (user_id, title, content, category) VALUES (?, ?, ?, ?)"
    )
    .bind(user.id)
    .bind(title)
    .bind(content)
    .bind(category)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let wish = sqlx::query_as::<_, Wish>("SELECT * FROM wishes WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok((StatusCode::CREATED, Json(wish)))
}

/// Update a wish. Members may edit their own pending wishes; admins may also
/// update the status (e.g. accept/reject/complete).
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateWish>,
) -> Result<Json<Wish>, (StatusCode, String)> {
    let existing: Option<Wish> = sqlx::query_as("SELECT * FROM wishes WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?;

    let existing = existing.ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
    ensure_owner(&user, existing.user_id.into())?;

    // Non-admins cannot change status and can only update pending wishes.
    if !user.is_admin {
        if payload.status.is_some() {
            return Err((StatusCode::FORBIDDEN, "Only admins can change status".to_string()));
        }
        if existing.status != "pending" {
            return Err((StatusCode::FORBIDDEN, "Only pending wishes can be edited".to_string()));
        }
    }

    let title = payload.title.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
    let content = payload.content.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
    let category = payload.category.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());

    if payload.title.is_some() && title.is_none() {
        return Err((StatusCode::BAD_REQUEST, "Title cannot be empty".to_string()));
    }
    if payload.content.is_some() && content.is_none() {
        return Err((StatusCode::BAD_REQUEST, "Content cannot be empty".to_string()));
    }
    if let Some(t) = title {
        if t.len() > 255 {
            return Err((StatusCode::BAD_REQUEST, "Title must be 255 characters or less".to_string()));
        }
    }

    let status = payload.status.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());

    sqlx::query(
        "UPDATE wishes SET
            title = COALESCE(?, title),
            content = COALESCE(?, content),
            category = COALESCE(?, category),
            status = COALESCE(?, status)
        WHERE id = ?"
    )
    .bind(title)
    .bind(content)
    .bind(category)
    .bind(status)
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let wish = sqlx::query_as::<_, Wish>("SELECT * FROM wishes WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(Json(wish))
}

/// Delete a wish. Members may delete their own; admins may delete any.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    let existing: Option<Wish> = sqlx::query_as("SELECT * FROM wishes WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?;

    if let Some(existing) = existing {
        ensure_owner(&user, existing.user_id.into())?;
        sqlx::query("DELETE FROM wishes WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(internal_error)?;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Admin-only: list all wishes with a simple status summary.
pub async fn summary(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;

    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, COUNT(*) FROM wishes GROUP BY status"
    )
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM wishes")
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    let mut counts = serde_json::Map::new();
    for (status, count) in rows {
        counts.insert(status, serde_json::json!(count));
    }

    Ok(Json(serde_json::json!({
        "total": total.0,
        "by_status": counts,
    })))
}
