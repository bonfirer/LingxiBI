use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::query;
use crate::routes::{ensure_owner, internal_error};
use crate::AppState;

/// Decode the stored `filters` JSON into typed conditions. A missing/invalid
/// value yields no filters, so the metric SQL runs unchanged.
fn parse_filters(ds: &ReportDataSource) -> Vec<FilterCondition> {
    ds.filters
        .as_ref()
        .and_then(|v| serde_json::from_value::<Vec<FilterCondition>>(v.clone()).ok())
        .unwrap_or_default()
}

/// Default parameter values for a report dataset's linked metric (if any),
/// used to resolve `{{param}}` placeholders on refresh/cache paths.
async fn metric_defaults(
    state: &AppState,
    metric_id: Option<i32>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let Some(mid) = metric_id else {
        return std::collections::HashMap::new();
    };
    let row: Option<(Option<serde_json::Value>,)> =
        sqlx::query_as("SELECT params FROM metric_pools WHERE id = ?")
            .bind(mid)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    query::param_defaults(&row.and_then(|(p,)| p))
}

/// Load a report's global filter controls (JSON), if any.
async fn load_report_filters(
    state: &AppState,
    report_id: i32,
) -> Result<Option<serde_json::Value>, (StatusCode, String)> {
    let row: Option<(Option<serde_json::Value>,)> =
        sqlx::query_as("SELECT report_filters FROM reports WHERE id = ?")
            .bind(report_id)
            .fetch_optional(&state.db)
            .await
            .map_err(internal_error)?;
    Ok(row.and_then(|(v,)| v))
}

/// Verify the caller owns the parent report (admins bypass). 404 otherwise.
async fn ensure_report_owned(
    state: &AppState,
    report_id: i32,
    user: &AuthUser,
) -> Result<(), (StatusCode, String)> {
    let r: Option<(Option<i32>,)> =
        sqlx::query_as("SELECT owner_user_id FROM reports WHERE id = ?")
            .bind(report_id)
            .fetch_optional(&state.db)
            .await
            .map_err(internal_error)?;
    match r {
        Some((owner,)) => ensure_owner(user, owner),
        None => Err((StatusCode::NOT_FOUND, "Report not found".to_string())),
    }
}

/// List all datasources for a report.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(report_id): Path<i32>,
) -> Result<Json<Vec<ReportDataSource>>, (StatusCode, String)> {
    ensure_report_owned(&state, report_id, &user).await?;
    let items = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE report_id = ? ORDER BY created_at ASC",
    )
    .bind(report_id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(items))
}

/// Add a datasource to a report (from metric or custom SQL).
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(report_id): Path<i32>,
    Json(payload): Json<CreateReportDataSource>,
) -> Result<(StatusCode, Json<ReportDataSource>), (StatusCode, String)> {
    ensure_report_owned(&state, report_id, &user).await?;
    // Require access to the datasource this report dataset runs against.
    crate::routes::datasources::ensure_access(&state, payload.datasource_id, &user).await?;
    // If linking from a metric, copy its result_cache
    let (result_cache, row_count): (Option<serde_json::Value>, Option<i32>) =
        if let Some(mid) = payload.metric_id {
            let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
                .bind(mid)
                .fetch_optional(&state.db)
                .await
                .map_err(crate::routes::internal_error)?;
            match metric {
                Some(m) => (m.result_cache, m.row_count),
                None => (None, None),
            }
        } else {
            (None, None)
        };

    let result = sqlx::query(
        "INSERT INTO report_datasources (report_id, metric_id, name, sql_query, datasource_id, result_cache, row_count) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(report_id)
    .bind(payload.metric_id)
    .bind(&payload.name)
    .bind(&payload.sql_query)
    .bind(payload.datasource_id)
    .bind(&result_cache)
    .bind(row_count)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let item = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE id = ?",
    )
    .bind(result.last_insert_id() as i32)
    .fetch_one(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(item)))
}

/// Remove a datasource from a report.
pub async fn remove(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((report_id, ds_id)): Path<(i32, i32)>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_report_owned(&state, report_id, &user).await?;
    let result = sqlx::query("DELETE FROM report_datasources WHERE id = ? AND report_id = ?")
        .bind(ds_id)
        .bind(report_id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Report datasource not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Refresh a report datasource (re-execute SQL).
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((report_id, ds_id)): Path<(i32, i32)>,
) -> Result<Json<ReportDataSource>, (StatusCode, String)> {
    ensure_report_owned(&state, report_id, &user).await?;
    let item = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE id = ? AND report_id = ?",
    )
    .bind(ds_id)
    .bind(report_id)
    .fetch_optional(&state.db)
    .await
    .map_err(crate::routes::internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;

    // Access to the underlying datasource may have been revoked since creation.
    crate::routes::datasources::ensure_access(&state, item.datasource_id, &user).await?;

    query::validate_sql(&item.sql_query)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(item.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // Re-run with the dataset's stored filters combined with the report's
    // global filter controls, so the cache matches the current view. Resolve
    // any {{param}} placeholders with the linked metric's defaults.
    let report_filters = load_report_filters(&state, report_id).await?;
    let filters = query::combined_filters(parse_filters(&item), &report_filters, item.datasource_id);
    let param_values = metric_defaults(&state, item.metric_id).await;
    let qr = query::execute_metric_sql(&state, &ds, &item.sql_query, &param_values, &filters)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let cache = serde_json::to_value(&qr.rows).ok();

    sqlx::query("UPDATE report_datasources SET result_cache=?, row_count=? WHERE id=?")
        .bind(&cache)
        .bind(qr.row_count as i32)
        .bind(ds_id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let updated = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE id = ?",
    )
    .bind(ds_id)
    .fetch_one(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(updated))
}

/// Set (or clear) the runtime filters on a report dataset, then re-execute so
/// the cached result reflects the new conditions. An empty `filters` array
/// clears them and reverts to the plain metric SQL.
pub async fn set_filters(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((report_id, ds_id)): Path<(i32, i32)>,
    Json(payload): Json<SetReportDataSourceFilters>,
) -> Result<Json<ReportDataSource>, (StatusCode, String)> {
    ensure_report_owned(&state, report_id, &user).await?;
    let item = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE id = ? AND report_id = ?",
    )
    .bind(ds_id)
    .bind(report_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;

    crate::routes::datasources::ensure_access(&state, item.datasource_id, &user).await?;

    query::validate_sql(&item.sql_query)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(item.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // Validate the filter shape (columns/operators) before persisting so a bad
    // filter is rejected with 400 rather than silently stored.
    query::build_filtered_sql(&item.sql_query, &payload.filters, &ds.db_type)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    // Cache with dataset filters combined with the report's global controls.
    let report_filters = load_report_filters(&state, report_id).await?;
    let effective = query::combined_filters(payload.filters.clone(), &report_filters, item.datasource_id);
    let param_values = metric_defaults(&state, item.metric_id).await;
    let qr = query::execute_metric_sql(&state, &ds, &item.sql_query, &param_values, &effective)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    // Empty filters => store NULL so the dataset reverts to plain metric SQL.
    let filters_json: Option<serde_json::Value> = if payload.filters.is_empty() {
        None
    } else {
        serde_json::to_value(&payload.filters).ok()
    };
    let cache = serde_json::to_value(&qr.rows).ok();

    sqlx::query("UPDATE report_datasources SET filters=?, result_cache=?, row_count=? WHERE id=?")
        .bind(&filters_json)
        .bind(&cache)
        .bind(qr.row_count as i32)
        .bind(ds_id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    let updated = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE id = ?",
    )
    .bind(ds_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(updated))
}


#[derive(serde::Deserialize)]
pub struct AiFilterRequest {
    pub instruction: String,
}

/// Suggest filter conditions for a dataset from a natural-language request.
///
/// The AI is given only the dataset's output columns (with a sample value each)
/// and the allowed operators, and returns structured conditions. Every
/// suggestion is validated against the same builder used at execution time, and
/// any that reference unknown columns or fail validation are dropped. Nothing is
/// applied — the caller reviews the suggestions and applies them via
/// `set_filters`.
pub async fn ai_filters(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path((report_id, ds_id)): Path<(i32, i32)>,
    Json(req): Json<AiFilterRequest>,
) -> Result<Json<Vec<FilterCondition>>, (StatusCode, String)> {
    ensure_report_owned(&state, report_id, &user).await?;
    let item = sqlx::query_as::<_, ReportDataSource>(
        "SELECT * FROM report_datasources WHERE id = ? AND report_id = ?",
    )
    .bind(ds_id)
    .bind(report_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;

    crate::routes::datasources::ensure_access(&state, item.datasource_id, &user).await?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(item.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // Available columns + a sample value each, from the cached result.
    let rows: Vec<serde_json::Value> = item
        .result_cache
        .as_ref()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let first = rows.first().and_then(|r| r.as_object());
    let columns: Vec<String> = first
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    if columns.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "No columns available. Refresh the dataset first.".to_string(),
        ));
    }
    let known: std::collections::HashSet<&str> = columns.iter().map(|s| s.as_str()).collect();

    let mut columns_context = String::new();
    if let Some(obj) = first {
        for (k, v) in obj {
            let sample = match v {
                serde_json::Value::Null => "null".to_string(),
                serde_json::Value::String(s) => format!("\"{}\"", s),
                other => other.to_string(),
            };
            columns_context.push_str(&format!("- {} (e.g. {})\n", k, sample));
        }
    }

    // Load LLM config.
    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "LLM not configured".to_string()))?;
    if llm_cfg.api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "API key not configured".to_string()));
    }

    let client = crate::llm::LlmClient::new(llm_cfg.base_url, llm_cfg.api_key, llm_cfg.model);
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let system = crate::llm::prompts::filter_suggest_prompt(&columns_context, &today, "zh");
    let messages = vec![crate::llm::ChatMessage {
        role: "user".into(),
        content: req.instruction.clone(),
        reasoning_content: None,
    }];

    let raw = client
        .chat_oneshot(&messages, &system, 2048, 0.0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM failed: {}", e)))?;

    // Parse the JSON array from the response (tolerate markdown fences / prose).
    let json_slice = extract_json_array(&raw)
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "AI did not return a filter list".to_string()))?;
    let candidates: Vec<FilterCondition> =
        serde_json::from_str(json_slice).map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not parse AI filters: {}", e))
        })?;

    // Keep only conditions on known columns that pass the real filter builder.
    let valid: Vec<FilterCondition> = candidates
        .into_iter()
        .filter(|c| known.contains(c.column.as_str()))
        .filter(|c| {
            query::build_filtered_sql(&item.sql_query, std::slice::from_ref(c), &ds.db_type)
                .map(|o| o.is_some())
                .unwrap_or(false)
        })
        .collect();

    Ok(Json(valid))
}

/// Extract the first top-level JSON array substring from an LLM response,
/// ignoring surrounding prose or ```json fences.
fn extract_json_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if end > start {
        Some(&text[start..=end])
    } else {
        None
    }
}
