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

pub async fn get_graph(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(ds_id): Path<i32>,
) -> Result<Json<KnowledgeGraph>, (StatusCode, String)> {
    crate::routes::datasources::ensure_access(&state, ds_id, &user).await?;
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT graph_data FROM knowledge_graphs WHERE datasource_id = ?")
            .bind(ds_id)
            .fetch_optional(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;

    match row {
        Some((data,)) => {
            let graph: KnowledgeGraph = serde_json::from_value(data)
                .map_err(crate::routes::internal_error)?;
            Ok(Json(graph))
        }
        None => Err((StatusCode::NOT_FOUND, "Knowledge graph not found. Run introspection first.".to_string())),
    }
}

pub async fn refresh_graph(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(ds_id): Path<i32>,
) -> Result<Json<KnowledgeGraph>, (StatusCode, String)> {
    ensure_admin(&user)?;
    // Get schema first
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT schema_data FROM `schemas` WHERE datasource_id = ?")
            .bind(ds_id)
            .fetch_optional(&state.db)
            .await
            .map_err(crate::routes::internal_error)?;

    let schema_data = row.ok_or((StatusCode::NOT_FOUND, "Schema not found. Run introspection first.".to_string()))?;

    let schema: SchemaInfo = serde_json::from_value(schema_data.0)
        .map_err(crate::routes::internal_error)?;

    // Build knowledge graph from schema
    let nodes: Vec<GraphNode> = schema
        .tables
        .iter()
        .map(|t| GraphNode {
            id: t.name.clone(),
            label: t.name.clone(),
            columns: t.columns.clone(),
        })
        .collect();

    let edges: Vec<GraphEdge> = schema
        .relationships
        .iter()
        .map(|r| GraphEdge {
            source: r.source_table.clone(),
            target: r.target_table.clone(),
            r#type: "FK".to_string(),
            on: format!("{}.{}", r.source_column, r.target_column),
        })
        .collect();

    let graph = KnowledgeGraph { nodes, edges };
    let graph_json = serde_json::to_value(&graph)
        .map_err(crate::routes::internal_error)?;

    sqlx::query(
        "INSERT INTO knowledge_graphs (datasource_id, graph_data) VALUES (?, ?)
         ON DUPLICATE KEY UPDATE graph_data = VALUES(graph_data), generated_at = CURRENT_TIMESTAMP",
    )
    .bind(ds_id)
    .bind(&graph_json)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(graph))
}
