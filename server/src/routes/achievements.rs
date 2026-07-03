use axum::{
    extract::State,
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;
use crate::routes::auth::AuthUser;
use crate::AppState;

#[derive(sqlx::FromRow, serde::Serialize)]
pub struct Achievement {
    pub id: i32,
    pub user_id: i32,
    pub achievement: String,
    pub unlocked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// List the current user's achievements.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<Achievement>>, (StatusCode, String)> {
    let achievements = sqlx::query_as::<_, Achievement>(
        "SELECT * FROM achievements WHERE user_id = ? ORDER BY unlocked_at DESC"
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    Ok(Json(achievements))
}

/// Try to unlock an achievement (idempotent — INSERT IGNORE).
pub async fn unlock(
    db: &sqlx::MySqlPool,
    user_id: i32,
    achievement: &str,
) {
    let _ = sqlx::query(
        "INSERT IGNORE INTO achievements (user_id, achievement) VALUES (?, ?)"
    )
    .bind(user_id)
    .bind(achievement)
    .execute(db)
    .await;
}

/// Check and unlock achievements based on current state.
pub async fn check_achievements(db: &sqlx::MySqlPool, user_id: i32) {
    // Count reports
    let report_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reports")
        .fetch_one(db).await.unwrap_or((0,));
    if report_count.0 >= 1 { unlock(db, user_id, "first_report").await; }
    if report_count.0 >= 5 { unlock(db, user_id, "report_five").await; }
    if report_count.0 >= 10 { unlock(db, user_id, "report_ten").await; }

    // Count published
    let published: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reports WHERE status = 'published'")
        .fetch_one(db).await.unwrap_or((0,));
    if published.0 >= 1 { unlock(db, user_id, "first_publish").await; }

    // Count shared
    let shared: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reports WHERE share_token IS NOT NULL")
        .fetch_one(db).await.unwrap_or((0,));
    if shared.0 >= 1 { unlock(db, user_id, "first_share").await; }

    // Count metrics
    let metrics: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM metric_pools")
        .fetch_one(db).await.unwrap_or((0,));
    if metrics.0 >= 5 { unlock(db, user_id, "metric_collector").await; }
    if metrics.0 >= 20 { unlock(db, user_id, "metric_master").await; }

    // Count knowledge
    let knowledge: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM knowledge_base")
        .fetch_one(db).await.unwrap_or((0,));
    if knowledge.0 >= 5 { unlock(db, user_id, "knowledge_seeker").await; }
    if knowledge.0 >= 20 { unlock(db, user_id, "knowledge_sage").await; }

    // Count training examples
    let examples: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_examples")
        .fetch_one(db).await.unwrap_or((0,));
    if examples.0 >= 5 { unlock(db, user_id, "ai_trainer").await; }
    if examples.0 >= 20 { unlock(db, user_id, "ai_master").await; }

    // Count conversations
    let convos: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
        .fetch_one(db).await.unwrap_or((0,));
    if convos.0 >= 10 { unlock(db, user_id, "chatterbox").await; }
    if convos.0 >= 50 { unlock(db, user_id, "data_explorer").await; }

    // Style variety
    let styles: (i64,) = sqlx::query_as("SELECT COUNT(DISTINCT style_key) FROM reports WHERE style_key IS NOT NULL")
        .fetch_one(db).await.unwrap_or((0,));
    if styles.0 >= 3 { unlock(db, user_id, "style_explorer").await; }
    if styles.0 >= 8 { unlock(db, user_id, "fashionista").await; }
}
