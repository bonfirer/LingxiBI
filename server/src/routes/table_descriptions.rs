use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::ensure_admin;
use crate::AppState;

/// List all table descriptions for a datasource.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(ds_id): Path<i32>,
) -> Result<Json<Vec<TableDescription>>, (StatusCode, String)> {
    crate::routes::datasources::ensure_access(&state, ds_id, &user).await?;
    let items = sqlx::query_as::<_, TableDescription>(
        "SELECT * FROM table_descriptions WHERE datasource_id = ? ORDER BY table_name",
    )
    .bind(ds_id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(items))
}

/// Create or update a table description (upsert by datasource_id + table_name).
/// An empty description deletes the entry.
pub async fn upsert(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(ds_id): Path<i32>,
    Json(payload): Json<UpsertTableDescription>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    if payload.description.trim().is_empty() {
        // Empty description → remove any existing entry
        sqlx::query("DELETE FROM table_descriptions WHERE datasource_id = ? AND table_name = ?")
            .bind(ds_id)
            .bind(&payload.table_name)
            .execute(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
        return Ok(Json(serde_json::json!({ "status": "deleted" })));
    }

    sqlx::query(
        "INSERT INTO table_descriptions (datasource_id, table_name, description)
         VALUES (?, ?, ?)
         ON DUPLICATE KEY UPDATE description = VALUES(description), updated_at = CURRENT_TIMESTAMP",
    )
    .bind(ds_id)
    .bind(&payload.table_name)
    .bind(payload.description.trim())
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ── Column Descriptions ──

/// List all column descriptions for a datasource.
pub async fn list_columns(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(ds_id): Path<i32>,
) -> Result<Json<Vec<ColumnDescription>>, (StatusCode, String)> {
    crate::routes::datasources::ensure_access(&state, ds_id, &user).await?;
    let items = sqlx::query_as::<_, ColumnDescription>(
        "SELECT * FROM column_descriptions WHERE datasource_id = ? ORDER BY table_name, column_name",
    )
    .bind(ds_id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(items))
}

/// Create or update a column description (upsert by datasource_id + table_name + column_name).
/// An empty description deletes the entry.
pub async fn upsert_column(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(ds_id): Path<i32>,
    Json(payload): Json<UpsertColumnDescription>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    if payload.description.trim().is_empty() {
        sqlx::query("DELETE FROM column_descriptions WHERE datasource_id = ? AND table_name = ? AND column_name = ?")
            .bind(ds_id)
            .bind(&payload.table_name)
            .bind(&payload.column_name)
            .execute(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;
        return Ok(Json(serde_json::json!({ "status": "deleted" })));
    }

    sqlx::query(
        "INSERT INTO column_descriptions (datasource_id, table_name, column_name, description)
         VALUES (?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE description = VALUES(description), updated_at = CURRENT_TIMESTAMP",
    )
    .bind(ds_id)
    .bind(&payload.table_name)
    .bind(&payload.column_name)
    .bind(payload.description.trim())
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(serde_json::json!({ "status": "ok" })))
}
