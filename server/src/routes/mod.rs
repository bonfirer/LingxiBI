pub mod datasources;
pub mod knowledge_graph;
pub mod conversations;
pub mod chat;
pub mod query;
pub mod reports;
pub mod report_groups;
pub mod report_datasources;
pub mod report_themes;
pub mod metric_groups;
pub mod metric_pools;
pub mod llm_config;
pub mod knowledge_base;
pub mod auth;
pub mod ai_logs;
pub mod ai_examples;
pub mod achievements;
pub mod snapshots;
pub mod alerts;
pub mod table_descriptions;
pub mod wishes;

use axum::http::StatusCode;
use crate::routes::auth::AuthUser;

/// Map any internal error to a generic HTTP 500, logging the real cause
/// server-side. Use this for unexpected failures (DB/driver/serialization) so
/// internal detail (SQL, schema, connection strings) never leaks to clients.
pub fn internal_error<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    tracing::error!("internal error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
    )
}

/// Ownership gate for per-user resources. Admins pass unconditionally; everyone
/// else must be the resource owner. Returns 404 (not 403) for non-owned rows so
/// we don't reveal that a resource with that id exists.
pub fn ensure_owner(
    user: &AuthUser,
    owner_user_id: Option<i32>,
) -> Result<(), (StatusCode, String)> {
    if user.is_admin || (owner_user_id.is_some() && owner_user_id == Some(user.id)) {
        Ok(())
    } else {
        Err((StatusCode::NOT_FOUND, "Not found".to_string()))
    }
}

/// Require admin privileges for a shared-infrastructure mutation.
pub fn ensure_admin(user: &AuthUser) -> Result<(), (StatusCode, String)> {
    if user.is_admin {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "Admin privileges required".to_string()))
    }
}
