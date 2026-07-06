use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::query;
use crate::routes::{ensure_owner, internal_error};
use crate::AppState;

/// Verify the caller owns metric `metric_id` (admins bypass). 404 otherwise.
async fn ensure_metric_owned(
    state: &AppState,
    metric_id: i32,
    user: &AuthUser,
) -> Result<(), (StatusCode, String)> {
    let m: Option<(Option<i32>,)> =
        sqlx::query_as("SELECT owner_user_id FROM metric_pools WHERE id = ?")
            .bind(metric_id)
            .fetch_optional(&state.db)
            .await
            .map_err(internal_error)?;
    match m {
        Some((owner,)) => ensure_owner(user, owner),
        None => Err((StatusCode::NOT_FOUND, "Metric pool not found".to_string())),
    }
}

/// Load a schedule and verify the caller owns it (admins bypass).
async fn load_owned_schedule(
    state: &AppState,
    id: i32,
    user: &AuthUser,
) -> Result<MetricSnapshotSchedule, (StatusCode, String)> {
    let schedule = sqlx::query_as::<_, MetricSnapshotSchedule>(
        "SELECT * FROM metric_snapshot_schedules WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Schedule not found".to_string()))?;
    ensure_owner(user, schedule.owner_user_id)?;
    Ok(schedule)
}

// ── Schedule CRUD ──

/// List the caller's snapshot schedules.
pub async fn list_schedules(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<MetricSnapshotSchedule>>, (StatusCode, String)> {
    let schedules = sqlx::query_as::<_, MetricSnapshotSchedule>(
        "SELECT * FROM metric_snapshot_schedules WHERE (owner_user_id = ? OR ? = 1) ORDER BY created_at DESC",
    )
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(schedules))
}

/// Get schedule for a specific metric.
pub async fn get_schedule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(metric_id): Path<i32>,
) -> Result<Json<MetricSnapshotSchedule>, (StatusCode, String)> {
    ensure_metric_owned(&state, metric_id, &user).await?;
    let schedule = sqlx::query_as::<_, MetricSnapshotSchedule>(
        "SELECT * FROM metric_snapshot_schedules WHERE metric_pool_id = ?",
    )
    .bind(metric_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Schedule not found".to_string()))?;

    Ok(Json(schedule))
}

/// Create a snapshot schedule for a metric.
pub async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateSnapshotSchedule>,
) -> Result<(StatusCode, Json<MetricSnapshotSchedule>), (StatusCode, String)> {
    // Validate metric exists and is owned by the caller.
    ensure_metric_owned(&state, payload.metric_pool_id, &user).await?;

    // Validate schedule_type
    let valid_types = ["hourly", "daily", "weekly", "monthly", "cron"];
    if !valid_types.contains(&payload.schedule_type.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid schedule_type. Must be one of: {:?}", valid_types),
        ));
    }

    if payload.schedule_type == "cron" && payload.cron_expr.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "cron_expr is required when schedule_type is 'cron'".to_string(),
        ));
    }

    let next_run = compute_next_run(&payload.schedule_type, payload.cron_expr.as_deref());

    // Use ON DUPLICATE KEY UPDATE to handle the case where a schedule already exists
    // (e.g., a disabled one from a previous manual snapshot)
    sqlx::query(
        "INSERT INTO metric_snapshot_schedules (metric_pool_id, schedule_type, cron_expr, retention_days, enabled, next_run_at, owner_user_id)
         VALUES (?, ?, ?, ?, 1, ?, ?)
         ON DUPLICATE KEY UPDATE schedule_type=VALUES(schedule_type), cron_expr=VALUES(cron_expr),
           retention_days=VALUES(retention_days), enabled=1, next_run_at=VALUES(next_run_at), updated_at=CURRENT_TIMESTAMP",
    )
    .bind(payload.metric_pool_id)
    .bind(&payload.schedule_type)
    .bind(&payload.cron_expr)
    .bind(payload.retention_days)
    .bind(&next_run)
    .bind(user.id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let schedule = sqlx::query_as::<_, MetricSnapshotSchedule>(
        "SELECT * FROM metric_snapshot_schedules WHERE metric_pool_id = ?",
    )
    .bind(payload.metric_pool_id)
    .fetch_one(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(schedule)))
}

/// Update a snapshot schedule.
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateSnapshotSchedule>,
) -> Result<Json<MetricSnapshotSchedule>, (StatusCode, String)> {
    let existing = load_owned_schedule(&state, id, &user).await?;

    let schedule_type = payload.schedule_type.as_deref().unwrap_or(&existing.schedule_type);
    let cron_expr = payload.cron_expr.as_deref().or(existing.cron_expr.as_deref());
    let enabled = payload.enabled.unwrap_or(existing.enabled);
    let retention_days = payload.retention_days.or(existing.retention_days);

    let next_run = if enabled {
        compute_next_run(schedule_type, cron_expr)
    } else {
        None
    };

    sqlx::query(
        "UPDATE metric_snapshot_schedules SET schedule_type=?, cron_expr=?, enabled=?, retention_days=?, next_run_at=?, updated_at=CURRENT_TIMESTAMP WHERE id=?",
    )
    .bind(schedule_type)
    .bind(cron_expr)
    .bind(enabled)
    .bind(retention_days)
    .bind(next_run)
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let schedule = sqlx::query_as::<_, MetricSnapshotSchedule>(
        "SELECT * FROM metric_snapshot_schedules WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(schedule))
}

/// Delete a snapshot schedule (and all its snapshots via CASCADE).
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    load_owned_schedule(&state, id, &user).await?;
    sqlx::query("DELETE FROM metric_snapshot_schedules WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Snapshot Data ──

#[derive(Debug, serde::Deserialize)]
pub struct SnapshotListQuery {
    pub period_type: Option<String>,
    pub limit: Option<i32>,
}

/// List snapshots for a metric.
pub async fn list_snapshots(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(metric_id): Path<i32>,
    Query(params): Query<SnapshotListQuery>,
) -> Result<Json<Vec<MetricSnapshot>>, (StatusCode, String)> {
    ensure_metric_owned(&state, metric_id, &user).await?;
    let limit = params.limit.unwrap_or(50).min(200);

    let snapshots = if let Some(ref pt) = params.period_type {
        sqlx::query_as::<_, MetricSnapshot>(
            "SELECT * FROM metric_snapshots WHERE metric_pool_id = ? AND period_type = ? ORDER BY snapshot_at DESC LIMIT ?",
        )
        .bind(metric_id)
        .bind(pt)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<_, MetricSnapshot>(
            "SELECT * FROM metric_snapshots WHERE metric_pool_id = ? ORDER BY snapshot_at DESC LIMIT ?",
        )
        .bind(metric_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    }
    .map_err(crate::routes::internal_error)?;

    Ok(Json(snapshots))
}

#[derive(Debug, serde::Deserialize)]
pub struct CompareQuery {
    pub period_type: String,
    pub current_key: String,
    pub previous_key: String,
}

/// Compare two snapshots (YoY, MoM, or arbitrary periods).
pub async fn compare_snapshots(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(metric_id): Path<i32>,
    Query(params): Query<CompareQuery>,
) -> Result<Json<SnapshotComparison>, (StatusCode, String)> {
    ensure_metric_owned(&state, metric_id, &user).await?;
    let current = sqlx::query_as::<_, MetricSnapshot>(
        "SELECT * FROM metric_snapshots WHERE metric_pool_id = ? AND period_type = ? AND period_key = ? ORDER BY snapshot_at DESC LIMIT 1",
    )
    .bind(metric_id)
    .bind(&params.period_type)
    .bind(&params.current_key)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let previous = sqlx::query_as::<_, MetricSnapshot>(
        "SELECT * FROM metric_snapshots WHERE metric_pool_id = ? AND period_type = ? AND period_key = ? ORDER BY snapshot_at DESC LIMIT 1",
    )
    .bind(metric_id)
    .bind(&params.period_type)
    .bind(&params.previous_key)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(SnapshotComparison {
        current,
        previous,
        period_type: params.period_type,
        current_key: params.current_key,
        previous_key: params.previous_key,
    }))
}

/// Manually trigger a snapshot for a metric (immediate capture).
pub async fn take_snapshot(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(metric_id): Path<i32>,
) -> Result<(StatusCode, Json<MetricSnapshot>), (StatusCode, String)> {
    ensure_metric_owned(&state, metric_id, &user).await?;
    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(metric_id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Metric not found".to_string()))?;

    // Access to the underlying datasource may have been revoked since creation.
    crate::routes::datasources::ensure_access(&state, metric.datasource_id, &user).await?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(metric.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // Validate + execute with shared safety guards (timeout + row cap),
    // resolving any {{param}} placeholders with the metric's default values.
    let defaults = query::param_defaults(&metric.params);
    let qr = query::execute_metric_sql(&state, &ds, &metric.sql_query, &defaults, &[])
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let result_data = serde_json::to_value(&qr.rows)
        .map_err(crate::routes::internal_error)?;

    let now = chrono::Utc::now();
    let period_type = "daily";
    let period_key = now.format("%Y-%m-%d").to_string();

    // Find or use a default schedule_id
    let schedule_id = sqlx::query_scalar::<_, i32>(
        "SELECT id FROM metric_snapshot_schedules WHERE metric_pool_id = ? LIMIT 1",
    )
    .bind(metric_id)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?
    .unwrap_or(0);

    let insert_result = sqlx::query(
        "INSERT INTO metric_snapshots (metric_pool_id, schedule_id, snapshot_at, period_type, period_key, result_data, row_count) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(metric_id)
    .bind(schedule_id)
    .bind(now)
    .bind(period_type)
    .bind(&period_key)
    .bind(&result_data)
    .bind(qr.row_count as i32)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let snapshot = sqlx::query_as::<_, MetricSnapshot>(
        "SELECT * FROM metric_snapshots WHERE id = ?",
    )
    .bind(insert_result.last_insert_id() as i32)
    .fetch_one(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(snapshot)))
}

/// Delete a specific snapshot.
pub async fn delete_snapshot(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((metric_id, snapshot_id)): Path<(i32, i32)>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_metric_owned(&state, metric_id, &user).await?;
    let result = sqlx::query("DELETE FROM metric_snapshots WHERE id = ? AND metric_pool_id = ?")
        .bind(snapshot_id)
        .bind(metric_id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Snapshot not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Helper functions ──

/// Compute the next run time based on schedule type.
fn compute_next_run(schedule_type: &str, cron_expr: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = chrono::Utc::now();
    match schedule_type {
        "hourly" => Some(now + chrono::Duration::hours(1)),
        "daily" => {
            // Next day at 00:00 UTC
            let tomorrow = (now + chrono::Duration::days(1)).date_naive();
            Some(tomorrow.and_hms_opt(0, 0, 0).unwrap().and_utc())
        }
        "weekly" => {
            Some(now + chrono::Duration::weeks(1))
        }
        "monthly" => {
            // Approximate: 30 days
            Some(now + chrono::Duration::days(30))
        }
        "cron" => {
            if let Some(expr) = cron_expr {
                if let Some(secs) = parse_cron_interval_secs(expr) {
                    return Some(now + chrono::Duration::seconds(secs));
                }
            }
            // Fallback: 1 minute
            Some(now + chrono::Duration::minutes(1))
        }
        _ => None,
    }
}

/// Parse a cron expression to extract an interval in seconds.
/// Supports: "*/10 * * * * *" (6-field, seconds), "*/5 * * * *" (5-field, minutes),
/// "10s" (shorthand seconds), "5m" (shorthand minutes)
fn parse_cron_interval_secs(expr: &str) -> Option<i64> {
    let trimmed = expr.trim();

    // Shorthand: "10s", "30s"
    if trimmed.ends_with('s') {
        if let Ok(n) = trimmed[..trimmed.len() - 1].trim().parse::<i64>() {
            if n > 0 { return Some(n); }
        }
    }

    // Shorthand: "5m"
    if trimmed.ends_with('m') {
        if let Ok(n) = trimmed[..trimmed.len() - 1].trim().parse::<i64>() {
            if n > 0 { return Some(n * 60); }
        }
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();

    // 6-field: sec min hour day month weekday
    if parts.len() == 6 {
        if let Some(secs) = parse_step_field(parts[0]) {
            return Some(secs);
        }
    }

    // 5-field: min hour day month weekday
    if parts.len() == 5 {
        if let Some(mins) = parse_step_field(parts[0]) {
            return Some(mins * 60);
        }
    }

    None
}

fn parse_step_field(field: &str) -> Option<i64> {
    if field.starts_with("*/") {
        if let Ok(n) = field[2..].parse::<i64>() {
            if n > 0 { return Some(n); }
        }
    }
    None
}

/// Determine period_type and period_key from schedule type and timestamp.
pub fn compute_period_info(schedule_type: &str, at: &chrono::DateTime<chrono::Utc>) -> (String, String) {
    match schedule_type {
        "hourly" => (
            "hourly".to_string(),
            at.format("%Y-%m-%d-%H").to_string(),
        ),
        "daily" => (
            "daily".to_string(),
            at.format("%Y-%m-%d").to_string(),
        ),
        "weekly" => (
            "weekly".to_string(),
            at.format("%G-W%V").to_string(),
        ),
        "monthly" => (
            "monthly".to_string(),
            at.format("%Y-%m").to_string(),
        ),
        _ => (
            "daily".to_string(),
            at.format("%Y-%m-%d").to_string(),
        ),
    }
}
