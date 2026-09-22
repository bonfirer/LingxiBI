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

#[derive(serde::Deserialize)]
pub struct AiParameterizeRequest {
    pub sql: String,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AiParameterizeResponse {
    pub sql: String,
    pub params: serde_json::Value,
}

/// Rewrite a metric's SQL to add `{{param}}` / `[[ ]]` placeholders for the
/// conditions a user would want to filter on, and return the parameter
/// definitions. Nothing is saved — the caller reviews and saves. The returned
/// SQL is validated as read-only.
pub async fn ai_parameterize(
    State(state): State<Arc<AppState>>,
    Extension(_user): Extension<AuthUser>,
    Json(req): Json<AiParameterizeRequest>,
) -> Result<Json<AiParameterizeResponse>, (StatusCode, String)> {
    if req.sql.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "SQL is required".to_string()));
    }
    // Only ever parameterize read-only queries.
    query::validate_sql(&req.sql).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "LLM not configured".to_string()))?;
    if llm_cfg.api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "API key not configured".to_string()));
    }

    let client = crate::llm::LlmClient::new(llm_cfg.base_url, llm_cfg.api_key, llm_cfg.model);
    let system = crate::llm::prompts::metric_parameterize_prompt(req.lang.as_deref().unwrap_or("zh"));
    let user_msg = match &req.instruction {
        Some(i) if !i.trim().is_empty() => format!("SQL:\n{}\n\nRequest: {}", req.sql, i.trim()),
        _ => format!("SQL:\n{}", req.sql),
    };
    let messages = vec![crate::llm::ChatMessage {
        role: "user".into(),
        content: user_msg,
        reasoning_content: None,
    }];

    let raw = client
        .chat_oneshot(&messages, &system, 4096, 0.0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM failed: {}", e)))?;

    // Parse the JSON object from the response (tolerate fences / prose).
    let start = raw.find('{');
    let end = raw.rfind('}');
    let (start, end) = match (start, end) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => return Err((StatusCode::INTERNAL_SERVER_ERROR, "AI did not return a JSON object".to_string())),
    };
    let parsed: serde_json::Value = serde_json::from_str(&raw[start..=end])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not parse AI response: {}", e)))?;

    let sql = parsed
        .get("sql")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.sql)
        .to_string();
    // The rewritten SQL must still be read-only. Placeholders don't affect the
    // allowlist tokenizer (braces are punctuation), so validate it directly.
    query::validate_sql(&sql).map_err(|e| (StatusCode::BAD_REQUEST, format!("AI produced invalid SQL: {}", e)))?;

    let params = parsed.get("params").cloned().unwrap_or(serde_json::Value::Array(vec![]));

    Ok(Json(AiParameterizeResponse { sql, params }))
}

#[derive(serde::Deserialize)]
pub struct AiGenerateRequest {
    pub datasource_id: i32,
    pub description: String,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AiGenerateResponse {
    pub name: String,
    pub sql: String,
    pub params: serde_json::Value,
}

/// Generate a full parameterized metric (name + SQL with `{{param}}` / `[[ ]]`
/// placeholders + param defs) from a natural-language description and the
/// datasource schema. Nothing is saved — the caller reviews and saves. The
/// returned SQL is validated as read-only.
pub async fn ai_generate(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<AiGenerateRequest>,
) -> Result<Json<AiGenerateResponse>, (StatusCode, String)> {
    if req.description.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Description is required".to_string()));
    }
    // The metric will run against this datasource — require access to it.
    crate::routes::datasources::ensure_access(&state, req.datasource_id, &user).await?;

    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "LLM not configured".to_string()))?;
    if llm_cfg.api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "API key not configured".to_string()));
    }

    // Give the model this datasource's schema so it uses real table/columns.
    let schema_context =
        crate::routes::chat::build_kg_context(&state, Some(req.datasource_id), &req.description, &user)
            .await;

    let client = crate::llm::LlmClient::new(llm_cfg.base_url, llm_cfg.api_key, llm_cfg.model);
    let system = crate::llm::prompts::metric_generate_prompt(
        &schema_context,
        req.lang.as_deref().unwrap_or("zh"),
    );
    let messages = vec![crate::llm::ChatMessage {
        role: "user".into(),
        content: req.description.clone(),
        reasoning_content: None,
    }];

    let raw = client
        .chat_oneshot(&messages, &system, 4096, 0.0)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM failed: {}", e)))?;

    let start = raw.find('{');
    let end = raw.rfind('}');
    let (start, end) = match (start, end) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => return Err((StatusCode::INTERNAL_SERVER_ERROR, "AI did not return a JSON object".to_string())),
    };
    let parsed: serde_json::Value = serde_json::from_str(&raw[start..=end])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not parse AI response: {}", e)))?;

    let sql = parsed
        .get("sql")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if sql.trim().is_empty() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "AI returned no SQL".to_string()));
    }
    query::validate_sql(&sql).map_err(|e| (StatusCode::BAD_REQUEST, format!("AI produced invalid SQL: {}", e)))?;

    let name = parsed
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("AI Metric")
        .to_string();
    let params = parsed.get("params").cloned().unwrap_or(serde_json::Value::Array(vec![]));

    Ok(Json(AiGenerateResponse { name, sql, params }))
}

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
         NULL AS result_cache, row_count, source_pool_id, params, owner_user_id, created_at, updated_at \
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

    // Idempotent favorite: if this user already has a metric with the same
    // datasource + SQL, return it instead of inserting a duplicate. Data pools
    // get a fresh id on every query, so dedup keys on the SQL (the actual
    // metric identity), not source_pool_id.
    if let Some(existing) = sqlx::query_as::<_, MetricPool>(
        "SELECT * FROM metric_pools \
         WHERE datasource_id = ? AND sql_query = ? AND (owner_user_id = ? OR owner_user_id IS NULL) \
         ORDER BY id ASC LIMIT 1",
    )
    .bind(payload.datasource_id)
    .bind(&payload.sql_query)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    {
        return Ok((StatusCode::OK, Json(existing)));
    }

    // Optionally copy result_cache (and any params) from the source data pool.
    // A pool created from a parameterized AI query carries its param defs, so a
    // metric saved from it keeps the placeholders working.
    let (result_cache, row_count, pool_params): (Option<serde_json::Value>, Option<i32>, Option<serde_json::Value>) =
        if let Some(source_id) = payload.source_pool_id {
            // `data_pools` rows are created by whoever ran the query, and the
            // table has no owner column — so gate on the pool's datasource and
            // require it to match the metric being created. Otherwise a member
            // could enumerate pool ids and read arbitrary users' cached result
            // sets from datasources they were never granted.
            let pool = sqlx::query_as::<_, DataPool>("SELECT * FROM data_pools WHERE id = ?")
                .bind(source_id)
                .fetch_optional(&state.db)
                .await
                .map_err(internal_error)?
                .ok_or((StatusCode::NOT_FOUND, "Source query not found".to_string()))?;
            crate::routes::datasources::ensure_access(&state, pool.datasource_id, &user).await?;
            if pool.datasource_id != payload.datasource_id {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Source query belongs to a different data source".to_string(),
                ));
            }
            (pool.result_cache, pool.row_count, pool.params)
        } else {
            (None, None, None)
        };
    // Explicit params in the request take precedence over the pool's.
    let params = payload.params.clone().or(pool_params);

    let result = sqlx::query(
        "INSERT INTO metric_pools (name, description, sql_query, datasource_id, group_id, result_cache, row_count, source_pool_id, params, owner_user_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(&payload.sql_query)
    .bind(payload.datasource_id)
    .bind(payload.group_id)
    .bind(&result_cache)
    .bind(row_count)
    .bind(payload.source_pool_id)
    .bind(&params)
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

/// Column names a metric currently exposes, read from its cached rows. Every
/// column is present on every row (NULLs serialize as `Value::Null`), so the
/// first row's keys are the complete set. Empty when there is no cache yet.
fn cached_columns(cache: &Option<serde_json::Value>) -> Vec<String> {
    cache
        .as_ref()
        .and_then(|v| v.as_array())
        .and_then(|rows| rows.first())
        .and_then(|row| row.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

/// Run a metric's edited SQL and enforce the stable-columns contract.
///
/// A metric's result columns are part of its public shape: generated report HTML
/// hardcodes column names in its chart configs (only the data behind them gets
/// swapped by `refreshData()`), and dataset/report filters match on them too. So
/// the SQL behind a metric may change freely — different tables, conditions,
/// aggregation — as long as every column it used to expose is still there.
/// Adding columns is allowed; existing charts simply ignore them.
///
/// Returns the fresh result so the caller can reuse it as the new cache instead
/// of running the query twice. Errors are 400: the caller must reject the edit
/// without persisting anything.
async fn validate_sql_change(
    state: &AppState,
    user: &AuthUser,
    existing: &MetricPool,
    new_sql: &str,
    params: &Option<serde_json::Value>,
) -> Result<QueryResult, (StatusCode, String)> {
    // Access to the underlying datasource may have been revoked since creation.
    crate::routes::datasources::ensure_access(state, existing.datasource_id, user).await?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(existing.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // `execute_metric_sql` validates read-only-ness and resolves `{{param}}`
    // placeholders with the metric's new defaults. A broken edit fails here as a
    // 400 rather than silently degrading the report's live-data path later.
    let qr = query::execute_metric_sql(state, &ds, new_sql, &query::param_defaults(params), &[])
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    // Baseline = what the metric exposed before. Cached rows are the cheap
    // source; with no cache (never refreshed) probe the previous SQL once. If
    // that fails too there is no contract to enforce, so the edit goes through.
    let mut baseline = cached_columns(&existing.result_cache);
    if baseline.is_empty() {
        let prev_defaults = query::param_defaults(&existing.params);
        if let Ok(prev) =
            query::execute_metric_sql(state, &ds, &existing.sql_query, &prev_defaults, &[]).await
        {
            baseline = prev.columns;
        }
    }

    let missing: Vec<&str> = baseline
        .iter()
        .filter(|c| !qr.columns.contains(c))
        .map(|c| c.as_str())
        .collect();
    if !missing.is_empty() {
        // Sentinel prefix — the frontend maps it to a localized message listing
        // the columns that would disappear.
        return Err((
            StatusCode::BAD_REQUEST,
            format!("metric-columns-changed:{}", missing.join(", ")),
        ));
    }

    Ok(qr)
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateMetricPool>,
) -> Result<Json<MetricPool>, (StatusCode, String)> {
    let existing = load_owned_metric(&state, id, &user).await?;

    // params: use the provided value if present, else keep the existing one.
    let params = payload.params.clone().or_else(|| existing.params.clone());
    let new_sql = payload.sql_query.as_deref().unwrap_or(&existing.sql_query);
    // The metric library is the source of truth for its SQL, but report datasets
    // keep a copy taken at link time. Track whether the statement actually
    // changed so a plain rename/move stays a single cheap UPDATE.
    let sql_changed = new_sql != existing.sql_query;

    // Check the edit before touching the row, so a rejected change leaves the
    // metric exactly as it was.
    let refreshed = if sql_changed {
        Some(validate_sql_change(&state, &user, &existing, new_sql, &params).await?)
    } else {
        None
    };

    sqlx::query("UPDATE metric_pools SET name=?, description=?, sql_query=?, group_id=?, params=?, updated_at=CURRENT_TIMESTAMP WHERE id=?")
        .bind(payload.name.as_deref().unwrap_or(&existing.name))
        .bind(payload.description.as_deref().or(existing.description.as_deref()))
        .bind(new_sql)
        .bind(payload.group_id.or(existing.group_id))
        .bind(&params)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;

    if let Some(qr) = refreshed {
        // Reuse the validation run as the new cache — the old rows came from the
        // previous statement, so they must not survive the edit.
        let cache = serde_json::to_value(&qr.rows).ok();
        sqlx::query("UPDATE metric_pools SET result_cache=?, row_count=? WHERE id=?")
            .bind(&cache)
            .bind(qr.row_count as i32)
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(internal_error)?;

        // Fan the new SQL out to every report dataset linked to this metric.
        // Runs after the UPDATE above so the new params are already visible.
        crate::routes::report_datasources::propagate_metric_sql(&state, id, new_sql).await?;
    }

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

    // Refuse deletion while the metric is still linked to any report datasource.
    // Those report datasets reference the metric via `report_datasources.metric_id`;
    // deleting the metric would orphan them. The frontend maps the `metric-in-use`
    // sentinel to a localized message.
    let (in_use,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM report_datasources WHERE metric_id = ?")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .map_err(internal_error)?;
    if in_use > 0 {
        return Err((StatusCode::CONFLICT, "metric-in-use".to_string()));
    }

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

    // Resolve any {{param}} placeholders using the metric's default values.
    let defaults = query::param_defaults(&metric.params);
    let qr = query::execute_metric_sql(&state, &ds, &metric.sql_query, &defaults, &[])
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
