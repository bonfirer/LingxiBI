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
) -> Result<Json<Vec<MetricGroup>>, (StatusCode, String)> {
    let groups = sqlx::query_as::<_, MetricGroup>(
        "SELECT * FROM metric_groups WHERE (owner_user_id = ? OR ? = 1) \
         ORDER BY sort_order ASC, created_at ASC",
    )
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(groups))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateMetricGroup>,
) -> Result<(StatusCode, Json<MetricGroup>), (StatusCode, String)> {
    let result = sqlx::query(
        "INSERT INTO metric_groups (name, description, owner_user_id) VALUES (?, ?, ?)",
    )
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(user.id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let group = sqlx::query_as::<_, MetricGroup>("SELECT * FROM metric_groups WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok((StatusCode::CREATED, Json(group)))
}

/// Load a metric group and verify the caller owns it (admins bypass).
async fn load_owned_group(
    state: &AppState,
    id: i32,
    user: &AuthUser,
) -> Result<MetricGroup, (StatusCode, String)> {
    let group = sqlx::query_as::<_, MetricGroup>("SELECT * FROM metric_groups WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Group not found".to_string()))?;
    ensure_owner(user, group.owner_user_id)?;
    Ok(group)
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateMetricGroup>,
) -> Result<Json<MetricGroup>, (StatusCode, String)> {
    let existing = load_owned_group(&state, id, &user).await?;

    sqlx::query("UPDATE metric_groups SET name=?, description=?, sort_order=? WHERE id=?")
        .bind(payload.name.as_deref().unwrap_or(&existing.name))
        .bind(payload.description.as_deref().or(existing.description.as_deref()))
        .bind(payload.sort_order.unwrap_or(existing.sort_order))
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let group = sqlx::query_as::<_, MetricGroup>("SELECT * FROM metric_groups WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(Json(group))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    load_owned_group(&state, id, &user).await?;

    // Move metrics in this group to ungrouped
    sqlx::query("UPDATE metric_pools SET group_id = NULL WHERE group_id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    sqlx::query("DELETE FROM metric_groups WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}
