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
) -> Result<Json<Vec<ReportGroup>>, (StatusCode, String)> {
    let groups = sqlx::query_as::<_, ReportGroup>(
        "SELECT * FROM report_groups WHERE (owner_user_id = ? OR ? = 1) \
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
    Json(payload): Json<CreateReportGroup>,
) -> Result<(StatusCode, Json<ReportGroup>), (StatusCode, String)> {
    let result = sqlx::query(
        "INSERT INTO report_groups (name, description, owner_user_id) VALUES (?, ?, ?)",
    )
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(user.id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let group = sqlx::query_as::<_, ReportGroup>("SELECT * FROM report_groups WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok((StatusCode::CREATED, Json(group)))
}

/// Load a report group and verify the caller owns it (admins bypass).
async fn load_owned_group(
    state: &AppState,
    id: i32,
    user: &AuthUser,
) -> Result<ReportGroup, (StatusCode, String)> {
    let group = sqlx::query_as::<_, ReportGroup>("SELECT * FROM report_groups WHERE id = ?")
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
    Json(payload): Json<UpdateReportGroup>,
) -> Result<Json<ReportGroup>, (StatusCode, String)> {
    let existing = load_owned_group(&state, id, &user).await?;

    sqlx::query("UPDATE report_groups SET name=?, description=?, sort_order=? WHERE id=?")
        .bind(payload.name.as_deref().unwrap_or(&existing.name))
        .bind(payload.description.as_deref().or(existing.description.as_deref()))
        .bind(payload.sort_order.unwrap_or(existing.sort_order))
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let group = sqlx::query_as::<_, ReportGroup>("SELECT * FROM report_groups WHERE id = ?")
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

    // Move reports in this group to ungrouped
    sqlx::query("UPDATE reports SET group_id = NULL WHERE group_id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    sqlx::query("DELETE FROM report_groups WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Move a report to a group (or ungrouped with group_id=null).
pub async fn move_report(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(report_id): Path<i32>,
    Json(payload): Json<MoveToGroup>,
) -> Result<Json<Report>, (StatusCode, String)> {
    // Verify the caller owns the report being moved.
    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(report_id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Report not found".to_string()))?;
    ensure_owner(&user, report.owner_user_id)?;

    // If a target group is given, verify the caller owns that group too.
    if let Some(group_id) = payload.group_id {
        load_owned_group(&state, group_id, &user).await?;
    }

    sqlx::query("UPDATE reports SET group_id = ? WHERE id = ?")
        .bind(payload.group_id)
        .bind(report_id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let report = sqlx::query_as::<_, Report>("SELECT * FROM reports WHERE id = ?")
        .bind(report_id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(Json(report))
}
