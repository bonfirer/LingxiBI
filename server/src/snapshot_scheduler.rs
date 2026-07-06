//! Background task that periodically checks for due snapshot schedules
//! and executes them, storing results in metric_snapshots.

use std::sync::Arc;
use chrono::Utc;
use tracing::{info, warn, error};

use crate::models::*;
use crate::routes::{query, snapshots};
use crate::AppState;

/// Spawn the background snapshot scheduler.
/// It wakes up every 5 seconds, checks for schedules where next_run_at <= now,
/// executes the metric SQL, stores the snapshot, and updates next_run_at.
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        info!("Snapshot scheduler started");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            if let Err(e) = tick(&state).await {
                error!("Snapshot scheduler tick error: {}", e);
            }
        }
    });
}

async fn tick(state: &Arc<AppState>) -> Result<(), String> {
    let now = Utc::now();

    // Find all enabled schedules that are due
    let due_schedules = sqlx::query_as::<_, MetricSnapshotSchedule>(
        "SELECT * FROM metric_snapshot_schedules WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?",
    )
    .bind(now)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    if due_schedules.is_empty() {
        return Ok(());
    }

    info!("Snapshot scheduler: {} schedules due", due_schedules.len());

    for schedule in due_schedules {
        // Atomically claim before running so a concurrent tick / second instance
        // doesn't take the same snapshot twice.
        let next_run = compute_next_from_schedule(&schedule);
        let claimed = sqlx::query(
            "UPDATE metric_snapshot_schedules SET next_run_at = ?, updated_at = CURRENT_TIMESTAMP
             WHERE id = ? AND enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?",
        )
        .bind(next_run)
        .bind(schedule.id)
        .bind(now)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        if claimed.rows_affected() == 0 {
            continue;
        }

        if let Err(e) = execute_snapshot(state, &schedule).await {
            warn!(
                "Failed to take snapshot for metric_pool_id={}: {}",
                schedule.metric_pool_id, e
            );
            // Don't stop; continue with other schedules
        }
    }

    // Cleanup old snapshots based on retention_days
    cleanup_expired(state).await?;

    Ok(())
}

async fn execute_snapshot(state: &Arc<AppState>, schedule: &MetricSnapshotSchedule) -> Result<(), String> {
    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(schedule.metric_pool_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Metric pool {} not found", schedule.metric_pool_id))?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(metric.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Datasource {} not found", metric.datasource_id))?;

    // Execute the metric SQL (validated, timed out, row-capped), resolving any
    // {{param}} placeholders with the metric's default values.
    let defaults = query::param_defaults(&metric.params);
    let qr = query::execute_metric_sql(state, &ds, &metric.sql_query, &defaults, &[]).await?;

    let result_data = serde_json::to_value(&qr.rows).map_err(|e| e.to_string())?;

    let now = Utc::now();
    let (period_type, period_key) = snapshots::compute_period_info(&schedule.schedule_type, &now);

    // Insert the snapshot record
    sqlx::query(
        "INSERT INTO metric_snapshots (metric_pool_id, schedule_id, snapshot_at, period_type, period_key, result_data, row_count) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(schedule.metric_pool_id)
    .bind(schedule.id)
    .bind(now)
    .bind(&period_type)
    .bind(&period_key)
    .bind(&result_data)
    .bind(qr.row_count as i32)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Also update the metric_pool's result_cache with latest data
    sqlx::query("UPDATE metric_pools SET result_cache=?, row_count=?, updated_at=CURRENT_TIMESTAMP WHERE id=?")
        .bind(&result_data)
        .bind(qr.row_count as i32)
        .bind(schedule.metric_pool_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // Compute and set next_run_at
    let next_run = compute_next_from_schedule(schedule);
    sqlx::query(
        "UPDATE metric_snapshot_schedules SET last_run_at=?, next_run_at=?, updated_at=CURRENT_TIMESTAMP WHERE id=?",
    )
    .bind(now)
    .bind(next_run)
    .bind(schedule.id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    info!(
        "Snapshot taken: metric_pool_id={}, period={}:{}",
        schedule.metric_pool_id, period_type, period_key
    );

    Ok(())
}

fn compute_next_from_schedule(schedule: &MetricSnapshotSchedule) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = Utc::now();
    match schedule.schedule_type.as_str() {
        "hourly" => Some(now + chrono::Duration::hours(1)),
        "daily" => {
            let tomorrow = (now + chrono::Duration::days(1)).date_naive();
            Some(tomorrow.and_hms_opt(0, 0, 0).unwrap().and_utc())
        }
        "weekly" => {
            Some(now + chrono::Duration::weeks(1))
        }
        "monthly" => {
            Some(now + chrono::Duration::days(30))
        }
        "cron" => {
            // Parse cron_expr to determine interval
            // Supports: "*/N s" or "*/N * * * *" style or simple "Ns" shorthand
            if let Some(ref expr) = schedule.cron_expr {
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
/// Supports formats:
///   - "*/10 * * * * *" (6-field with seconds) → 10 seconds
///   - "*/5 * * * *"   (5-field, minute-level) → 5 minutes (300s)
///   - "10s" or "30s"   (shorthand seconds) → N seconds
///   - "5m"             (shorthand minutes) → N minutes
fn parse_cron_interval_secs(expr: &str) -> Option<i64> {
    let trimmed = expr.trim();

    // Shorthand: "10s", "30s", "60s" etc.
    if trimmed.ends_with('s') {
        if let Ok(n) = trimmed[..trimmed.len() - 1].trim().parse::<i64>() {
            if n > 0 {
                return Some(n);
            }
        }
    }

    // Shorthand: "5m", "10m" etc.
    if trimmed.ends_with('m') {
        if let Ok(n) = trimmed[..trimmed.len() - 1].trim().parse::<i64>() {
            if n > 0 {
                return Some(n * 60);
            }
        }
    }

    // Standard cron fields
    let parts: Vec<&str> = trimmed.split_whitespace().collect();

    if parts.len() == 6 {
        // 6-field cron: sec min hour day month weekday
        // e.g., "*/10 * * * * *" → every 10 seconds
        if let Some(secs) = parse_step_field(parts[0]) {
            return Some(secs);
        }
    }

    if parts.len() == 5 {
        // 5-field cron: min hour day month weekday
        // e.g., "*/5 * * * *" → every 5 minutes
        if let Some(mins) = parse_step_field(parts[0]) {
            return Some(mins * 60);
        }
    }

    None
}

/// Parse "*/N" to extract N, returns None for non-step patterns.
fn parse_step_field(field: &str) -> Option<i64> {
    if field.starts_with("*/") {
        if let Ok(n) = field[2..].parse::<i64>() {
            if n > 0 {
                return Some(n);
            }
        }
    }
    None
}

/// Remove snapshots that exceed their schedule's retention_days.
async fn cleanup_expired(state: &Arc<AppState>) -> Result<(), String> {
    let schedules_with_retention = sqlx::query_as::<_, MetricSnapshotSchedule>(
        "SELECT * FROM metric_snapshot_schedules WHERE retention_days IS NOT NULL AND retention_days > 0",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    for schedule in schedules_with_retention {
        if let Some(days) = schedule.retention_days {
            let cutoff = Utc::now() - chrono::Duration::days(days as i64);
            let deleted = sqlx::query(
                "DELETE FROM metric_snapshots WHERE schedule_id = ? AND snapshot_at < ?",
            )
            .bind(schedule.id)
            .bind(cutoff)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;

            if deleted.rows_affected() > 0 {
                info!(
                    "Cleaned up {} expired snapshots for schedule_id={}",
                    deleted.rows_affected(),
                    schedule.id
                );
            }
        }
    }

    Ok(())
}
