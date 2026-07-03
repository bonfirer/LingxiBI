//! CRUD for user-curated report themes.
//!
//! A theme bundles a reusable style spec (free-form guidance) plus an optional
//! sample HTML template captured from a report the user liked. Themes are then
//! selectable when generating a new dashboard (see `reports::render`).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::{ensure_owner, internal_error};
use crate::AppState;

/// Max characters of sample HTML we keep — bounds prompt size at generation time.
const MAX_SAMPLE_HTML: usize = 60_000;

/// List all themes (without the heavy `sample_html` payload). Scoped to owner.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<ReportTheme>>, (StatusCode, String)> {
    let themes = sqlx::query_as::<_, ReportTheme>(
        "SELECT id, name, description, style_prompt, NULL AS sample_html, emoji, \
         source_report_id, owner_user_id, created_at, updated_at \
         FROM report_themes WHERE (owner_user_id = ? OR ? = 1) ORDER BY updated_at DESC",
    )
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(Json(themes))
}

/// Create a theme. If `source_report_id` is given and no explicit `sample_html`,
/// capture that report's current HTML as the theme's reference template.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateReportThemeRequest>,
) -> Result<(StatusCode, Json<ReportTheme>), (StatusCode, String)> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Theme name is required".to_string()));
    }

    // Resolve the sample HTML: explicit value wins, else capture from the report.
    // When capturing from a report, verify the caller owns that report.
    let mut sample_html = payload.sample_html;
    if sample_html.is_none() {
        if let Some(rid) = payload.source_report_id {
            let row: Option<(Option<i32>, Option<String>)> =
                sqlx::query_as("SELECT owner_user_id, html_content FROM reports WHERE id = ?")
                    .bind(rid)
                    .fetch_optional(&state.db)
                    .await
                    .map_err(internal_error)?;
            match row {
                Some((owner, html)) => {
                    ensure_owner(&user, owner)?;
                    sample_html = html;
                }
                None => return Err((StatusCode::NOT_FOUND, "Source report not found".to_string())),
            }
        }
    }
    // Bound the stored template so it can never blow up the generation prompt.
    if let Some(html) = &mut sample_html {
        if html.len() > MAX_SAMPLE_HTML {
            *html = html.chars().take(MAX_SAMPLE_HTML).collect();
        }
    }

    let result = sqlx::query(
        "INSERT INTO report_themes (name, description, style_prompt, sample_html, emoji, source_report_id, owner_user_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(name)
    .bind(payload.description.unwrap_or_default())
    .bind(&payload.style_prompt)
    .bind(&sample_html)
    .bind(payload.emoji.unwrap_or_else(|| "🎨".to_string()))
    .bind(payload.source_report_id)
    .bind(user.id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let theme = sqlx::query_as::<_, ReportTheme>(
        "SELECT id, name, description, style_prompt, NULL AS sample_html, emoji, \
         source_report_id, owner_user_id, created_at, updated_at FROM report_themes WHERE id = ?",
    )
    .bind(result.last_insert_id() as i32)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;

    Ok((StatusCode::CREATED, Json(theme)))
}

/// Delete a theme (owner or admin only).
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    let owner: Option<(Option<i32>,)> =
        sqlx::query_as("SELECT owner_user_id FROM report_themes WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(internal_error)?;
    match owner {
        Some((owner_user_id,)) => ensure_owner(&user, owner_user_id)?,
        None => return Err((StatusCode::NOT_FOUND, "Theme not found".to_string())),
    }

    sqlx::query("DELETE FROM report_themes WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}
