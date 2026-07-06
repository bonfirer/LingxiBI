use axum::{
    extract::State,
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::ensure_admin;
use crate::AppState;

pub async fn get_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "LLM config not initialized".to_string()))?;

    Ok(Json(mask_config(config)))
}

pub async fn update_config(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<UpdateLLMConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let existing = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "LLM config not initialized".to_string()))?;

    sqlx::query(
        "UPDATE llm_config SET provider=?, base_url=?, api_key=?, model=?, max_tokens=?, temperature=?, updated_at=CURRENT_TIMESTAMP WHERE id=1",
    )
    .bind(payload.provider.as_deref().unwrap_or(&existing.provider))
    .bind(payload.base_url.as_deref().unwrap_or(&existing.base_url))
    // Treat empty/missing api_key as "keep existing" — the GET endpoint returns a
    // masked key, so the client only sends a real key when the user actually changes it.
    // Stored encrypted at rest; `encrypt` is idempotent on the existing ciphertext.
    .bind(match payload.api_key.as_deref() {
        Some(k) if !k.is_empty() => crate::crypto::encrypt(k),
        _ => existing.api_key.clone(),
    })
    .bind(payload.model.as_deref().unwrap_or(&existing.model))
    .bind(payload.max_tokens.unwrap_or(existing.max_tokens))
    .bind(payload.temperature.unwrap_or(existing.temperature))
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let config = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(mask_config(config)))
}

/// Mask the API key, keeping only a hint of the last 4 characters.
/// Returns a JSON value with the masked key and an `api_key_set` flag.
fn mask_config(config: LLMConfig) -> serde_json::Value {
    // Decrypt so the "last 4 chars" hint reflects the real key, not ciphertext.
    let key = crate::crypto::decrypt(&config.api_key);
    let key = &key;
    let masked = if key.is_empty() {
        String::new()
    } else if key.len() <= 4 {
        "••••".to_string()
    } else {
        format!("••••••••{}", &key[key.len() - 4..])
    };
    serde_json::json!({
        "id": config.id,
        "provider": config.provider,
        "base_url": config.base_url,
        "api_key": masked,
        "api_key_set": !key.is_empty(),
        "model": config.model,
        "max_tokens": config.max_tokens,
        "temperature": config.temperature,
    })
}

pub async fn test_connection(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "LLM config not initialized".to_string()))?;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/models", config.base_url.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", crate::crypto::decrypt(&config.api_key)))
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            Ok(Json(serde_json::json!({
                "status": "connected",
                "message": "LLM provider is reachable"
            })))
        }
        Ok(resp) => {
            Ok(Json(serde_json::json!({
                "status": "error",
                "message": format!("Provider returned status: {}", resp.status())
            })))
        }
        Err(e) => {
            Ok(Json(serde_json::json!({
                "status": "error",
                "message": format!("Connection failed: {}", e)
            })))
        }
    }
}
