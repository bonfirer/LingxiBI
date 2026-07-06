use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use sqlx::Column;
use sqlx::Row;
use std::sync::Arc;
use tokio::time::{timeout, Duration};

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::AppState;

/// Maximum rows to return from any query.
const MAX_ROWS: usize = 50_000;

/// Maximum execution time per query.
const QUERY_TIMEOUT_SECS: u64 = 30;

/// SQL tokenizer that respects string literals, comments, and quoted identifiers.
/// Returns a list of upper-cased keyword tokens for safety analysis.
fn tokenize_sql(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let c = chars[i];

        // Skip whitespace
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // Single-line comment: -- ...
        if c == '-' && i + 1 < len && chars[i + 1] == '-' {
            i += 2;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Block comment: /* ... */
        if c == '/' && i + 1 < len && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2; // skip */
            continue;
        }

        // Single-quoted string: '...'
        if c == '\'' {
            i += 1;
            while i < len {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2; // skip escaped char
                } else if chars[i] == '\'' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Double-quoted identifier: "..."
        if c == '"' {
            i += 1;
            while i < len {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                } else if chars[i] == '"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Backtick-quoted identifier (MySQL): `...`
        if c == '`' {
            i += 1;
            while i < len && chars[i] != '`' {
                i += 1;
            }
            i += 1; // skip closing backtick
            continue;
        }

        // Identifier or keyword: [a-zA-Z_][a-zA-Z0-9_]*
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            tokens.push(word.to_uppercase());
            continue;
        }

        // Number literal: skip
        if c.is_ascii_digit() {
            i += 1;
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
            continue;
        }

        // Other punctuation/symbols: skip
        i += 1;
    }

    tokens
}

/// SQL safety validator — tokenization-based allowlist.
/// Allows: SELECT, SHOW, DESCRIBE, EXPLAIN, WITH (CTEs).
/// Denies: all mutation statements and dangerous functions.
pub fn validate_sql(sql: &str) -> Result<(), String> {
    let tokens = tokenize_sql(sql);

    // Check for multiple statements (semicolons outside of string literals/comments)
    let semicolon_count = sql
        .chars()
        .filter(|&c| c == ';')
        .count();
    if semicolon_count > 0 {
        // Allow a single trailing semicolon
        let trimmed = sql.trim_end();
        if !(semicolon_count == 1 && trimmed.ends_with(';')) {
            return Err("Multiple statements are not allowed".to_string());
        }
    }

    let forbidden: &[&str] = &[
        "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "TRUNCATE", "CREATE",
        "REPLACE", "GRANT", "REVOKE", "LOAD_FILE", "LOAD", "CALL", "EXEC",
        "EXECUTE", "MERGE", "RENAME", "SET", "LOCK", "UNLOCK", "FLUSH",
        "KILL", "PURGE", "RESET", "OPTIMIZE", "HANDLER", "IMPORT",
    ];

    for token in &tokens {
        if forbidden.contains(&token.as_str()) {
            return Err(format!("Operation not allowed: {}", token));
        }
    }

    // Check for INTO OUTFILE / INTO DUMPFILE / INTO @variable
    for window in tokens.windows(2) {
        if window[0] == "INTO" && (window[1] == "OUTFILE" || window[1] == "DUMPFILE") {
            return Err("Operation not allowed: INTO OUTFILE/DUMPFILE".to_string());
        }
    }
    // Block SELECT ... INTO (variable assignment or file export)
    if tokens.contains(&"INTO".to_string()) {
        // Allow INTO only if it's part of a subquery context — but for safety, block it entirely
        // in user-facing queries. INTO in SELECT is used for variable assignment or file writes.
        return Err("INTO clause is not allowed in queries".to_string());
    }

    // Allowlist: first meaningful token must be a read-only statement
    if let Some(first) = tokens.first() {
        let allowed = ["SELECT", "SHOW", "DESCRIBE", "EXPLAIN", "WITH", "DESC"];
        if !allowed.contains(&first.as_str()) {
            return Err(format!(
                "Only SELECT, SHOW, DESCRIBE, EXPLAIN, and WITH (CTE) queries are allowed. Got: {}",
                first
            ));
        }
    } else {
        return Err("Empty query".to_string());
    }

    Ok(())
}

pub async fn execute(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResult>, (StatusCode, String)> {
    // Enforce datasource-level access before running any SQL.
    crate::routes::datasources::ensure_access(&state, req.datasource_id, &user).await?;

    // Get datasource connection info
    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(req.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // Resolve any {{param}} / [[ ]] placeholders: start from the param defaults,
    // then apply explicit value overrides. Non-parameterized SQL takes the fast
    // path inside `execute_metric_sql` (identical to plain execution).
    let mut param_values = param_defaults(&req.params);
    if let Some(serde_json::Value::Object(map)) = &req.param_values {
        for (k, v) in map {
            param_values.insert(k.clone(), v.clone());
        }
    }

    // Validate + execute with shared safety guards (timeout + row cap).
    let query_result = execute_metric_sql(&state, &ds, &req.sql, &param_values, &[])
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let truncated_rows = query_result.rows;
    let row_count = truncated_rows.len();

    // Save as data pool
    let result_cache = serde_json::to_value(&truncated_rows).ok();
    let pool_name = format!("query_{}", chrono::Utc::now().timestamp());

    let pool_result = sqlx::query(
        "INSERT INTO data_pools (name, sql_query, datasource_id, result_cache, row_count) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&pool_name)
    .bind(&req.sql)
    .bind(req.datasource_id)
    .bind(&result_cache)
    .bind(row_count as i32)
    .execute(&state.db)
    .await;

    let _pool_id = pool_result.map(|r| r.last_insert_id() as i32).ok();

    Ok(Json(QueryResult {
        columns: query_result.columns,
        rows: truncated_rows,
        row_count,
    }))
}

/// Validate a SQL string, dispatch to the right per-DB executor, and enforce the
/// query timeout. The per-DB executors already cap results at `MAX_ROWS`.
///
/// Use this from any path that runs user/metric SQL (HTTP, snapshot scheduler,
/// alert engine) so the safety guards are applied consistently.
pub async fn execute_validated(
    state: &AppState,
    ds: &DataSource,
    sql: &str,
) -> Result<QueryResult, String> {
    validate_sql(sql)?;

    let fut = async {
        match ds.db_type.as_str() {
            "mysql" => execute_mysql(state, ds, sql).await,
            "postgresql" => execute_postgres(state, ds, sql).await,
            "oracle" => execute_oracle(state, ds, sql).await,
            other => Err(format!("Unsupported database type: {}", other)),
        }
    };

    timeout(Duration::from_secs(QUERY_TIMEOUT_SECS), fut)
        .await
        .map_err(|_| format!("Query timed out after {} seconds", QUERY_TIMEOUT_SECS))?
}

/// Operators allowed in a runtime filter. The operator itself is never taken
/// from user input verbatim — it is matched against this set and a canonical
/// form is emitted, so it cannot carry injected SQL.
fn is_scalar_op(op: &str) -> Option<&'static str> {
    match op {
        "=" => Some("="),
        "!=" | "<>" => Some("<>"),
        ">" => Some(">"),
        ">=" => Some(">="),
        "<" => Some("<"),
        "<=" => Some("<="),
        "LIKE" => Some("LIKE"),
        _ => None,
    }
}

/// A filter column must be a plain SQL identifier. This is the only part of a
/// filter that is inlined into SQL text, so it is strictly validated; values
/// always go through bound parameters.
fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Placeholder syntax differs per database: MySQL/Oracle accept positional
/// markers, Postgres uses `$n`. Oracle uses `:n`, MySQL uses `?`.
fn placeholder(db_type: &str, idx: usize) -> String {
    match db_type {
        "postgresql" => format!("${}", idx),
        "oracle" => format!(":{}", idx),
        _ => "?".to_string(),
    }
}

/// Wrap already-validated read-only SQL as a subquery and append a WHERE clause
/// built from `filters`. Returns `Ok(None)` when there are no filters, so the
/// caller runs the base SQL unchanged (identical to pre-filter behavior).
///
/// Column names are validated as identifiers and operators are canonicalized;
/// every value is emitted as a placeholder and returned in `binds` for the
/// caller to bind — user input is never interpolated into SQL text.
pub fn build_filtered_sql(
    base_sql: &str,
    filters: &[FilterCondition],
    db_type: &str,
) -> Result<Option<(String, Vec<serde_json::Value>)>, String> {
    let mut idx = 1usize;
    build_filtered_sql_from(base_sql, filters, db_type, &mut idx)
}

/// Like `build_filtered_sql`, but placeholder numbering starts at `*idx` and
/// `*idx` is advanced past the binds produced here. Used to compose a filter
/// WHERE after parameter placeholders so numbered dialects (Postgres `$n`,
/// Oracle `:n`) stay sequential across the whole statement.
pub fn build_filtered_sql_from(
    base_sql: &str,
    filters: &[FilterCondition],
    db_type: &str,
    idx: &mut usize,
) -> Result<Option<(String, Vec<serde_json::Value>)>, String> {
    if filters.is_empty() {
        return Ok(None);
    }

    let base = base_sql.trim().trim_end_matches(';').trim();
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<serde_json::Value> = Vec::new();

    for f in filters {
        if !is_valid_identifier(&f.column) {
            return Err(format!("Invalid filter column: {}", f.column));
        }
        let op = f.op.trim().to_uppercase();

        if let Some(op_sql) = is_scalar_op(&op) {
            if f.value.is_null() {
                return Err(format!("Filter on '{}' requires a value", f.column));
            }
            let ph = placeholder(db_type, *idx);
            *idx += 1;
            clauses.push(format!("_m.{} {} {}", f.column, op_sql, ph));
            binds.push(f.value.clone());
        } else if op == "IN" {
            let arr = f
                .value
                .as_array()
                .ok_or_else(|| format!("IN filter on '{}' requires an array value", f.column))?;
            if arr.is_empty() {
                return Err(format!("IN filter on '{}' requires a non-empty array", f.column));
            }
            let mut phs = Vec::with_capacity(arr.len());
            for v in arr {
                phs.push(placeholder(db_type, *idx));
                *idx += 1;
                binds.push(v.clone());
            }
            clauses.push(format!("_m.{} IN ({})", f.column, phs.join(", ")));
        } else if op == "BETWEEN" {
            let arr = f
                .value
                .as_array()
                .ok_or_else(|| format!("BETWEEN filter on '{}' requires a [min, max] array", f.column))?;
            if arr.len() != 2 {
                return Err(format!("BETWEEN filter on '{}' requires exactly two values", f.column));
            }
            let ph1 = placeholder(db_type, *idx);
            let ph2 = placeholder(db_type, *idx + 1);
            *idx += 2;
            clauses.push(format!("_m.{} BETWEEN {} AND {}", f.column, ph1, ph2));
            binds.push(arr[0].clone());
            binds.push(arr[1].clone());
        } else {
            return Err(format!("Unsupported filter operator: {}", f.op));
        }
    }

    let sql = format!(
        "SELECT * FROM ({}) AS _m WHERE {}",
        base,
        clauses.join(" AND ")
    );
    Ok(Some((sql, binds)))
}

/// Extract default parameter values from a metric's `params` JSON. Used when a
/// metric runs without an interactive request (refresh, snapshot, alert) so
/// `{{param}}` placeholders still resolve. Blank defaults are skipped.
pub fn param_defaults(
    params: &Option<serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut out = std::collections::HashMap::new();
    if let Some(v) = params {
        if let Ok(defs) = serde_json::from_value::<Vec<MetricParam>>(v.clone()) {
            for d in defs {
                if let Some(def) = d.default {
                    if !is_blank_param_value(&def) {
                        out.insert(d.name, def);
                    }
                }
            }
        }
    }
    out
}

/// Whether a parameter value counts as "not provided": null, blank string,
/// empty array, or an array with any blank element.
fn is_blank_param_value(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(a) => a.is_empty() || a.iter().any(is_blank_param_value),
        _ => false,
    }
}

/// Render `{{name}}` placeholders and `[[ ... ]]` optional blocks in metric SQL
/// against `values`, producing SQL with bound placeholders and the ordered bind
/// list. Placeholder numbering starts at `*idx` and advances it.
///
/// Semantics (Metabase-style, backward compatible):
/// - `[[ ... {{p}} ... ]]`: kept only if every `{{p}}` inside has a non-blank
///   value; otherwise the whole block is dropped. This is how a metric stays
///   "unfiltered" when no value is supplied.
/// - Bare `{{p}}` (outside a block): replaced by a bound value; errors if `p`
///   has no value (such params should have a default or be wrapped in `[[ ]]`).
/// - SQL with no `{{` / `[[` is returned unchanged with no binds.
pub fn render_parameterized_sql(
    sql: &str,
    values: &std::collections::HashMap<String, serde_json::Value>,
    db_type: &str,
    idx: &mut usize,
) -> Result<(String, Vec<serde_json::Value>), String> {
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut out = String::new();
    let mut binds: Vec<serde_json::Value> = Vec::new();
    let mut i = 0;

    while i < len {
        // Optional block: [[ ... ]]
        if chars[i] == '[' && i + 1 < len && chars[i + 1] == '[' {
            let close = find_seq(&chars, i + 2, ']', ']')
                .ok_or_else(|| "Unclosed '[[' block in metric SQL".to_string())?;
            let inner: String = chars[i + 2..close].iter().collect();
            match render_inline_placeholders(&inner, values, db_type, idx)? {
                Some((rendered, mut block_binds)) => {
                    out.push_str(&rendered);
                    binds.append(&mut block_binds);
                }
                None => { /* a referenced param is blank -> drop the whole block */ }
            }
            i = close + 2;
            continue;
        }

        // Bare placeholder: {{ name }}
        if chars[i] == '{' && i + 1 < len && chars[i + 1] == '{' {
            let close = find_seq(&chars, i + 2, '}', '}')
                .ok_or_else(|| "Unclosed '{{' placeholder in metric SQL".to_string())?;
            let name: String = chars[i + 2..close].iter().collect();
            let name = name.trim();
            match values.get(name) {
                Some(v) if !is_blank_param_value(v) => {
                    out.push_str(&placeholder(db_type, *idx));
                    *idx += 1;
                    binds.push(v.clone());
                }
                _ => {
                    return Err(format!(
                        "Missing value for required parameter '{}' (wrap it in [[ ]] to make it optional)",
                        name
                    ));
                }
            }
            i = close + 2;
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }

    Ok((out, binds))
}

/// Render the placeholders inside one `[[ ]]` block. Returns None if any
/// referenced parameter is blank (caller drops the block), otherwise the
/// rendered text and its binds. A block with no placeholders is always kept.
fn render_inline_placeholders(
    inner: &str,
    values: &std::collections::HashMap<String, serde_json::Value>,
    db_type: &str,
    idx: &mut usize,
) -> Result<Option<(String, Vec<serde_json::Value>)>, String> {
    let chars: Vec<char> = inner.chars().collect();
    let len = chars.len();
    let mut out = String::new();
    let mut binds: Vec<serde_json::Value> = Vec::new();
    let mut i = 0;

    while i < len {
        if chars[i] == '{' && i + 1 < len && chars[i + 1] == '{' {
            let close = find_seq(&chars, i + 2, '}', '}')
                .ok_or_else(|| "Unclosed '{{' placeholder in optional block".to_string())?;
            let name: String = chars[i + 2..close].iter().collect();
            let name = name.trim();
            match values.get(name) {
                Some(v) if !is_blank_param_value(v) => {
                    out.push_str(&placeholder(db_type, *idx));
                    *idx += 1;
                    binds.push(v.clone());
                }
                // A referenced param is missing/blank -> drop the whole block.
                _ => return Ok(None),
            }
            i = close + 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    Ok(Some((out, binds)))
}

/// Find the index of the first occurrence of the two-char sequence `a``b`
/// starting at `from`, returning the index of `a`.
fn find_seq(chars: &[char], from: usize, a: char, b: char) -> Option<usize> {
    let mut i = from;
    while i + 1 < chars.len() {
        if chars[i] == a && chars[i + 1] == b {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// True when a filter value should be treated as "not set" (control inactive):
/// null, blank string, empty array, or an array containing any blank element
/// (so a half-filled BETWEEN / IN never produces a broken condition).
fn is_blank_filter_value(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(a) => a.is_empty() || a.iter().any(is_blank_filter_value),
        _ => false,
    }
}

/// Translate a report's global filter controls into per-dataset conditions for
/// the given datasource. Controls with a blank value are skipped (inactive), so
/// an untouched global filter never changes a dataset's data.
pub fn report_filters_to_conditions(
    report_filters: &serde_json::Value,
    datasource_id: i32,
) -> Vec<FilterCondition> {
    let Ok(filters) = serde_json::from_value::<Vec<ReportFilter>>(report_filters.clone()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in filters {
        if is_blank_filter_value(&f.value) {
            continue;
        }
        for target in &f.targets {
            if target.datasource_id == datasource_id {
                out.push(FilterCondition {
                    column: target.column.clone(),
                    op: f.op.clone(),
                    value: f.value.clone(),
                });
            }
        }
    }
    out
}

/// Combine a dataset's own filters with any report-level controls that target
/// it. Report-level conditions are appended (AND-ed) after the dataset's own.
pub fn combined_filters(
    ds_filters: Vec<FilterCondition>,
    report_filters: &Option<serde_json::Value>,
    datasource_id: i32,
) -> Vec<FilterCondition> {
    let mut all = ds_filters;
    if let Some(rf) = report_filters {
        all.extend(report_filters_to_conditions(rf, datasource_id));
    }
    all
}

/// Validate + execute SQL, optionally applying runtime filters. When `filters`
/// is empty this is exactly `execute_validated` (base SQL, no bound params).
/// Otherwise the base SQL is wrapped as a subquery and filter values are passed
/// as bound parameters.
pub async fn execute_validated_with_filters(
    state: &AppState,
    ds: &DataSource,
    sql: &str,
    filters: &[FilterCondition],
) -> Result<QueryResult, String> {
    validate_sql(sql)?;

    let Some((wrapped, binds)) = build_filtered_sql(sql, filters, &ds.db_type)? else {
        // No filters — run the base query exactly as before.
        return execute_validated(state, ds, sql).await;
    };

    // The wrapper is derived from an already-validated read-only base; re-check
    // the assembled statement to keep the read-only guarantee.
    validate_sql(&wrapped)?;

    let fut = async {
        match ds.db_type.as_str() {
            "mysql" => execute_mysql_params(state, ds, &wrapped, &binds).await,
            "postgresql" => execute_postgres_params(state, ds, &wrapped, &binds).await,
            "oracle" => execute_oracle_params(state, ds, &wrapped, &binds).await,
            other => Err(format!("Unsupported database type: {}", other)),
        }
    };

    timeout(Duration::from_secs(QUERY_TIMEOUT_SECS), fut)
        .await
        .map_err(|_| format!("Query timed out after {} seconds", QUERY_TIMEOUT_SECS))?
}

/// Full metric execution pipeline: resolve `{{param}}` / `[[ ]]` placeholders in
/// the metric SQL from `param_values`, then optionally wrap the result with a
/// filter WHERE — all bound as parameters with one continuous placeholder
/// numbering so numbered dialects stay valid. Empty params + empty filters is
/// exactly `execute_validated` (unchanged base SQL).
pub async fn execute_metric_sql(
    state: &AppState,
    ds: &DataSource,
    sql: &str,
    param_values: &std::collections::HashMap<String, serde_json::Value>,
    filters: &[FilterCondition],
) -> Result<QueryResult, String> {
    validate_sql(sql)?;

    // Fast path: nothing to inject → identical to the pre-parameter behavior.
    let has_placeholders = sql.contains("{{") || sql.contains("[[");
    if !has_placeholders && filters.is_empty() {
        return execute_validated(state, ds, sql).await;
    }

    let mut idx = 1usize;
    let (rendered, mut binds) = render_parameterized_sql(sql, param_values, &ds.db_type, &mut idx)?;

    // Apply the outer filter WHERE (方案B/C), continuing the placeholder count.
    let final_sql = match build_filtered_sql_from(&rendered, filters, &ds.db_type, &mut idx)? {
        Some((wrapped, mut filter_binds)) => {
            binds.append(&mut filter_binds);
            wrapped
        }
        None => rendered,
    };

    // Re-validate the assembled statement (read-only guarantee).
    validate_sql(&final_sql)?;

    let fut = async {
        match ds.db_type.as_str() {
            "mysql" => execute_mysql_params(state, ds, &final_sql, &binds).await,
            "postgresql" => execute_postgres_params(state, ds, &final_sql, &binds).await,
            "oracle" => execute_oracle_params(state, ds, &final_sql, &binds).await,
            other => Err(format!("Unsupported database type: {}", other)),
        }
    };

    timeout(Duration::from_secs(QUERY_TIMEOUT_SECS), fut)
        .await
        .map_err(|_| format!("Query timed out after {} seconds", QUERY_TIMEOUT_SECS))?
}

pub async fn get_pool(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(pool_id): Path<i32>,
) -> Result<Json<DataPool>, (StatusCode, String)> {
    let pool = sqlx::query_as::<_, DataPool>("SELECT * FROM data_pools WHERE id = ?")
        .bind(pool_id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data pool not found".to_string()))?;

    // A data pool exposes rows from a datasource — gate it by datasource access.
    crate::routes::datasources::ensure_access(&state, pool.datasource_id, &user).await?;

    Ok(Json(pool))
}

// ── Bind helpers: JSON value -> typed DB bind ──

/// Bind a JSON filter value onto a MySQL query, preserving its type so numeric
/// comparisons don't degrade to string comparisons.
fn bind_mysql<'q>(
    q: sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>,
    v: &serde_json::Value,
) -> sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments> {
    match v {
        serde_json::Value::String(s) => q.bind(s.clone()),
        serde_json::Value::Bool(b) => q.bind(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                q.bind(n.to_string())
            }
        }
        serde_json::Value::Null => q.bind(Option::<String>::None),
        other => q.bind(other.to_string()),
    }
}

/// Bind a JSON filter value onto a PostgreSQL query (see `bind_mysql`).
fn bind_postgres<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    v: &serde_json::Value,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match v {
        serde_json::Value::String(s) => q.bind(s.clone()),
        serde_json::Value::Bool(b) => q.bind(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                q.bind(n.to_string())
            }
        }
        serde_json::Value::Null => q.bind(Option::<String>::None),
        other => q.bind(other.to_string()),
    }
}

/// Convert a JSON filter value into an owned Oracle bind value.
fn oracle_bind_value(v: &serde_json::Value) -> Box<dyn oracle::sql_type::ToSql> {
    match v {
        serde_json::Value::String(s) => Box::new(s.clone()),
        serde_json::Value::Bool(b) => Box::new(if *b { 1i64 } else { 0i64 }),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::Null => Box::new(Option::<String>::None),
        other => Box::new(other.to_string()),
    }
}

// ── Per-DB query execution helpers ──

pub async fn execute_mysql(state: &AppState, ds: &DataSource, sql: &str) -> Result<QueryResult, String> {
    execute_mysql_params(state, ds, sql, &[]).await
}

pub async fn execute_mysql_params(
    state: &AppState,
    ds: &DataSource,
    sql: &str,
    binds: &[serde_json::Value],
) -> Result<QueryResult, String> {
    use futures::TryStreamExt;
    let pool = state.pool_cache.get_mysql(ds).await?;

    // Stream rows and stop at MAX_ROWS so an oversized result set cannot exhaust
    // memory (this path is also used by the snapshot/alert schedulers).
    let mut query = sqlx::query(sql);
    for v in binds {
        query = bind_mysql(query, v);
    }
    let mut stream = query.fetch(&pool);
    let mut columns: Vec<String> = Vec::new();
    let mut json_rows: Vec<serde_json::Value> = Vec::new();

    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|e| format!("MySQL query error: {}", e))?
    {
        if columns.is_empty() {
            columns = row.columns().iter().map(|c| c.name().to_string()).collect();
        }
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            obj.insert(col.name().to_string(), mysql_column_to_json(&row, i));
        }
        json_rows.push(serde_json::Value::Object(obj));
        if json_rows.len() >= MAX_ROWS {
            break;
        }
    }

    let row_count = json_rows.len();
    Ok(QueryResult { columns, rows: json_rows, row_count })
}

/// Extract a MySQL column value as a proper JSON type (number, bool, string, null).
fn mysql_column_to_json(row: &sqlx::mysql::MySqlRow, idx: usize) -> serde_json::Value {
    // Integer types
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        return v.map(|n| serde_json::Value::Number(n.into())).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<u64>, _>(idx) {
        return v.map(|n| serde_json::Value::Number(n.into())).unwrap_or(serde_json::Value::Null);
    }
    // Floating point
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return match v {
            Some(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| serde_json::Value::String(f.to_string())),
            None => serde_json::Value::Null,
        };
    }
    // Boolean
    if let Ok(v) = row.try_get::<Option<bool>, _>(idx) {
        return v.map(serde_json::Value::Bool).unwrap_or(serde_json::Value::Null);
    }
    // JSON columns — return the parsed JSON value directly
    if let Ok(v) = row.try_get::<Option<serde_json::Value>, _>(idx) {
        return v.unwrap_or(serde_json::Value::Null);
    }
    // DATETIME / TIMESTAMP
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(idx) {
        return v.map(|dt| serde_json::Value::String(dt.format("%Y-%m-%d %H:%M:%S").to_string()))
            .unwrap_or(serde_json::Value::Null);
    }
    // DATE
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(idx) {
        return v.map(|d| serde_json::Value::String(d.format("%Y-%m-%d").to_string()))
            .unwrap_or(serde_json::Value::Null);
    }
    // TIME
    if let Ok(v) = row.try_get::<Option<chrono::NaiveTime>, _>(idx) {
        return v.map(|t| serde_json::Value::String(t.format("%H:%M:%S").to_string()))
            .unwrap_or(serde_json::Value::Null);
    }
    // String / text (also catches DECIMAL rendered as string by sqlx)
    if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
        return v.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null);
    }
    // Raw bytes (BLOB / BINARY) — represent as base64-ish length marker
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(idx) {
        return match v {
            Some(bytes) => serde_json::Value::String(String::from_utf8_lossy(&bytes).to_string()),
            None => serde_json::Value::Null,
        };
    }
    serde_json::Value::Null
}

pub async fn execute_postgres(state: &AppState, ds: &DataSource, sql: &str) -> Result<QueryResult, String> {
    execute_postgres_params(state, ds, sql, &[]).await
}

pub async fn execute_postgres_params(
    state: &AppState,
    ds: &DataSource,
    sql: &str,
    binds: &[serde_json::Value],
) -> Result<QueryResult, String> {
    use futures::TryStreamExt;
    let pool = state.pool_cache.get_postgres(ds).await?;

    // Stream and cap at MAX_ROWS (see execute_mysql).
    let mut query = sqlx::query(sql);
    for v in binds {
        query = bind_postgres(query, v);
    }
    let mut stream = query.fetch(&pool);
    let mut columns: Vec<String> = Vec::new();
    let mut json_rows: Vec<serde_json::Value> = Vec::new();

    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|e| format!("PostgreSQL query error: {}", e))?
    {
        if columns.is_empty() {
            columns = row.columns().iter().map(|c| c.name().to_string()).collect();
        }
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            obj.insert(col.name().to_string(), pg_column_to_json(&row, i));
        }
        json_rows.push(serde_json::Value::Object(obj));
        if json_rows.len() >= MAX_ROWS {
            break;
        }
    }

    let row_count = json_rows.len();
    Ok(QueryResult { columns, rows: json_rows, row_count })
}

/// Extract a PostgreSQL column value as a proper JSON type.
fn pg_column_to_json(row: &sqlx::postgres::PgRow, idx: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        return v.map(|n| serde_json::Value::Number(n.into())).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(idx) {
        return v.map(|n| serde_json::Value::Number(n.into())).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return match v {
            Some(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| serde_json::Value::String(f.to_string())),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(idx) {
        return v.map(serde_json::Value::Bool).unwrap_or(serde_json::Value::Null);
    }
    // JSON / JSONB columns
    if let Ok(v) = row.try_get::<Option<serde_json::Value>, _>(idx) {
        return v.unwrap_or(serde_json::Value::Null);
    }
    // TIMESTAMP
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(idx) {
        return v.map(|dt| serde_json::Value::String(dt.format("%Y-%m-%d %H:%M:%S").to_string()))
            .unwrap_or(serde_json::Value::Null);
    }
    // DATE
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(idx) {
        return v.map(|d| serde_json::Value::String(d.format("%Y-%m-%d").to_string()))
            .unwrap_or(serde_json::Value::Null);
    }
    // TIME
    if let Ok(v) = row.try_get::<Option<chrono::NaiveTime>, _>(idx) {
        return v.map(|t| serde_json::Value::String(t.format("%H:%M:%S").to_string()))
            .unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
        return v.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null);
    }
    serde_json::Value::Null
}

pub async fn execute_oracle(state: &AppState, ds: &DataSource, sql: &str) -> Result<QueryResult, String> {
    execute_oracle_params(state, ds, sql, &[]).await
}

pub async fn execute_oracle_params(
    state: &AppState,
    ds: &DataSource,
    sql: &str,
    binds: &[serde_json::Value],
) -> Result<QueryResult, String> {
    let pool = state.pool_cache.get_oracle(ds).await?;
    let sql_owned = sql.to_string();
    let binds_owned: Vec<serde_json::Value> = binds.to_vec();

    tokio::task::spawn_blocking(move || -> Result<QueryResult, String> {
        let conn = pool.get()
            .map_err(|e| format!("Oracle pool get failed: {}", e))?;

        // Bind values are positional (`:1`, `:2`, ...) — build owned Oracle
        // values, then a parallel Vec of trait-object refs to pass to `query`.
        let ora_params: Vec<Box<dyn oracle::sql_type::ToSql>> = binds_owned
            .iter()
            .map(oracle_bind_value)
            .collect();
        let param_refs: Vec<&dyn oracle::sql_type::ToSql> =
            ora_params.iter().map(|b| b.as_ref()).collect();

        let rows_result = conn.query(&sql_owned, &param_refs)
            .map_err(|e| format!("Oracle query error: {}", e))?;

        let col_info: Vec<String> = rows_result
            .column_info()
            .iter()
            .map(|info| info.name().to_string())
            .collect();

        let columns = col_info.clone();

        let json_rows: Vec<serde_json::Value> = rows_result
            .filter_map(|r| r.ok())
            .map(|row| {
                let mut obj = serde_json::Map::new();
                for col_name in &col_info {
                    let val = oracle_value_to_json(&row, col_name);
                    obj.insert(col_name.clone(), val);
                }
                serde_json::Value::Object(obj)
            })
            .take(MAX_ROWS)
            .collect();

        let row_count = json_rows.len();
        // Connection returns to the pool automatically on drop.
        Ok(QueryResult { columns, rows: json_rows, row_count })
    })
    .await
    .map_err(|e| format!("Oracle spawn: {}", e))?
}

/// Extract an Oracle column value as a proper JSON type (number, string, null).
fn oracle_value_to_json(row: &oracle::Row, col: &str) -> serde_json::Value {
    // Integer
    if let Ok(v) = row.get::<&str, Option<i64>>(col) {
        return v.map(|n| serde_json::Value::Number(n.into())).unwrap_or(serde_json::Value::Null);
    }
    // Floating point / NUMBER with decimals
    if let Ok(v) = row.get::<&str, Option<f64>>(col) {
        return match v {
            Some(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| serde_json::Value::String(f.to_string())),
            None => serde_json::Value::Null,
        };
    }
    // Everything else (VARCHAR2, CHAR, DATE, TIMESTAMP, CLOB) as string.
    // The oracle crate renders DATE/TIMESTAMP to their string representation.
    if let Ok(v) = row.get::<&str, Option<String>>(col) {
        return v.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null);
    }
    serde_json::Value::Null
}

#[cfg(test)]
mod validate_tests {
    use super::validate_sql;

    #[test]
    fn allows_read_only_statements() {
        for sql in [
            "SELECT * FROM users",
            "select id, name from orders where total > 100",
            "SHOW TABLES",
            "DESCRIBE users",
            "DESC users",
            "EXPLAIN SELECT * FROM users",
            "WITH t AS (SELECT 1) SELECT * FROM t",
            "SELECT * FROM users;", // single trailing semicolon is fine
        ] {
            assert!(validate_sql(sql).is_ok(), "should allow: {sql}");
        }
    }

    #[test]
    fn blocks_mutations() {
        for sql in [
            "INSERT INTO users (name) VALUES ('x')",
            "UPDATE users SET name = 'x'",
            "DELETE FROM users",
            "DROP TABLE users",
            "ALTER TABLE users ADD COLUMN x INT",
            "TRUNCATE users",
            "CREATE TABLE t (id INT)",
            "REPLACE INTO users VALUES (1)",
            "GRANT ALL ON *.* TO 'x'",
            "CALL some_proc()",
            "SET @x = 1",
            "FLUSH PRIVILEGES",
        ] {
            assert!(validate_sql(sql).is_err(), "should block: {sql}");
        }
    }

    #[test]
    fn blocks_stacked_statements() {
        assert!(validate_sql("SELECT 1; DROP TABLE users").is_err());
        assert!(validate_sql("SELECT 1; SELECT 2").is_err());
        assert!(validate_sql("SELECT 1; DROP TABLE users;").is_err());
    }

    #[test]
    fn blocks_into_outfile_and_variable() {
        assert!(validate_sql("SELECT * FROM users INTO OUTFILE '/tmp/x'").is_err());
        assert!(validate_sql("SELECT * FROM users INTO DUMPFILE '/tmp/x'").is_err());
        assert!(validate_sql("SELECT id INTO @v FROM users").is_err());
    }

    #[test]
    fn blocks_mutation_hidden_after_comment() {
        // A mutation smuggled after a leading comment must still be caught: the
        // tokenizer skips the comment, so DROP becomes the first real token.
        assert!(validate_sql("/* comment */ DROP TABLE users").is_err());
        assert!(validate_sql("-- harmless\nDELETE FROM users").is_err());
    }

    #[test]
    fn forbidden_keywords_inside_string_literals_are_not_flagged() {
        // Keywords appearing only inside a string value are data, not SQL verbs,
        // and must not trip the allowlist.
        assert!(validate_sql("SELECT 'drop table users' AS note").is_ok());
        assert!(validate_sql("SELECT * FROM logs WHERE msg = 'delete failed'").is_ok());
    }

    #[test]
    fn forbidden_keywords_inside_comments_are_ignored() {
        assert!(validate_sql("SELECT 1 -- DROP TABLE users").is_ok());
        assert!(validate_sql("SELECT 1 /* DELETE FROM users */").is_ok());
    }

    #[test]
    fn rejects_empty_query() {
        assert!(validate_sql("").is_err());
        assert!(validate_sql("   ").is_err());
    }

    #[test]
    fn rejects_non_read_first_token() {
        // A bare identifier / unknown verb is not on the allowlist.
        assert!(validate_sql("PRAGMA foo").is_err());
        assert!(validate_sql("USE mydb").is_err());
    }
}

#[cfg(test)]
mod filter_tests {
    use super::build_filtered_sql;
    use crate::models::FilterCondition;
    use serde_json::json;

    fn cond(column: &str, op: &str, value: serde_json::Value) -> FilterCondition {
        FilterCondition { column: column.to_string(), op: op.to_string(), value }
    }

    #[test]
    fn empty_filters_returns_none() {
        // No filters => caller runs the base SQL unchanged.
        let out = build_filtered_sql("SELECT * FROM orders", &[], "mysql").unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn wraps_and_binds_scalar_conditions() {
        let filters = vec![
            cond("region", "=", json!("华东")),
            cond("total", ">=", json!(100)),
        ];
        let (sql, binds) = build_filtered_sql("SELECT * FROM orders", &filters, "mysql")
            .unwrap()
            .unwrap();
        assert_eq!(
            sql,
            "SELECT * FROM (SELECT * FROM orders) AS _m WHERE _m.region = ? AND _m.total >= ?"
        );
        assert_eq!(binds, vec![json!("华东"), json!(100)]);
    }

    #[test]
    fn strips_trailing_semicolon_from_base() {
        let filters = vec![cond("id", "=", json!(1))];
        let (sql, _) = build_filtered_sql("SELECT * FROM t;", &filters, "mysql")
            .unwrap()
            .unwrap();
        assert_eq!(sql, "SELECT * FROM (SELECT * FROM t) AS _m WHERE _m.id = ?");
    }

    #[test]
    fn postgres_uses_numbered_placeholders() {
        let filters = vec![cond("a", "=", json!(1)), cond("b", "<", json!(2))];
        let (sql, _) = build_filtered_sql("SELECT * FROM t", &filters, "postgresql")
            .unwrap()
            .unwrap();
        assert!(sql.ends_with("WHERE _m.a = $1 AND _m.b < $2"));
    }

    #[test]
    fn oracle_uses_colon_placeholders() {
        let filters = vec![cond("a", "=", json!(1))];
        let (sql, _) = build_filtered_sql("SELECT * FROM t", &filters, "oracle")
            .unwrap()
            .unwrap();
        assert!(sql.ends_with("WHERE _m.a = :1"));
    }

    #[test]
    fn in_expands_to_placeholder_list() {
        let filters = vec![cond("region", "IN", json!(["A", "B", "C"]))];
        let (sql, binds) = build_filtered_sql("SELECT * FROM t", &filters, "mysql")
            .unwrap()
            .unwrap();
        assert!(sql.ends_with("WHERE _m.region IN (?, ?, ?)"));
        assert_eq!(binds, vec![json!("A"), json!("B"), json!("C")]);
    }

    #[test]
    fn between_takes_two_values() {
        let filters = vec![cond("d", "BETWEEN", json!(["2024-01-01", "2024-12-31"]))];
        let (sql, binds) = build_filtered_sql("SELECT * FROM t", &filters, "mysql")
            .unwrap()
            .unwrap();
        assert!(sql.ends_with("WHERE _m.d BETWEEN ? AND ?"));
        assert_eq!(binds.len(), 2);
    }

    #[test]
    fn ne_is_canonicalized() {
        let filters = vec![cond("a", "!=", json!(1))];
        let (sql, _) = build_filtered_sql("SELECT * FROM t", &filters, "mysql")
            .unwrap()
            .unwrap();
        assert!(sql.contains("_m.a <> ?"));
    }

    #[test]
    fn rejects_bad_column_identifier() {
        for bad in ["a b", "a;drop", "1a", "a-b", "a)", ""] {
            let filters = vec![cond(bad, "=", json!(1))];
            assert!(
                build_filtered_sql("SELECT * FROM t", &filters, "mysql").is_err(),
                "should reject column: {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_unknown_operator() {
        let filters = vec![cond("a", "; DROP TABLE t --", json!(1))];
        assert!(build_filtered_sql("SELECT * FROM t", &filters, "mysql").is_err());
    }

    #[test]
    fn rejects_empty_in_and_bad_between() {
        assert!(build_filtered_sql(
            "SELECT * FROM t",
            &[cond("a", "IN", json!([]))],
            "mysql"
        )
        .is_err());
        assert!(build_filtered_sql(
            "SELECT * FROM t",
            &[cond("a", "BETWEEN", json!([1]))],
            "mysql"
        )
        .is_err());
    }
}

#[cfg(test)]
mod param_tests {
    use super::{build_filtered_sql_from, render_parameterized_sql};
    use crate::models::FilterCondition;
    use serde_json::json;
    use std::collections::HashMap;

    fn vals(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn no_placeholders_unchanged() {
        let mut idx = 1;
        let (sql, binds) =
            render_parameterized_sql("SELECT * FROM t", &vals(&[]), "mysql", &mut idx).unwrap();
        assert_eq!(sql, "SELECT * FROM t");
        assert!(binds.is_empty());
        assert_eq!(idx, 1);
    }

    #[test]
    fn optional_block_included_when_value_present() {
        let mut idx = 1;
        let (sql, binds) = render_parameterized_sql(
            "SELECT * FROM t WHERE 1=1 [[ AND region = {{region}} ]]",
            &vals(&[("region", json!("华东"))]),
            "mysql",
            &mut idx,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE 1=1  AND region = ? ");
        assert_eq!(binds, vec![json!("华东")]);
    }

    #[test]
    fn optional_block_dropped_when_value_absent() {
        let mut idx = 1;
        let (sql, binds) = render_parameterized_sql(
            "SELECT * FROM t WHERE 1=1 [[ AND region = {{region}} ]]",
            &vals(&[]),
            "mysql",
            &mut idx,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE 1=1 ");
        assert!(binds.is_empty());
    }

    #[test]
    fn optional_block_dropped_when_value_blank() {
        let mut idx = 1;
        let (sql, _) = render_parameterized_sql(
            "SELECT * FROM t WHERE 1=1 [[ AND region = {{region}} ]]",
            &vals(&[("region", json!("  "))]),
            "mysql",
            &mut idx,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE 1=1 ");
    }

    #[test]
    fn bare_placeholder_requires_value() {
        let mut idx = 1;
        assert!(render_parameterized_sql(
            "SELECT * FROM t WHERE d = {{d}}",
            &vals(&[]),
            "mysql",
            &mut idx,
        )
        .is_err());
    }

    #[test]
    fn bare_placeholder_substituted() {
        let mut idx = 1;
        let (sql, binds) = render_parameterized_sql(
            "SELECT * FROM t WHERE d = {{d}}",
            &vals(&[("d", json!("2024-01-01"))]),
            "postgresql",
            &mut idx,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE d = $1");
        assert_eq!(binds, vec![json!("2024-01-01")]);
    }

    #[test]
    fn continuous_numbering_params_then_filters_postgres() {
        // Param placeholders come first ($1), then the filter WHERE continues ($2).
        let mut idx = 1;
        let (rendered, mut binds) = render_parameterized_sql(
            "SELECT region, amount FROM t WHERE 1=1 [[ AND ts >= {{start}} ]]",
            &vals(&[("start", json!("2024-01-01"))]),
            "postgresql",
            &mut idx,
        )
        .unwrap();
        let filters = vec![FilterCondition { column: "region".into(), op: "=".into(), value: json!("华东") }];
        let (wrapped, more) =
            build_filtered_sql_from(&rendered, &filters, "postgresql", &mut idx).unwrap().unwrap();
        binds.append(&mut { more });
        assert!(rendered.contains("$1"));
        assert!(wrapped.contains("_m.region = $2"));
        assert_eq!(binds.len(), 2);
    }

    #[test]
    fn multiple_blocks_mixed() {
        let mut idx = 1;
        let (sql, binds) = render_parameterized_sql(
            "SELECT * FROM t WHERE 1=1 [[ AND a = {{a}} ]] [[ AND b = {{b}} ]]",
            &vals(&[("b", json!(5))]),
            "mysql",
            &mut idx,
        )
        .unwrap();
        // 'a' block dropped, 'b' block kept.
        assert_eq!(sql, "SELECT * FROM t WHERE 1=1   AND b = ? ");
        assert_eq!(binds, vec![json!(5)]);
    }
}

#[cfg(test)]
mod report_filter_tests {
    use super::{combined_filters, report_filters_to_conditions};
    use serde_json::json;

    #[test]
    fn maps_active_controls_to_targeted_datasets() {
        let rf = json!([
            {
                "key": "region", "op": "=", "value": "华东",
                "targets": [
                    {"datasource_id": 1, "column": "region"},
                    {"datasource_id": 2, "column": "area"}
                ]
            }
        ]);
        // Dataset 1 gets region=..., dataset 2 gets area=..., dataset 3 nothing.
        let c1 = report_filters_to_conditions(&rf, 1);
        assert_eq!(c1.len(), 1);
        assert_eq!(c1[0].column, "region");
        let c2 = report_filters_to_conditions(&rf, 2);
        assert_eq!(c2[0].column, "area");
        assert!(report_filters_to_conditions(&rf, 3).is_empty());
    }

    #[test]
    fn blank_controls_are_skipped() {
        // Empty string, null, empty array, and half-filled range => inactive.
        let rf = json!([
            {"key": "a", "op": "=", "value": "", "targets": [{"datasource_id": 1, "column": "a"}]},
            {"key": "b", "op": "=", "value": null, "targets": [{"datasource_id": 1, "column": "b"}]},
            {"key": "c", "op": "IN", "value": [], "targets": [{"datasource_id": 1, "column": "c"}]},
            {"key": "d", "op": "BETWEEN", "value": ["2024-01-01", ""], "targets": [{"datasource_id": 1, "column": "d"}]}
        ]);
        assert!(report_filters_to_conditions(&rf, 1).is_empty());
    }

    #[test]
    fn combined_appends_report_conditions_after_dataset() {
        let ds_filters = vec![super::FilterCondition {
            column: "status".into(),
            op: "=".into(),
            value: json!("paid"),
        }];
        let rf = json!([
            {"key": "region", "op": "=", "value": "华东", "targets": [{"datasource_id": 7, "column": "region"}]}
        ]);
        let all = combined_filters(ds_filters, &Some(rf), 7);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].column, "status");
        assert_eq!(all[1].column, "region");
    }

    #[test]
    fn combined_without_report_filters_is_unchanged() {
        let ds_filters = vec![super::FilterCondition {
            column: "x".into(),
            op: ">".into(),
            value: json!(1),
        }];
        let all = combined_filters(ds_filters, &None, 1);
        assert_eq!(all.len(), 1);
    }
}
