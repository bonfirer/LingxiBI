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

/// List knowledge entries the caller may see.
///
/// Scoped by datasource grants: these entries describe table semantics and
/// business rules, so an unscoped listing handed every member the documented
/// meaning of every connected database.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<KnowledgeEntry>>, (StatusCode, String)> {
    let allowed = crate::routes::datasources::accessible_ids(&state, &user).await?;
    if allowed.is_empty() {
        return Ok(Json(vec![]));
    }
    let placeholders = std::iter::repeat("?")
        .take(allowed.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT * FROM knowledge_base WHERE datasource_id IN ({placeholders})
         ORDER BY datasource_id, category, id"
    );
    let mut q = sqlx::query_as::<_, KnowledgeEntry>(&sql);
    for id in &allowed {
        q = q.bind(id);
    }
    let entries = q
        .fetch_all(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(entries))
}

/// List knowledge entries for a specific datasource the caller has access to.
pub async fn list_by_datasource(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(ds_id): Path<i32>,
) -> Result<Json<Vec<KnowledgeEntry>>, (StatusCode, String)> {
    crate::routes::datasources::ensure_access(&state, ds_id, &user).await?;
    let entries = sqlx::query_as::<_, KnowledgeEntry>(
        "SELECT * FROM knowledge_base WHERE datasource_id = ? ORDER BY category, id"
    )
    .bind(ds_id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(entries))
}

/// Create a new knowledge entry.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateKnowledgeEntry>,
) -> Result<(StatusCode, Json<KnowledgeEntry>), (StatusCode, String)> {
    ensure_admin(&user)?;
    let category = payload.category.as_deref().unwrap_or("relation");
    let source = payload.source.as_deref().unwrap_or("manual");
    let confidence = payload.confidence.as_deref().unwrap_or("high");

    let result = sqlx::query(
        "INSERT INTO knowledge_base (datasource_id, category, title, content, source, confidence) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(payload.datasource_id)
    .bind(category)
    .bind(&payload.title)
    .bind(&payload.content)
    .bind(source)
    .bind(confidence)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let entry = sqlx::query_as::<_, KnowledgeEntry>("SELECT * FROM knowledge_base WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(entry)))
}

/// Update a knowledge entry.
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateKnowledgeEntry>,
) -> Result<Json<KnowledgeEntry>, (StatusCode, String)> {
    ensure_admin(&user)?;
    if let Some(title) = &payload.title {
        sqlx::query("UPDATE knowledge_base SET title = ? WHERE id = ?")
            .bind(title).bind(id).execute(&state.db).await.ok();
    }
    if let Some(content) = &payload.content {
        sqlx::query("UPDATE knowledge_base SET content = ? WHERE id = ?")
            .bind(content).bind(id).execute(&state.db).await.ok();
    }
    if let Some(category) = &payload.category {
        sqlx::query("UPDATE knowledge_base SET category = ? WHERE id = ?")
            .bind(category).bind(id).execute(&state.db).await.ok();
    }
    if let Some(confidence) = &payload.confidence {
        sqlx::query("UPDATE knowledge_base SET confidence = ? WHERE id = ?")
            .bind(confidence).bind(id).execute(&state.db).await.ok();
    }

    let entry = sqlx::query_as::<_, KnowledgeEntry>("SELECT * FROM knowledge_base WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(entry))
}

/// Delete a knowledge entry.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    ensure_admin(&user)?;
    sqlx::query("DELETE FROM knowledge_base WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(StatusCode::NO_CONTENT)
}
