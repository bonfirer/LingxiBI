//! Column profiling: sample values, distinct counts, min/max for each column.
//! This gives the AI much better context about actual data content.

use sqlx::MySqlPool;
use crate::models::DataSource;
use crate::AppState;

/// Escape a MySQL identifier (table/column) for safe interpolation:
/// wrap in backticks and double any embedded backtick.
fn mysql_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Escape a PostgreSQL/Oracle identifier: wrap in double quotes and double any
/// embedded double quote.
fn quoted_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Profile all columns of all tables for a given datasource.
/// Stores results in `column_profiles` table.
pub async fn profile_datasource(state: &AppState, ds: &DataSource) -> Result<u32, String> {
    match ds.db_type.as_str() {
        "mysql" => profile_mysql(state, ds).await,
        "postgresql" => profile_postgres(state, ds).await,
        "oracle" => profile_oracle(state, ds).await,
        _ => Ok(0),
    }
}

async fn profile_mysql(state: &AppState, ds: &DataSource) -> Result<u32, String> {   
    let pool = state.pool_cache.get_mysql(ds).await?;

    // Get all tables
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT CAST(TABLE_NAME AS CHAR) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE'",
    )
    .bind(&ds.database_name)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut profiled = 0u32;

    for (table_name,) in &tables {
        // Get columns
        let columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT CAST(COLUMN_NAME AS CHAR), CAST(DATA_TYPE AS CHAR) FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
        )
        .bind(&ds.database_name)
        .bind(table_name)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;

        // Get row count (approx)
        let row_count: Option<(i64,)> = sqlx::query_as(
            &format!("SELECT COUNT(*) FROM {}", mysql_ident(table_name))
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();
        let total = row_count.map(|(c,)| c as i32).unwrap_or(0);

        for (col_name, data_type) in &columns {
            let profile = profile_mysql_column(&pool, table_name, col_name, data_type, total).await;
            if let Some((distinct, nulls, min_val, max_val, samples)) = profile {
                let samples_json = serde_json::to_value(&samples).ok();

                sqlx::query(
                    "INSERT INTO column_profiles (datasource_id, table_name, column_name, distinct_count, null_count, total_count, min_value, max_value, sample_values, profiled_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NOW())
                     ON DUPLICATE KEY UPDATE distinct_count=VALUES(distinct_count), null_count=VALUES(null_count),
                       total_count=VALUES(total_count), min_value=VALUES(min_value), max_value=VALUES(max_value),
                       sample_values=VALUES(sample_values), profiled_at=NOW()"
                )
                .bind(ds.id)
                .bind(table_name)
                .bind(col_name)
                .bind(distinct)
                .bind(nulls)
                .bind(total)
                .bind(&min_val)
                .bind(&max_val)
                .bind(&samples_json)
                .execute(&state.db)
                .await
                .ok();

                profiled += 1;
            }
        }
    }

    Ok(profiled)
}

async fn profile_mysql_column(
    pool: &MySqlPool,
    table: &str,
    column: &str,
    data_type: &str,
    _total: i32,
) -> Option<(i32, i32, Option<String>, Option<String>, Vec<String>)> {
    // For very large tables, use approximate methods
    let _is_text = matches!(
        data_type,
        "varchar" | "char" | "text" | "tinytext" | "mediumtext" | "longtext" | "enum" | "set"
    );
    let is_numeric = matches!(
        data_type,
        "int" | "bigint" | "smallint" | "tinyint" | "decimal" | "float" | "double" | "mediumint"
    );

    // Get distinct count and null count
    let stats: Option<(i64, i64)> = sqlx::query_as(
        &format!(
            "SELECT COUNT(DISTINCT {col}) AS dc, SUM(CASE WHEN {col} IS NULL THEN 1 ELSE 0 END) AS nc FROM {tbl}",
            col = mysql_ident(column), tbl = mysql_ident(table)
        )
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let (distinct, nulls) = stats.unwrap_or((0, 0));

    // Get min/max for numeric and date types
    let (min_val, max_val) = if is_numeric || data_type.contains("date") || data_type.contains("time") {
        let minmax: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            &format!(
                "SELECT CAST(MIN({col}) AS CHAR), CAST(MAX({col}) AS CHAR) FROM {tbl}",
                col = mysql_ident(column), tbl = mysql_ident(table)
            )
        )
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        minmax.unwrap_or((None, None))
    } else {
        (None, None)
    };

    // Get sample distinct values (up to 10 for enums/low-cardinality, 5 otherwise)
    let sample_limit = if distinct <= 20 { 20 } else { 5 };
    let samples: Vec<(Option<String>,)> = sqlx::query_as(
        &format!(
            "SELECT CAST({col} AS CHAR) FROM {tbl} WHERE {col} IS NOT NULL GROUP BY {col} LIMIT {lim}",
            col = mysql_ident(column), tbl = mysql_ident(table), lim = sample_limit
        )
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let sample_values: Vec<String> = samples
        .into_iter()
        .filter_map(|(v,)| v)
        .filter(|v| v.len() <= 100) // Skip very long values
        .collect();

    Some((
        distinct as i32,
        nulls as i32,
        min_val,
        max_val,
        sample_values,
    ))
}

async fn profile_postgres(state: &AppState, ds: &DataSource) -> Result<u32, String> {
    let pool = state.pool_cache.get_postgres(ds).await?;

    // Read the catalog rather than `information_schema`: the latter hides
    // objects the connected role has no privilege on, which makes profiling
    // silently no-op for read-only accounts without an explicit GRANT.
    // Only real/partitioned tables are profiled — views and foreign tables
    // would re-run their underlying query for every single aggregate.
    let tables: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT c.oid::int8, n.nspname::text, c.relname::text
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE c.relkind IN ('r', 'p')
           AND NOT c.relispartition
           AND n.nspname NOT IN ('pg_catalog', 'information_schema')
           AND left(n.nspname, 3) <> 'pg_'
         ORDER BY n.nspname, c.relname",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut profiled = 0u32;

    for (relid, schema_name, rel_name) in &tables {
        // Column profiles are keyed by the same name introspection stores, so
        // the AI can join them against the schema it was given.
        let table_name = &crate::routes::datasources::pg_qualified_name(schema_name, rel_name);
        // ...but SQL needs each identifier quoted separately.
        let table_ref = format!("{}.{}", quoted_ident(schema_name), quoted_ident(rel_name));

        // Looked up by oid: resolving a name via `regclass` needs USAGE on the
        // schema, which a restricted role may not hold.
        let columns: Vec<(String, String)> = match sqlx::query_as(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod)::text
             FROM pg_attribute a
             WHERE a.attrelid = $1::int8::oid
               AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum",
        )
        .bind(relid)
        .fetch_all(&pool)
        .await
        {
            Ok(cols) => cols,
            Err(e) => {
                // One unreadable table must not abort the whole profiling run.
                tracing::debug!("skip profiling {}: {}", table_name, e);
                continue;
            }
        };

        let row_count: Option<(i64,)> = sqlx::query_as(
            &format!("SELECT COUNT(*) FROM {}", table_ref)
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();
        let total = row_count.map(|(c,)| c as i32).unwrap_or(0);

        for (col_name, data_type) in &columns {
            // Get stats
            let stats: Option<(i64, i64)> = sqlx::query_as(
                &format!(
                    "SELECT COUNT(DISTINCT {col}), SUM(CASE WHEN {col} IS NULL THEN 1 ELSE 0 END) FROM {tbl}",
                    col = quoted_ident(col_name), tbl = table_ref
                )
            )
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

            let (distinct, nulls) = stats.unwrap_or((0, 0));

            // `format_type` includes modifiers (e.g. `numeric(10,2)`), so match
            // on a prefix rather than the whole string.
            let base_type = data_type.split('(').next().unwrap_or(data_type).trim();
            let is_numeric = matches!(
                base_type,
                "smallint" | "integer" | "bigint" | "numeric" | "decimal" | "real" | "double precision" | "money"
            );

            let (min_val, max_val) = if is_numeric || base_type.contains("date") || base_type.contains("time") {
                let minmax: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                    &format!(
                        "SELECT CAST(MIN({col}) AS TEXT), CAST(MAX({col}) AS TEXT) FROM {tbl}",
                        col = quoted_ident(col_name), tbl = table_ref
                    )
                )
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();
                minmax.unwrap_or((None, None))
            } else {
                (None, None)
            };

            let sample_limit = if distinct <= 20 { 20 } else { 5 };
            let samples: Vec<(Option<String>,)> = sqlx::query_as(
                &format!(
                    "SELECT CAST({col} AS TEXT) FROM {tbl} WHERE {col} IS NOT NULL GROUP BY {col} LIMIT {lim}",
                    col = quoted_ident(col_name), tbl = table_ref, lim = sample_limit
                )
            )
            .fetch_all(&pool)
            .await
            .unwrap_or_default();

            let sample_values: Vec<String> = samples
                .into_iter()
                .filter_map(|(v,)| v)
                .filter(|v| v.len() <= 100)
                .collect();

            let samples_json = serde_json::to_value(&sample_values).ok();

            sqlx::query(
                "INSERT INTO column_profiles (datasource_id, table_name, column_name, distinct_count, null_count, total_count, min_value, max_value, sample_values, profiled_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NOW())
                 ON DUPLICATE KEY UPDATE distinct_count=VALUES(distinct_count), null_count=VALUES(null_count),
                   total_count=VALUES(total_count), min_value=VALUES(min_value), max_value=VALUES(max_value),
                   sample_values=VALUES(sample_values), profiled_at=NOW()"
            )
            .bind(ds.id)
            .bind(table_name)
            .bind(col_name)
            .bind(distinct as i32)
            .bind(nulls as i32)
            .bind(total)
            .bind(&min_val)
            .bind(&max_val)
            .bind(&samples_json)
            .execute(&state.db)
            .await
            .ok();

            profiled += 1;
        }
    }

    Ok(profiled)
}

/// Profile Oracle columns. The oracle crate is synchronous, so all DB work
/// runs on a blocking thread. Results are collected then written via sqlx.
async fn profile_oracle(state: &AppState, ds: &DataSource) -> Result<u32, String> {
    let pool = state.pool_cache.get_oracle(ds).await?;

    // Collect profiles on a blocking thread, then write them with sqlx afterwards.
    type ColProfile = (String, String, i32, i32, i32, Option<String>, Option<String>, Vec<String>);
    let collected: Vec<ColProfile> = tokio::task::spawn_blocking(move || -> Result<Vec<ColProfile>, String> {
        let conn = pool.get().map_err(|e| format!("Oracle pool get: {}", e))?;

        // Get all user tables
        let table_rows = conn
            .query_as::<(String,)>("SELECT TABLE_NAME FROM USER_TABLES ORDER BY TABLE_NAME", &[])
            .map_err(|e| format!("Oracle tables: {}", e))?;
        let tables: Vec<String> = table_rows.filter_map(|r| r.ok()).map(|(t,)| t).collect();

        let mut out: Vec<ColProfile> = Vec::new();

        for table in &tables {
            // Columns + data types
            let col_rows = conn
                .query_as::<(String, String)>(
                    "SELECT COLUMN_NAME, DATA_TYPE FROM USER_TAB_COLUMNS WHERE TABLE_NAME = :1 ORDER BY COLUMN_ID",
                    &[table],
                )
                .map_err(|e| format!("Oracle columns: {}", e))?;
            let columns: Vec<(String, String)> = col_rows.filter_map(|r| r.ok()).collect();

            // Total row count
            let total: i64 = conn
                .query_row_as::<i64>(&format!("SELECT COUNT(*) FROM {}", quoted_ident(table)), &[])
                .unwrap_or(0);

            for (col, data_type) in &columns {
                let dt = data_type.to_uppercase();
                let is_numeric = dt.contains("NUMBER") || dt.contains("FLOAT") || dt.contains("INTEGER");
                let is_date = dt.contains("DATE") || dt.contains("TIMESTAMP");

                // distinct + null counts
                let stats_sql = format!(
                    "SELECT COUNT(DISTINCT {col}), SUM(CASE WHEN {col} IS NULL THEN 1 ELSE 0 END) FROM {tbl}",
                    col = quoted_ident(col), tbl = quoted_ident(table)
                );
                let (distinct, nulls): (i64, i64) = conn
                    .query_row_as::<(i64, Option<i64>)>(&stats_sql, &[])
                    .map(|(d, n)| (d, n.unwrap_or(0)))
                    .unwrap_or((0, 0));

                // min / max for numeric & date
                let (min_val, max_val) = if is_numeric || is_date {
                    let mm_sql = format!(
                        "SELECT TO_CHAR(MIN({col})), TO_CHAR(MAX({col})) FROM {tbl}",
                        col = quoted_ident(col), tbl = quoted_ident(table)
                    );
                    conn.query_row_as::<(Option<String>, Option<String>)>(&mm_sql, &[])
                        .unwrap_or((None, None))
                } else {
                    (None, None)
                };

                // sample distinct values
                let sample_limit = if distinct > 0 && distinct <= 20 { 20 } else { 5 };
                let sample_sql = format!(
                    "SELECT * FROM (SELECT DISTINCT TO_CHAR({col}) AS v FROM {tbl} WHERE {col} IS NOT NULL) WHERE ROWNUM <= {lim}",
                    col = quoted_ident(col), tbl = quoted_ident(table), lim = sample_limit
                );
                let sample_values: Vec<String> = conn
                    .query_as::<(Option<String>,)>(&sample_sql, &[])
                    .map(|rows| {
                        rows.filter_map(|r| r.ok())
                            .filter_map(|(v,)| v)
                            .filter(|v| v.len() <= 100)
                            .collect()
                    })
                    .unwrap_or_default();

                out.push((
                    table.clone(),
                    col.clone(),
                    distinct as i32,
                    nulls as i32,
                    total as i32,
                    min_val,
                    max_val,
                    sample_values,
                ));
            }
        }

        conn.close().ok();
        Ok(out)
    })
    .await
    .map_err(|e| format!("Oracle profile spawn: {}", e))??;

    // Write collected profiles
    let mut profiled = 0u32;
    for (table, col, distinct, nulls, total, min_val, max_val, samples) in &collected {
        let samples_json = serde_json::to_value(samples).ok();

        sqlx::query(
            "INSERT INTO column_profiles (datasource_id, table_name, column_name, distinct_count, null_count, total_count, min_value, max_value, sample_values, profiled_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NOW())
             ON DUPLICATE KEY UPDATE distinct_count=VALUES(distinct_count), null_count=VALUES(null_count),
               total_count=VALUES(total_count), min_value=VALUES(min_value), max_value=VALUES(max_value),
               sample_values=VALUES(sample_values), profiled_at=NOW()"
        )
        .bind(ds.id)
        .bind(table)
        .bind(col)
        .bind(distinct)
        .bind(nulls)
        .bind(total)
        .bind(min_val)
        .bind(max_val)
        .bind(&samples_json)
        .execute(&state.db)
        .await
        .ok();

        profiled += 1;
    }

    Ok(profiled)
}

/// Build a concise profile context string for the LLM prompt.
pub async fn build_profile_context(state: &AppState, datasource_id: i32) -> String {
    let profiles: Vec<(String, String, i32, i32, i32, Option<String>, Option<String>, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT table_name, column_name, distinct_count, null_count, total_count, min_value, max_value, sample_values
         FROM column_profiles WHERE datasource_id = ? ORDER BY table_name, column_name"
    )
    .bind(datasource_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    if profiles.is_empty() {
        return String::new();
    }

    let mut ctx = String::from("\n### Column Statistics & Sample Values\n");
    let mut current_table = String::new();

    for (table, col, distinct, nulls, total, min_val, max_val, samples) in &profiles {
        // Only include interesting profiles
        let null_pct = if *total > 0 { (*nulls as f64 / *total as f64 * 100.0) as i32 } else { 0 };
        let is_enum_like = *distinct > 0 && *distinct <= 20 && *total > 0;
        let has_range = min_val.is_some() || max_val.is_some();

        if !is_enum_like && !has_range && null_pct < 50 {
            continue; // Skip uninteresting columns
        }

        if *table != current_table {
            ctx.push_str(&format!("{}:\n", table));
            current_table = table.clone();
        }

        let mut parts: Vec<String> = Vec::new();

        if is_enum_like {
            if let Some(vals) = samples {
                if let Some(arr) = vals.as_array() {
                    let values: Vec<&str> = arr.iter()
                        .filter_map(|v| v.as_str())
                        .take(15)
                        .collect();
                    if !values.is_empty() {
                        parts.push(format!("values=[{}]", values.join(", ")));
                    }
                }
            }
        }

        if let (Some(mn), Some(mx)) = (min_val, max_val) {
            parts.push(format!("range: {} ~ {}", mn, mx));
        }

        if null_pct >= 50 {
            parts.push(format!("{}% NULL", null_pct));
        }

        if *distinct > 0 && *total > 0 {
            parts.push(format!("{} distinct", distinct));
        }

        if !parts.is_empty() {
            ctx.push_str(&format!("  {}: {}\n", col, parts.join(" | ")));
        }
    }

    ctx
}
