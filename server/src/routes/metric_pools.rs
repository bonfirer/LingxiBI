use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::{ensure_owner, internal_error};
use crate::routes::query;
use crate::AppState;

/// Load a metric and verify the caller owns it (admins bypass).
async fn load_owned_metric(
    state: &AppState,
    id: i32,
    user: &AuthUser,
) -> Result<MetricPool, (StatusCode, String)> {
    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Metric not found".to_string()))?;
    ensure_owner(user, metric.owner_user_id)?;
    Ok(metric)
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<MetricPool>>, (StatusCode, String)> {
    // Deliberately exclude the heavy `result_cache` column: it can hold up to
    // MAX_ROWS (50k) rows of JSON per metric, and the sidebar + metrics page
    // both poll this list. Callers that need the cached rows fetch the single
    // metric via `get_one`, which returns the full row. `row_count` is kept so
    // the UI can still show counts cheaply. Scoped to the caller's own metrics.
    let metrics = sqlx::query_as::<_, MetricPool>(
        "SELECT id, name, description, sql_query, datasource_id, group_id, \
         NULL AS result_cache, row_count, source_pool_id, owner_user_id, created_at, updated_at \
         FROM metric_pools WHERE (owner_user_id = ? OR ? = 1) \
         ORDER BY group_id ASC, created_at DESC",
    )
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(metrics))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<MetricPool>, (StatusCode, String)> {
    let metric = load_owned_metric(&state, id, &user).await?;
    Ok(Json(metric))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateMetricPool>,
) -> Result<(StatusCode, Json<MetricPool>), (StatusCode, String)> {
    // The metric runs SQL against this datasource — require access to it.
    crate::routes::datasources::ensure_access(&state, payload.datasource_id, &user).await?;

    // Optionally copy result_cache from source data pool
    let (result_cache, row_count): (Option<serde_json::Value>, Option<i32>) =
        if let Some(source_id) = payload.source_pool_id {
            let pool = sqlx::query_as::<_, DataPool>("SELECT * FROM data_pools WHERE id = ?")
                .bind(source_id)
                .fetch_optional(&state.db)
                .await
                .map_err(internal_error)?;
            match pool {
                Some(p) => (p.result_cache, p.row_count),
                None => (None, None),
            }
        } else {
            (None, None)
        };

    let result = sqlx::query(
        "INSERT INTO metric_pools (name, description, sql_query, datasource_id, group_id, result_cache, row_count, source_pool_id, owner_user_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(&payload.sql_query)
    .bind(payload.datasource_id)
    .bind(payload.group_id)
    .bind(&result_cache)
    .bind(row_count)
    .bind(payload.source_pool_id)
    .bind(user.id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok((StatusCode::CREATED, Json(metric)))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateMetricPool>,
) -> Result<Json<MetricPool>, (StatusCode, String)> {
    let existing = load_owned_metric(&state, id, &user).await?;

    sqlx::query("UPDATE metric_pools SET name=?, description=?, sql_query=?, group_id=?, updated_at=CURRENT_TIMESTAMP WHERE id=?")
        .bind(payload.name.as_deref().unwrap_or(&existing.name))
        .bind(payload.description.as_deref().or(existing.description.as_deref()))
        .bind(payload.sql_query.as_deref().unwrap_or(&existing.sql_query))
        .bind(payload.group_id.or(existing.group_id))
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(Json(metric))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    load_owned_metric(&state, id, &user).await?;

    sqlx::query("DELETE FROM metric_pools WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Re-execute the metric's SQL and refresh its cached data.
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<MetricPool>, (StatusCode, String)> {
    let metric = load_owned_metric(&state, id, &user).await?;

    // Access to the underlying datasource may have been revoked since creation.
    crate::routes::datasources::ensure_access(&state, metric.datasource_id, &user).await?;

    // Validate SQL
    query::validate_sql(&metric.sql_query)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(metric.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    let qr = query::execute_validated(&state, &ds, &metric.sql_query)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let cache = serde_json::to_value(&qr.rows).ok();

    sqlx::query("UPDATE metric_pools SET result_cache=?, row_count=?, updated_at=CURRENT_TIMESTAMP WHERE id=?")
        .bind(&cache)
        .bind(qr.row_count as i32)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let updated = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(Json(updated))
}

/// Move a metric to a different group.
pub async fn move_metric(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<MoveToGroup>,
) -> Result<Json<MetricPool>, (StatusCode, String)> {
    load_owned_metric(&state, id, &user).await?;

    // If a target group is given, verify the caller owns it too.
    if let Some(group_id) = payload.group_id {
        let g: Option<(Option<i32>,)> =
            sqlx::query_as("SELECT owner_user_id FROM metric_groups WHERE id = ?")
                .bind(group_id)
                .fetch_optional(&state.db)
                .await
                .map_err(internal_error)?;
        match g {
            Some((owner,)) => ensure_owner(&user, owner)?,
            None => return Err((StatusCode::NOT_FOUND, "Group not found".to_string())),
        }
    }

    sqlx::query("UPDATE metric_pools SET group_id = ? WHERE id = ?")
        .bind(payload.group_id)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(Json(metric))
}
