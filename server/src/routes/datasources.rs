use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::{ensure_admin, internal_error};
use crate::AppState;

/// Whether `user` may access datasource `ds_id`. Admins access all; members
/// need an explicit grant.
pub async fn has_access(
    state: &AppState,
    ds_id: i32,
    user: &AuthUser,
) -> Result<bool, (StatusCode, String)> {
    if user.is_admin {
        return Ok(true);
    }
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT id FROM datasource_grants WHERE datasource_id = ? AND user_id = ?",
    )
    .bind(ds_id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(row.is_some())
}

/// Every datasource id `user` may read: all of them for an admin, exactly the
/// granted ones for a member.
///
/// An empty result means "this member may read nothing" and must be treated as
/// such — never as "no filter needed". Use this anywhere a query would otherwise
/// sweep across all datasources (AI context building, cross-datasource listings).
pub async fn accessible_ids(
    state: &AppState,
    user: &AuthUser,
) -> Result<Vec<i32>, (StatusCode, String)> {
    let rows: Vec<(i32,)> = if user.is_admin {
        sqlx::query_as("SELECT id FROM datasources")
            .fetch_all(&state.db)
            .await
    } else {
        sqlx::query_as("SELECT datasource_id FROM datasource_grants WHERE user_id = ?")
            .bind(user.id)
            .fetch_all(&state.db)
            .await
    }
    .map_err(internal_error)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Enforce datasource access, returning 404 (not 403) so we don't reveal that a
/// datasource the caller can't use exists. Call this from any path that reads
/// data from, or runs SQL against, a datasource on behalf of a member.
pub async fn ensure_access(
    state: &AppState,
    ds_id: i32,
    user: &AuthUser,
) -> Result<(), (StatusCode, String)> {
    if has_access(state, ds_id, user).await? {
        Ok(())
    } else {
        Err((StatusCode::NOT_FOUND, "Data source not found".to_string()))
    }
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateDataSource>,
) -> Result<(StatusCode, Json<DataSource>), (StatusCode, String)> {
    ensure_admin(&user)?;
    let result = sqlx::query(
        "INSERT INTO datasources (name, db_type, host, port, database_name, username, password)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&payload.name)
    .bind(payload.db_type.as_deref().unwrap_or("mysql"))
    .bind(&payload.host)
    .bind(payload.port.unwrap_or(3306))
    .bind(&payload.database_name)
    .bind(&payload.username)
    .bind(crate::crypto::encrypt(&payload.password))
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(ds)))
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<DataSource>>, (StatusCode, String)> {
    // Admins see all datasources; members see only the ones granted to them.
    let sources = if user.is_admin {
        sqlx::query_as::<_, DataSource>("SELECT * FROM datasources ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
    } else {
        sqlx::query_as::<_, DataSource>(
            "SELECT d.* FROM datasources d \
             JOIN datasource_grants g ON g.datasource_id = d.id \
             WHERE g.user_id = ? ORDER BY d.created_at DESC",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await
    }
    .map_err(internal_error)?;

    Ok(Json(sources))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<DataSource>, (StatusCode, String)> {
    ensure_access(&state, id, &user).await?;
    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    Ok(Json(ds))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateDataSource>,
) -> Result<Json<DataSource>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let existing = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // Only overwrite the password when a non-empty value is provided; otherwise
    // keep the existing (already-encrypted) one. `encrypt` is idempotent, so
    // re-encrypting the stored ciphertext leaves it unchanged.
    let password = match payload.password.as_deref() {
        Some(p) if !p.is_empty() => crate::crypto::encrypt(p),
        _ => existing.password.clone(),
    };

    sqlx::query(
        "UPDATE datasources SET name=?, host=?, port=?, database_name=?, username=?, password=? WHERE id=?",
    )
    .bind(payload.name.as_deref().unwrap_or(&existing.name))
    .bind(payload.host.as_deref().unwrap_or(&existing.host))
    .bind(payload.port.unwrap_or(existing.port))
    .bind(payload.database_name.as_deref().unwrap_or(&existing.database_name))
    .bind(payload.username.as_deref().unwrap_or(&existing.username))
    .bind(&password)
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    // Evict cached pool — credentials may have changed
    state.pool_cache.evict(id).await;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(ds))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_admin(&user)?;
    let result = sqlx::query("DELETE FROM datasources WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Data source not found".to_string()));
    }

    // Evict cached pool for this datasource
    state.pool_cache.evict(id).await;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_connection(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    let result = match ds.db_type.as_str() {
        "mysql" => test_conn_mysql(&state, &ds).await,
        "postgresql" => test_conn_postgres(&state, &ds).await,
        "oracle" => test_conn_oracle(&state, &ds).await,
        other => Err(format!("Unsupported database type: {}", other)),
    };

    match result {
        Ok(()) => {
            let _ = sqlx::query("UPDATE datasources SET status='connected' WHERE id=?")
                .bind(id)
                .execute(&state.db)
                .await;
            Ok(Json(serde_json::json!({"status": "connected", "message": "Connection successful"})))
        }
        Err(e) => {
            // Evict failed pool from cache
            state.pool_cache.evict(id).await;
            let _ = sqlx::query("UPDATE datasources SET status='error' WHERE id=?")
                .bind(id)
                .execute(&state.db)
                .await;
            Ok(Json(serde_json::json!({"status": "error", "message": e})))
        }
    }
}

pub async fn introspect(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<SchemaInfo>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    let schema = match ds.db_type.as_str() {
        "mysql" => introspect_mysql(&state, &ds).await,
        "postgresql" => introspect_postgres(&state, &ds).await,
        "oracle" => introspect_oracle(&state, &ds).await,
        other => Err(format!("Unsupported database type: {}", other)),
    }
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    // Save schema
    let schema_json = serde_json::to_value(&schema)
        .map_err(crate::routes::internal_error)?;

    sqlx::query(
        "INSERT INTO `schemas` (datasource_id, schema_data) VALUES (?, ?)
         ON DUPLICATE KEY UPDATE schema_data = VALUES(schema_data)",
    )
    .bind(id)
    .bind(&schema_json)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    // Auto-profile columns in background
    let state_clone = state.clone();
    let ds_clone = ds.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::column_profiler::profile_datasource(&state_clone, &ds_clone).await {
            tracing::warn!("Auto-profile failed for ds={}: {}", ds_clone.id, e);
        } else {
            tracing::info!("Auto-profiled columns for ds={}", ds_clone.id);
        }
    });

    Ok(Json(schema))
}

pub async fn get_schema(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<SchemaInfo>, (StatusCode, String)> {
    ensure_access(&state, id, &user).await?;
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT schema_data FROM `schemas` WHERE datasource_id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;

    match row {
        Some((data,)) => {
            let schema: SchemaInfo = serde_json::from_value(data)
                .map_err(crate::routes::internal_error)?;
            Ok(Json(schema))
        }
        None => Err((StatusCode::NOT_FOUND, "Schema not found. Run introspection first.".to_string())),
    }
}

/// Profile all columns — sample values, distinct counts, min/max.
pub async fn profile(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    let count = crate::column_profiler::profile_datasource(&state, &ds)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "columns_profiled": count
    })))
}

#[derive(serde::Deserialize)]
pub struct SetDefault {
    #[serde(default = "default_true")]
    pub is_default: bool,
}

fn default_true() -> bool {
    true
}

/// PUT /api/datasources/{id}/default — lock this datasource as the default, or
/// clear it (admin only). Setting a default clears the flag on all others so at
/// most one datasource is ever the default.
pub async fn set_default(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<SetDefault>,
) -> Result<Json<DataSource>, (StatusCode, String)> {
    ensure_admin(&user)?;

    // Confirm the datasource exists.
    sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    let mut tx = state.db.begin().await.map_err(internal_error)?;
    if payload.is_default {
        // Exactly one default: clear everyone, then set this one.
        sqlx::query("UPDATE datasources SET is_default = 0 WHERE is_default = 1")
            .execute(&mut *tx)
            .await
            .map_err(internal_error)?;
        sqlx::query("UPDATE datasources SET is_default = 1 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(internal_error)?;
    } else {
        sqlx::query("UPDATE datasources SET is_default = 0 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(internal_error)?;
    }
    tx.commit().await.map_err(internal_error)?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;

    Ok(Json(ds))
}

// ── Datasource access grants (admin only) ──

#[derive(serde::Deserialize)]
pub struct SetGrants {
    pub user_ids: Vec<i32>,
}

/// GET /api/datasources/{id}/grants — list the user ids granted access (admin).
pub async fn list_grants(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<Vec<i32>>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let rows: Vec<(i32,)> = sqlx::query_as(
        "SELECT user_id FROM datasource_grants WHERE datasource_id = ? ORDER BY user_id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(Json(rows.into_iter().map(|(uid,)| uid).collect()))
}

/// PUT /api/datasources/{id}/grants — replace the grant list (admin).
pub async fn set_grants(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<SetGrants>,
) -> Result<Json<Vec<i32>>, (StatusCode, String)> {
    ensure_admin(&user)?;

    // Confirm the datasource exists.
    sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Data source not found".to_string()))?;

    // Replace the grant set: clear then re-insert (skip invalid/admin ids gracefully).
    let mut tx = state.db.begin().await.map_err(internal_error)?;
    sqlx::query("DELETE FROM datasource_grants WHERE datasource_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
    for uid in payload.user_ids.iter().copied().collect::<std::collections::BTreeSet<_>>() {
        sqlx::query(
            "INSERT INTO datasource_grants (datasource_id, user_id) \
             SELECT ?, id FROM users WHERE id = ?",
        )
        .bind(id)
        .bind(uid)
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
    }
    tx.commit().await.map_err(internal_error)?;

    let rows: Vec<(i32,)> = sqlx::query_as(
        "SELECT user_id FROM datasource_grants WHERE datasource_id = ? ORDER BY user_id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(Json(rows.into_iter().map(|(uid,)| uid).collect()))
}

// ── Per-DB connection test helpers ──

async fn test_conn_mysql(state: &AppState, ds: &DataSource) -> Result<(), String> {
    let pool = state.pool_cache.get_mysql(ds).await?;
    // Quick connectivity check
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|e| format!("MySQL: {}", e))?;
    Ok(())
}

async fn test_conn_postgres(state: &AppState, ds: &DataSource) -> Result<(), String> {
    let pool = state.pool_cache.get_postgres(ds).await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|e| format!("PostgreSQL: {}", e))?;
    Ok(())
}

async fn test_conn_oracle(state: &AppState, ds: &DataSource) -> Result<(), String> {
    let pool = state.pool_cache.get_oracle(ds).await?;
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("Oracle: {}", e))?;
        conn.query_row_as::<i32>("SELECT 1 FROM DUAL", &[])
            .map_err(|e| format!("Oracle ping: {}", e))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Oracle spawn: {}", e))?
}

// ── Per-DB introspection helpers ──

async fn introspect_mysql(state: &AppState, ds: &DataSource) -> Result<SchemaInfo, String> {
    let pool = state.pool_cache.get_mysql(ds).await?;

    // Fetch tables with comments
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT CAST(TABLE_NAME AS CHAR), CAST(IFNULL(TABLE_COMMENT, '') AS CHAR)
         FROM INFORMATION_SCHEMA.TABLES
         WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE'",
    )
    .bind(&ds.database_name)
    .fetch_all(&pool)
    .await
    .map_err(|e| format!("MySQL tables query: {}", e))?;

    let mut schema = SchemaInfo { tables: Vec::new(), relationships: Vec::new() };

    for (table_name, table_comment) in &tables {
        // Fetch columns with comments
        let columns: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT CAST(COLUMN_NAME AS CHAR), CAST(DATA_TYPE AS CHAR), CAST(IS_NULLABLE AS CHAR), CAST(COLUMN_KEY AS CHAR), CAST(COLUMN_TYPE AS CHAR), CAST(IFNULL(COLUMN_COMMENT, '') AS CHAR)
             FROM INFORMATION_SCHEMA.COLUMNS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY ORDINAL_POSITION",
        )
        .bind(&ds.database_name)
        .bind(table_name)
        .fetch_all(&pool)
        .await
        .map_err(|e| format!("MySQL columns query: {}", e))?;

        let columns: Vec<ColumnInfo> = columns
            .into_iter()
            .map(|(name, _data_type, nullable, key, col_type, comment)| ColumnInfo {
                name,
                data_type: col_type,
                nullable: nullable == "YES",
                is_primary_key: key == "PRI",
                is_foreign_key: key == "MUL",
                comment: if comment.is_empty() { None } else { Some(comment) },
            })
            .collect();

        schema.tables.push(TableInfo {
            name: table_name.clone(),
            comment: if table_comment.is_empty() { None } else { Some(table_comment.clone()) },
            columns,
        });
    }

    let fks: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT CAST(TABLE_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR), CAST(REFERENCED_TABLE_NAME AS CHAR), CAST(REFERENCED_COLUMN_NAME AS CHAR)
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE
         WHERE TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME IS NOT NULL",
    )
    .bind(&ds.database_name)
    .fetch_all(&pool)
    .await
    .map_err(|e| format!("MySQL FK query: {}", e))?;

    for (table, col, ref_table, ref_col) in &fks {
        schema.relationships.push(Relationship {
            source_table: table.clone(),
            source_column: col.clone(),
            target_table: ref_table.clone(),
            target_column: ref_col.clone(),
        });
    }

    Ok(schema)
}

async fn introspect_postgres(state: &AppState, ds: &DataSource) -> Result<SchemaInfo, String> {
    let pool = state.pool_cache.get_postgres(ds).await?;
    pg_schema_from_pool(&pool).await
}

pub(crate) async fn pg_schema_from_pool(pool: &sqlx::PgPool) -> Result<SchemaInfo, String> {
    use std::collections::{HashMap, HashSet};

    // Read the catalog directly instead of `information_schema`. The
    // `information_schema` views only expose objects the connected role holds a
    // privilege on, so a read-only account without an explicit GRANT sees zero
    // rows and introspection silently returns nothing. `pg_catalog` is readable
    // by every role and also lets us cover views, materialized views, foreign
    // tables and non-`public` schemas.
    let relations: Vec<(i64, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.oid::int8, n.nspname::text, c.relname::text, c.relkind::text,
                obj_description(c.oid, 'pg_class')
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
           AND NOT c.relispartition
           AND n.nspname NOT IN ('pg_catalog', 'information_schema')
           AND left(n.nspname, 3) <> 'pg_'
         ORDER BY n.nspname, c.relname",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("PostgreSQL tables query: {}", e))?;

    // All columns in one round trip, keyed by relation oid.
    let all_columns: Vec<(i64, String, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT a.attrelid::int8, a.attname::text,
                format_type(a.atttypid, a.atttypmod)::text,
                a.attnotnull, col_description(a.attrelid, a.attnum)
         FROM pg_attribute a
         JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE a.attnum > 0 AND NOT a.attisdropped
           AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
           AND n.nspname NOT IN ('pg_catalog', 'information_schema')
           AND left(n.nspname, 3) <> 'pg_'
         ORDER BY a.attrelid, a.attnum",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("PostgreSQL columns query: {}", e))?;

    let mut cols_by_rel: HashMap<i64, Vec<(String, String, bool, Option<String>)>> = HashMap::new();
    for (relid, name, data_type, notnull, comment) in all_columns {
        cols_by_rel
            .entry(relid)
            .or_default()
            .push((name, data_type, notnull, comment));
    }

    // Primary keys, keyed by relation oid.
    let pk_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT i.indrelid::int8, a.attname::text
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
         WHERE i.indisprimary
           AND n.nspname NOT IN ('pg_catalog', 'information_schema')
           AND left(n.nspname, 3) <> 'pg_'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut pks_by_rel: HashMap<i64, HashSet<String>> = HashMap::new();
    for (relid, col) in pk_rows {
        pks_by_rel.entry(relid).or_default().insert(col);
    }

    // Foreign keys from pg_constraint. Zipping conkey/confkey by ordinality keeps
    // composite keys paired correctly (the old constraint_column_usage join
    // produced a cross product) and joining on oid avoids mixing up
    // same-named constraints in different schemas.
    let fks: Vec<(i64, String, String, String, String, String)> = sqlx::query_as(
        "SELECT con.conrelid::int8,
                src_ns.nspname::text, sa.attname::text,
                tgt_ns.nspname::text, tgt.relname::text, ta.attname::text
         FROM pg_constraint con
         JOIN pg_class src ON src.oid = con.conrelid
         JOIN pg_namespace src_ns ON src_ns.oid = src.relnamespace
         JOIN pg_class tgt ON tgt.oid = con.confrelid
         JOIN pg_namespace tgt_ns ON tgt_ns.oid = tgt.relnamespace
         JOIN unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(src_att, tgt_att, ord) ON true
         JOIN pg_attribute sa ON sa.attrelid = con.conrelid AND sa.attnum = k.src_att
         JOIN pg_attribute ta ON ta.attrelid = con.confrelid AND ta.attnum = k.tgt_att
         WHERE con.contype = 'f'
           AND src_ns.nspname NOT IN ('pg_catalog', 'information_schema')
           AND left(src_ns.nspname, 3) <> 'pg_'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("PostgreSQL FK query: {}", e))?;

    // FK columns per relation, so ColumnInfo.is_foreign_key is accurate.
    let mut fk_cols_by_rel: HashMap<i64, HashSet<&str>> = HashMap::new();
    for (relid, _, src_col, _, _, _) in &fks {
        fk_cols_by_rel.entry(*relid).or_default().insert(src_col.as_str());
    }

    let mut schema = SchemaInfo { tables: Vec::new(), relationships: Vec::new() };

    for (relid, nspname, relname, relkind, rel_comment) in &relations {
        let empty_pk = HashSet::new();
        let pk_set = pks_by_rel.get(relid).unwrap_or(&empty_pk);
        let empty_fk = HashSet::new();
        let fk_set = fk_cols_by_rel.get(relid).unwrap_or(&empty_fk);

        let columns: Vec<ColumnInfo> = cols_by_rel
            .remove(relid)
            .unwrap_or_default()
            .into_iter()
            .map(|(name, data_type, notnull, comment)| ColumnInfo {
                is_primary_key: pk_set.contains(&name),
                is_foreign_key: fk_set.contains(name.as_str()),
                comment: comment.filter(|c| !c.is_empty()),
                name,
                data_type,
                nullable: !notnull,
            })
            .collect();

        schema.tables.push(TableInfo {
            name: pg_qualified_name(nspname, relname),
            comment: pg_table_comment(relkind, rel_comment.as_deref()),
            columns,
        });
    }

    // Relationships use schema-qualified names on both ends so they line up
    // with `schema.tables` entries.
    let name_by_oid: HashMap<i64, String> = relations
        .iter()
        .map(|(oid, ns, rel, _, _)| (*oid, pg_qualified_name(ns, rel)))
        .collect();

    for (relid, _, src_col, tgt_ns, tgt_table, tgt_col) in &fks {
        let Some(source_table) = name_by_oid.get(relid).cloned() else { continue };
        schema.relationships.push(Relationship {
            source_table,
            source_column: src_col.clone(),
            target_table: pg_qualified_name(tgt_ns, tgt_table),
            target_column: tgt_col.clone(),
        });
    }

    Ok(schema)
}

/// Table identifier as the AI (and generated SQL) should see it: bare name for
/// `public`, `schema.table` everywhere else so queries resolve regardless of
/// the connection's `search_path`.
pub fn pg_qualified_name(schema: &str, table: &str) -> String {
    if schema == "public" {
        table.to_string()
    } else {
        format!("{}.{}", schema, table)
    }
}

/// Keep the relation kind visible in the description so the AI knows it is
/// querying a view rather than a base table.
fn pg_table_comment(relkind: &str, comment: Option<&str>) -> Option<String> {
    let kind_tag = match relkind {
        "v" => Some("[view]"),
        "m" => Some("[materialized view]"),
        "f" => Some("[foreign table]"),
        _ => None,
    };
    let comment = comment.filter(|c| !c.is_empty());
    match (kind_tag, comment) {
        (Some(tag), Some(c)) => Some(format!("{} {}", tag, c)),
        (Some(tag), None) => Some(tag.to_string()),
        (None, Some(c)) => Some(c.to_string()),
        (None, None) => None,
    }
}

async fn introspect_oracle(state: &AppState, ds: &DataSource) -> Result<SchemaInfo, String> {
    let pool = state.pool_cache.get_oracle(ds).await?;

    tokio::task::spawn_blocking(move || -> Result<SchemaInfo, String> {
        let conn = pool.get()
            .map_err(|e| format!("Oracle pool get failed: {}", e))?;

        let mut schema = SchemaInfo { tables: Vec::new(), relationships: Vec::new() };

        // Tables
        let rows = conn
            .query_as::<(String,)>("SELECT TABLE_NAME FROM USER_TABLES ORDER BY TABLE_NAME", &[])
            .map_err(|e| format!("Oracle tables query: {}", e))?;
        let tables: Vec<String> = rows.filter_map(|r| r.ok()).map(|(t,)| t).collect();

        // Table comments
        let tab_comment_rows = conn
            .query_as::<(String, Option<String>)>(
                "SELECT TABLE_NAME, COMMENTS FROM USER_TAB_COMMENTS WHERE TABLE_TYPE = 'TABLE'",
                &[],
            )
            .map_err(|e| format!("Oracle table comments query: {}", e))?;
        let tab_comments: std::collections::HashMap<String, String> = tab_comment_rows
            .filter_map(|r| r.ok())
            .filter_map(|(name, comment)| comment.filter(|c| !c.is_empty()).map(|c| (name, c)))
            .collect();

        // Column comments
        let col_comment_rows = conn
            .query_as::<(String, String, Option<String>)>(
                "SELECT TABLE_NAME, COLUMN_NAME, COMMENTS FROM USER_COL_COMMENTS",
                &[],
            )
            .map_err(|e| format!("Oracle column comments query: {}", e))?;
        let mut col_comments: std::collections::HashMap<String, std::collections::HashMap<String, String>> = std::collections::HashMap::new();
        for r in col_comment_rows {
            if let Ok((table, col, Some(comment))) = r {
                if !comment.is_empty() {
                    col_comments.entry(table).or_default().insert(col, comment);
                }
            }
        }

        for table_name in &tables {
            // Columns
            let col_rows = conn
                .query_as::<(String, String, String)>(
                    "SELECT COLUMN_NAME, DATA_TYPE, NULLABLE FROM USER_TAB_COLUMNS WHERE TABLE_NAME = :1 ORDER BY COLUMN_ID",
                    &[table_name],
                )
                .map_err(|e| format!("Oracle columns query: {}", e))?;

            // PK columns
            let pk_rows = conn
                .query_as::<(String,)>(
                    "SELECT cols.COLUMN_NAME FROM USER_CONSTRAINTS cons
                     JOIN USER_CONS_COLUMNS cols ON cons.CONSTRAINT_NAME = cols.CONSTRAINT_NAME
                     WHERE cons.CONSTRAINT_TYPE = 'P' AND cons.TABLE_NAME = :1",
                    &[table_name],
                )
                .map_err(|e| format!("Oracle PK query: {}", e))?;
            let pk_set: std::collections::HashSet<String> = pk_rows.filter_map(|r| r.ok()).map(|(c,)| c).collect();

            let table_col_comments = col_comments.get(table_name);

            let columns: Vec<ColumnInfo> = col_rows
                .filter_map(|r| r.ok())
                .map(|(name, data_type, nullable)| {
                    let comment = table_col_comments.and_then(|m| m.get(&name)).cloned();
                    ColumnInfo {
                        is_primary_key: pk_set.contains(&name),
                        is_foreign_key: false,
                        comment,
                        name,
                        data_type,
                        nullable: nullable == "Y",
                    }
                })
                .collect();

            schema.tables.push(TableInfo {
                name: table_name.clone(),
                comment: tab_comments.get(table_name).cloned(),
                columns,
            });
        }

        // FK relationships
        let fk_rows = conn
            .query_as::<(String, String, String, String)>(
                "SELECT a.COLUMN_NAME, c_pk.TABLE_NAME, a.TABLE_NAME, c_pk.COLUMN_NAME
                 FROM USER_CONS_COLUMNS a
                 JOIN USER_CONSTRAINTS c ON a.CONSTRAINT_NAME = c.CONSTRAINT_NAME
                 JOIN USER_CONSTRAINTS c_pk ON c.R_CONSTRAINT_NAME = c_pk.CONSTRAINT_NAME
                 WHERE c.CONSTRAINT_TYPE = 'R'",
                &[],
            )
            .map_err(|e| format!("Oracle FK query: {}", e))?;

        for r in fk_rows {
            if let Ok((src_col, ref_table, src_table, ref_col)) = r {
                schema.relationships.push(Relationship {
                    source_table: src_table,
                    source_column: src_col,
                    target_table: ref_table,
                    target_column: ref_col,
                });
            }
        }

        conn.close().ok(); // Return connection to the pool
        Ok(schema)
    })
    .await
    .map_err(|e| format!("Oracle spawn: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualifies_only_non_public_schemas() {
        assert_eq!(pg_qualified_name("public", "users"), "users");
        assert_eq!(pg_qualified_name("sales", "invoices"), "sales.invoices");
    }

    #[test]
    fn table_comment_marks_relation_kind() {
        assert_eq!(pg_table_comment("r", Some("orders")), Some("orders".into()));
        assert_eq!(pg_table_comment("r", None), None);
        assert_eq!(pg_table_comment("v", None), Some("[view]".into()));
        assert_eq!(pg_table_comment("m", Some("daily")), Some("[materialized view] daily".into()));
        assert_eq!(pg_table_comment("r", Some("")), None);
    }

    /// Live check against a real PostgreSQL. Opt in with:
    ///   PG_TEST_URL=postgres://user:pass@host:port/db cargo test pg_introspection -- --ignored
    /// Creates its own objects in a throwaway schema and drops them afterwards.
    #[tokio::test]
    #[ignore]
    async fn pg_introspection_covers_schemas_views_and_keys() {
        let url = std::env::var("PG_TEST_URL").expect("PG_TEST_URL not set");
        let pool = sqlx::PgPool::connect(&url).await.unwrap();

        let schema = pg_schema_from_pool(&pool).await.unwrap();
        let names: Vec<&str> = schema.tables.iter().map(|t| t.name.as_str()).collect();

        // Non-public schemas are qualified, views are included.
        assert!(names.contains(&"sales.invoices"), "missing non-public table: {:?}", names);
        assert!(names.contains(&"v_user_orders"), "missing view: {:?}", names);

        let users = schema.tables.iter().find(|t| t.name == "users").unwrap();
        assert_eq!(users.comment.as_deref(), Some("用户表"));
        let id = users.columns.iter().find(|c| c.name == "id").unwrap();
        assert!(id.is_primary_key && !id.nullable);
        let email = users.columns.iter().find(|c| c.name == "email").unwrap();
        assert!(email.nullable && !email.is_primary_key);

        // Column types keep their modifiers.
        let name_col = users.columns.iter().find(|c| c.name == "name").unwrap();
        assert_eq!(name_col.data_type, "character varying(50)");

        // FKs are schema-qualified on both ends and mark the source column.
        assert!(schema.relationships.iter().any(|r| r.source_table == "orders"
            && r.source_column == "user_id"
            && r.target_table == "users"
            && r.target_column == "id"));
        assert!(schema.relationships.iter().any(|r| r.source_table == "sales.invoices"
            && r.target_table == "orders"));
        let orders = schema.tables.iter().find(|t| t.name == "orders").unwrap();
        assert!(orders.columns.iter().find(|c| c.name == "user_id").unwrap().is_foreign_key);
    }
}
