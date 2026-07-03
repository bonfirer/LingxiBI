//! HTTP handlers for the email alert module:
//! - SMTP configuration (get / update / test)
//! - Alert rule CRUD
//! - AI template generation
//! - Manual trigger + test send
//! - Alert logs

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use std::sync::Arc;

use crate::alert_engine;
use crate::email;
use crate::llm::{prompts, ChatMessage, LlmClient};
use crate::models::*;
use crate::routes::auth::AuthUser;
use crate::routes::{ensure_admin, ensure_owner, internal_error};
use crate::AppState;

/// Load an alert rule and verify the caller owns it (admins bypass).
async fn load_owned_rule(
    state: &AppState,
    id: i32,
    user: &AuthUser,
) -> Result<AlertRule, (StatusCode, String)> {
    let rule = sqlx::query_as::<_, AlertRule>("SELECT * FROM alert_rules WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Alert rule not found".to_string()))?;
    ensure_owner(user, rule.owner_user_id)?;
    Ok(rule)
}

/// Verify the caller owns metric `metric_id` (admins bypass). Used when a rule
/// references a metric — you can only alert on your own metrics.
async fn ensure_metric_owned(
    state: &AppState,
    metric_id: i32,
    user: &AuthUser,
) -> Result<(), (StatusCode, String)> {
    let m: Option<(Option<i32>,)> =
        sqlx::query_as("SELECT owner_user_id FROM metric_pools WHERE id = ?")
            .bind(metric_id)
            .fetch_optional(&state.db)
            .await
            .map_err(internal_error)?;
    match m {
        Some((owner,)) => ensure_owner(user, owner),
        None => Err((StatusCode::NOT_FOUND, "Metric pool not found".to_string())),
    }
}

// ── SMTP config ──

/// Get the SMTP config (password masked).
pub async fn get_smtp(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cfg = sqlx::query_as::<_, SmtpConfig>("SELECT * FROM smtp_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let value = match cfg {
        Some(c) => {
            let password_set = !c.password.is_empty();
            serde_json::json!({
                "id": c.id,
                "host": c.host,
                "port": c.port,
                "username": c.username,
                "password_set": password_set,
                "from_email": c.from_email,
                "from_name": c.from_name,
                "use_tls": c.use_tls,
                "enabled": c.enabled,
            })
        }
        None => serde_json::json!({
            "id": 1, "host": "", "port": 465, "username": "",
            "password_set": false, "from_email": "", "from_name": "LingxiBI",
            "use_tls": true, "enabled": false,
        }),
    };
    Ok(Json(value))
}

/// Update the SMTP config. Empty password preserves the existing one.
pub async fn update_smtp(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<UpdateSmtpConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let existing = sqlx::query_as::<_, SmtpConfig>("SELECT * FROM smtp_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let base = existing.unwrap_or(SmtpConfig {
        id: 1,
        host: String::new(),
        port: 465,
        username: String::new(),
        password: String::new(),
        from_email: String::new(),
        from_name: "LingxiBI".to_string(),
        use_tls: true,
        enabled: false,
        created_at: None,
        updated_at: None,
    });

    let host = payload.host.unwrap_or(base.host);
    let port = payload.port.unwrap_or(base.port);
    let username = payload.username.unwrap_or(base.username);
    // Only overwrite the password when a non-empty value is provided.
    let password = match payload.password {
        Some(p) if !p.is_empty() => p,
        _ => base.password,
    };
    let from_email = payload.from_email.unwrap_or(base.from_email);
    let from_name = payload.from_name.unwrap_or(base.from_name);
    let use_tls = payload.use_tls.unwrap_or(base.use_tls);
    let enabled = payload.enabled.unwrap_or(base.enabled);

    sqlx::query(
        "INSERT INTO smtp_config (id, host, port, username, password, from_email, from_name, use_tls, enabled)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE host=VALUES(host), port=VALUES(port), username=VALUES(username),
           password=VALUES(password), from_email=VALUES(from_email), from_name=VALUES(from_name),
           use_tls=VALUES(use_tls), enabled=VALUES(enabled), updated_at=CURRENT_TIMESTAMP",
    )
    .bind(&host)
    .bind(port)
    .bind(&username)
    .bind(&password)
    .bind(&from_email)
    .bind(&from_name)
    .bind(use_tls)
    .bind(enabled)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    get_smtp(State(state)).await
}

#[derive(Debug, serde::Deserialize)]
pub struct TestSmtpRequest {
    pub to: String,
}

/// Send a quick test email to verify SMTP credentials.
pub async fn test_smtp(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<TestSmtpRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let cfg = sqlx::query_as::<_, SmtpConfig>("SELECT * FROM smtp_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "SMTP not configured".to_string()))?;

    let body = "<div style=\"font-family:Arial,sans-serif;padding:16px;\"><h2>✅ SMTP 测试成功</h2><p>如果你收到这封邮件，说明 LingxiBI 的邮件预警通道已正常工作。</p></div>";

    email::send_email(&cfg, &[payload.to.clone()], "LingxiBI — SMTP 测试", body, vec![])
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": format!("Test email sent to {}", payload.to)
    })))
}

// ── Feishu config ──

/// Get the Feishu config (secret masked).
pub async fn get_feishu(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cfg = sqlx::query_as::<_, FeishuConfig>("SELECT * FROM feishu_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let value = match cfg {
        Some(c) => serde_json::json!({
            "id": c.id,
            "webhook_url": c.webhook_url,
            "secret_set": !c.secret.is_empty(),
            "enabled": c.enabled,
        }),
        None => serde_json::json!({
            "id": 1, "webhook_url": "", "secret_set": false, "enabled": false,
        }),
    };
    Ok(Json(value))
}

/// Update the Feishu config. Empty secret preserves the existing one.
pub async fn update_feishu(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<UpdateFeishuConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let existing = sqlx::query_as::<_, FeishuConfig>("SELECT * FROM feishu_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    let base_url = existing.as_ref().map(|c| c.webhook_url.clone()).unwrap_or_default();
    let base_secret = existing.as_ref().map(|c| c.secret.clone()).unwrap_or_default();
    let base_enabled = existing.as_ref().map(|c| c.enabled).unwrap_or(false);

    let webhook_url = payload.webhook_url.unwrap_or(base_url);
    // Only overwrite the secret when a non-empty value is provided.
    let secret = match payload.secret {
        Some(s) if !s.is_empty() => s,
        _ => base_secret,
    };
    let enabled = payload.enabled.unwrap_or(base_enabled);

    // SSRF guard: only allow official Feishu/Lark HTTPS webhook hosts to be saved.
    if !webhook_url.trim().is_empty() {
        crate::feishu::validate_webhook_url(&webhook_url)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    }

    sqlx::query(
        "INSERT INTO feishu_config (id, webhook_url, secret, enabled)
         VALUES (1, ?, ?, ?)
         ON DUPLICATE KEY UPDATE webhook_url=VALUES(webhook_url), secret=VALUES(secret),
           enabled=VALUES(enabled), updated_at=CURRENT_TIMESTAMP",
    )
    .bind(&webhook_url)
    .bind(&secret)
    .bind(enabled)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    get_feishu(State(state)).await
}

/// Send a test card to verify the Feishu webhook + signing secret.
pub async fn test_feishu(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ensure_admin(&user)?;
    let cfg = sqlx::query_as::<_, FeishuConfig>("SELECT * FROM feishu_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "Feishu not configured".to_string()))?;

    crate::feishu::send_text(&cfg, "测试连接成功")
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": "Test card sent to Feishu"
    })))
}

// ── Alert rule CRUD ──

const VALID_OPERATORS: &[&str] = &["gt", "gte", "lt", "lte", "eq", "ne"];
const VALID_SCHEDULES: &[&str] = &["hourly", "daily", "weekly", "monthly", "cron"];

pub async fn list_rules(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<AlertRule>>, (StatusCode, String)> {
    let rules = sqlx::query_as::<_, AlertRule>(
        "SELECT * FROM alert_rules WHERE (owner_user_id = ? OR ? = 1) ORDER BY created_at DESC",
    )
    .bind(user.id)
    .bind(user.is_admin as i32)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(Json(rules))
}

pub async fn get_rule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<AlertRule>, (StatusCode, String)> {
    let rule = load_owned_rule(&state, id, &user).await?;
    Ok(Json(rule))
}

pub async fn create_rule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<CreateAlertRule>,
) -> Result<(StatusCode, Json<AlertRule>), (StatusCode, String)> {
    // Validate metric exists and is owned by the caller (you can only alert on
    // your own metrics).
    ensure_metric_owned(&state, payload.metric_pool_id, &user).await?;

    if !VALID_OPERATORS.contains(&payload.operator.as_str()) {
        return Err((StatusCode::BAD_REQUEST, format!("Invalid operator. One of: {:?}", VALID_OPERATORS)));
    }
    if !VALID_SCHEDULES.contains(&payload.schedule_type.as_str()) {
        return Err((StatusCode::BAD_REQUEST, format!("Invalid schedule_type. One of: {:?}", VALID_SCHEDULES)));
    }
    if payload.schedule_type == "cron" && payload.cron_expr.is_none() {
        return Err((StatusCode::BAD_REQUEST, "cron_expr required for cron schedule".to_string()));
    }

    let recipients = serde_json::to_value(&payload.recipients).unwrap_or(serde_json::json!([]));
    let next_run = alert_engine::compute_next_run(&payload.schedule_type, payload.cron_expr.as_deref());

    let result = sqlx::query(
        "INSERT INTO alert_rules (name, metric_pool_id, condition_column, operator, threshold, recipients,
            schedule_type, cron_expr, enabled, subject_template, body_template, include_excel, notify_feishu, cooldown_minutes, next_run_at, owner_user_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&payload.name)
    .bind(payload.metric_pool_id)
    .bind(&payload.condition_column)
    .bind(&payload.operator)
    .bind(payload.threshold)
    .bind(&recipients)
    .bind(&payload.schedule_type)
    .bind(&payload.cron_expr)
    .bind(payload.subject_template.unwrap_or_default())
    .bind(&payload.body_template)
    .bind(payload.include_excel.unwrap_or(true))
    .bind(payload.notify_feishu.unwrap_or(false))
    .bind(payload.cooldown_minutes.unwrap_or(0))
    .bind(next_run)
    .bind(user.id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let rule = sqlx::query_as::<_, AlertRule>("SELECT * FROM alert_rules WHERE id = ?")
        .bind(result.last_insert_id() as i32)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok((StatusCode::CREATED, Json(rule)))
}

pub async fn update_rule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateAlertRule>,
) -> Result<Json<AlertRule>, (StatusCode, String)> {
    let existing = load_owned_rule(&state, id, &user).await?;

    let name = payload.name.unwrap_or(existing.name);
    let condition_column = payload.condition_column.or(existing.condition_column);
    let operator = payload.operator.unwrap_or(existing.operator);
    if !VALID_OPERATORS.contains(&operator.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "Invalid operator".to_string()));
    }
    let threshold = payload.threshold.unwrap_or(existing.threshold);
    let recipients = match payload.recipients {
        Some(r) => serde_json::to_value(&r).unwrap_or(serde_json::json!([])),
        None => existing.recipients,
    };
    let schedule_type = payload.schedule_type.unwrap_or(existing.schedule_type);
    if !VALID_SCHEDULES.contains(&schedule_type.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "Invalid schedule_type".to_string()));
    }
    let cron_expr = payload.cron_expr.or(existing.cron_expr);
    let enabled = payload.enabled.unwrap_or(existing.enabled);
    let subject_template = payload.subject_template.unwrap_or(existing.subject_template);
    let body_template = payload.body_template.or(existing.body_template);
    let include_excel = payload.include_excel.unwrap_or(existing.include_excel);
    let notify_feishu = payload.notify_feishu.unwrap_or(existing.notify_feishu);
    let cooldown_minutes = payload.cooldown_minutes.unwrap_or(existing.cooldown_minutes);

    let next_run = if enabled {
        alert_engine::compute_next_run(&schedule_type, cron_expr.as_deref())
    } else {
        None
    };

    sqlx::query(
        "UPDATE alert_rules SET name=?, condition_column=?, operator=?, threshold=?, recipients=?,
            schedule_type=?, cron_expr=?, enabled=?, subject_template=?, body_template=?,
            include_excel=?, notify_feishu=?, cooldown_minutes=?, next_run_at=?, updated_at=CURRENT_TIMESTAMP WHERE id=?",
    )
    .bind(&name)
    .bind(&condition_column)
    .bind(&operator)
    .bind(threshold)
    .bind(&recipients)
    .bind(&schedule_type)
    .bind(&cron_expr)
    .bind(enabled)
    .bind(&subject_template)
    .bind(&body_template)
    .bind(include_excel)
    .bind(notify_feishu)
    .bind(cooldown_minutes)
    .bind(next_run)
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(crate::routes::internal_error)?;

    let rule = sqlx::query_as::<_, AlertRule>("SELECT * FROM alert_rules WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(crate::routes::internal_error)?;

    Ok(Json(rule))
}

pub async fn delete_rule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    load_owned_rule(&state, id, &user).await?;
    sqlx::query("DELETE FROM alert_rules WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Manually evaluate + (if triggered) send an alert now.
pub async fn trigger_rule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rule = load_owned_rule(&state, id, &user).await?;

    match alert_engine::run_alert(&state, &rule, false).await {
        Ok(outcome) => {
            alert_engine::log_outcome(&state, &rule, &outcome, None).await;
            if outcome.status == "sent" {
                let _ = sqlx::query("UPDATE alert_rules SET last_triggered_at=CURRENT_TIMESTAMP WHERE id=?")
                    .bind(id)
                    .execute(&state.db)
                    .await;
            }
            Ok(Json(serde_json::json!({
                "triggered": outcome.triggered,
                "status": outcome.status,
                "evaluated_value": outcome.evaluated_value,
                "message": outcome.message,
            })))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e)),
    }
}

/// Send a test alert email immediately, ignoring the condition + cooldown.
pub async fn test_rule(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i32>,
    Json(payload): Json<TestAlertEmail>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut rule = load_owned_rule(&state, id, &user).await?;

    // Override recipients for this test, if provided.
    if let Some(recipients) = payload.recipients {
        if !recipients.is_empty() {
            rule.recipients = serde_json::to_value(&recipients).unwrap_or(rule.recipients);
        }
    }

    match alert_engine::run_alert(&state, &rule, true).await {
        Ok(outcome) => {
            alert_engine::log_outcome(&state, &rule, &outcome, None).await;
            Ok(Json(serde_json::json!({
                "status": outcome.status,
                "message": outcome.message,
            })))
        }
        Err(e) => {
            let outcome = alert_engine::AlertOutcome {
                triggered: false,
                evaluated_value: None,
                status: "failed".to_string(),
                message: "Test send failed".to_string(),
            };
            alert_engine::log_outcome(&state, &rule, &outcome, Some(&e)).await;
            Err((StatusCode::BAD_REQUEST, e))
        }
    }
}

// ── AI template generation ──

pub async fn generate_template(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<GenerateAlertTemplateRequest>,
) -> Result<Json<AlertTemplate>, (StatusCode, String)> {
    ensure_metric_owned(&state, payload.metric_pool_id, &user).await?;
    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(payload.metric_pool_id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Metric pool not found".to_string()))?;

    let llm_cfg = sqlx::query_as::<_, LLMConfig>("SELECT * FROM llm_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(crate::routes::internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "LLM not configured".to_string()))?;

    if llm_cfg.api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "LLM API key not configured".to_string()));
    }

    // Build metric context for the prompt.
    let mut metric_context = format!(
        "Metric name: {}\nDescription: {}\nSQL: {}\n",
        metric.name,
        metric.description.clone().unwrap_or_default(),
        metric.sql_query
    );
    if let Some(cache) = &metric.result_cache {
        let sample = serde_json::to_string_pretty(cache).unwrap_or_default();
        let sample: String = sample.chars().take(2000).collect();
        metric_context.push_str(&format!("Sample data:\n{}\n", sample));
    }

    let op_word = match payload.operator.as_str() {
        "gt" => "greater than",
        "gte" => "greater than or equal to",
        "lt" => "less than",
        "lte" => "less than or equal to",
        "eq" => "equal to",
        "ne" => "not equal to",
        _ => &payload.operator,
    };
    let mut condition_desc = format!(
        "Alert fires when the value of column '{}' is {} {}.",
        payload.condition_column.clone().unwrap_or_else(|| "(first numeric column)".to_string()),
        op_word,
        payload.threshold
    );
    if let Some(instr) = &payload.instructions {
        if !instr.trim().is_empty() {
            condition_desc.push_str(&format!("\nAdditional guidance: {}", instr));
        }
    }

    let lang = payload.lang.unwrap_or_else(|| "zh".to_string());
    let system = prompts::alert_template_prompt(&metric_context, &condition_desc, &lang);
    let client = LlmClient::new(llm_cfg.base_url, llm_cfg.api_key, llm_cfg.model);

    let messages = vec![ChatMessage {
        role: "user".into(),
        content: "Generate the alert email template now.".to_string(),
        reasoning_content: None,
    }];

    let template: AlertTemplate = client
        .generate_json(&messages, &system, llm_cfg.max_tokens.max(2048), llm_cfg.temperature)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("AI generation failed: {}", e)))?;

    Ok(Json(template))
}

// ── Alert logs ──

#[derive(Debug, serde::Deserialize)]
pub struct AlertLogQuery {
    pub rule_id: Option<i32>,
    pub limit: Option<i32>,
}

pub async fn list_logs(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Query(params): Query<AlertLogQuery>,
) -> Result<Json<Vec<AlertLog>>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(100).min(500);

    let logs = if let Some(rule_id) = params.rule_id {
        // Verify the caller owns the rule before returning its logs.
        load_owned_rule(&state, rule_id, &user).await?;
        sqlx::query_as::<_, AlertLog>(
            "SELECT * FROM alert_logs WHERE alert_rule_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(rule_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    } else {
        // Only logs for rules the caller owns (admins see all).
        sqlx::query_as::<_, AlertLog>(
            "SELECT al.* FROM alert_logs al \
             JOIN alert_rules ar ON al.alert_rule_id = ar.id \
             WHERE (ar.owner_user_id = ? OR ? = 1) \
             ORDER BY al.created_at DESC LIMIT ?",
        )
        .bind(user.id)
        .bind(user.is_admin as i32)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    }
    .map_err(internal_error)?;

    Ok(Json(logs))
}
