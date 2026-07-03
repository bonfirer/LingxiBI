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

    // Validate + execute with shared safety guards (timeout + row cap).
    let query_result = execute_validated(&state, &ds, &req.sql)
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

// ── Per-DB query execution helpers ──

pub async fn execute_mysql(state: &AppState, ds: &DataSource, sql: &str) -> Result<QueryResult, String> {
    use futures::TryStreamExt;
    let pool = state.pool_cache.get_mysql(ds).await?;

    // Stream rows and stop at MAX_ROWS so an oversized result set cannot exhaust
    // memory (this path is also used by the snapshot/alert schedulers).
    let mut stream = sqlx::query(sql).fetch(&pool);
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
    use futures::TryStreamExt;
    let pool = state.pool_cache.get_postgres(ds).await?;

    // Stream and cap at MAX_ROWS (see execute_mysql).
    let mut stream = sqlx::query(sql).fetch(&pool);
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
    let pool = state.pool_cache.get_oracle(ds).await?;
    let sql_owned = sql.to_string();

    tokio::task::spawn_blocking(move || -> Result<QueryResult, String> {
        let conn = pool.get()
            .map_err(|e| format!("Oracle pool get failed: {}", e))?;

        let rows_result = conn.query(&sql_owned, &[])
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
