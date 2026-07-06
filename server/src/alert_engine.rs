//! Core alert evaluation + delivery logic shared by the background scheduler
//! and the manual "trigger now / test" HTTP endpoints.

use chrono::Utc;
use serde_json::Value;

use crate::email::{self, EmailAttachment};
use crate::excel;
use crate::feishu;
use crate::models::*;
use crate::routes::query;
use crate::AppState;

/// Result of running an alert rule once.
pub struct AlertOutcome {
    pub triggered: bool,
    pub evaluated_value: Option<f64>,
    pub status: String, // sent | not_triggered | skipped | failed
    pub message: String,
}

/// Evaluate a rule and (if its condition holds) send the alert email.
/// When `force` is true the email is sent regardless of the condition or
/// cooldown — used for "send test email".
pub async fn run_alert(
    state: &AppState,
    rule: &AlertRule,
    force: bool,
) -> Result<AlertOutcome, String> {
    let metric = sqlx::query_as::<_, MetricPool>("SELECT * FROM metric_pools WHERE id = ?")
        .bind(rule.metric_pool_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Metric pool {} not found", rule.metric_pool_id))?;

    let ds = sqlx::query_as::<_, DataSource>("SELECT * FROM datasources WHERE id = ?")
        .bind(metric.datasource_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Datasource {} not found", metric.datasource_id))?;

    // Execute the metric SQL to get fresh data (validated, timed out, row-capped),
    // resolving any {{param}} placeholders with the metric's default values.
    let defaults = query::param_defaults(&metric.params);
    let qr = query::execute_metric_sql(state, &ds, &metric.sql_query, &defaults, &[]).await?;

    let rows_value = serde_json::to_value(&qr.rows).map_err(|e| e.to_string())?;

    // Extract the numeric value to evaluate.
    let evaluated = extract_value(&rows_value, rule.condition_column.as_deref());
    let triggered = match evaluated {
        Some(v) => compare(v, &rule.operator, rule.threshold),
        None => false,
    };

    // Decide whether we actually deliver.
    if !force {
        if !triggered {
            return Ok(AlertOutcome {
                triggered: false,
                evaluated_value: evaluated,
                status: "not_triggered".to_string(),
                message: format!(
                    "Condition not met (value={:?}, {} {})",
                    evaluated, rule.operator, rule.threshold
                ),
            });
        }
        // Cooldown check.
        if rule.cooldown_minutes > 0 {
            if let Some(last) = rule.last_triggered_at {
                let elapsed = Utc::now().signed_duration_since(last);
                if elapsed.num_minutes() < rule.cooldown_minutes as i64 {
                    return Ok(AlertOutcome {
                        triggered: true,
                        evaluated_value: evaluated,
                        status: "skipped".to_string(),
                        message: "Within cooldown window — email suppressed".to_string(),
                    });
                }
            }
        }
    }

    // Load SMTP config (may be absent/disabled — Feishu can still deliver).
    let smtp = sqlx::query_as::<_, SmtpConfig>("SELECT * FROM smtp_config WHERE id = 1")
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let recipients: Vec<String> = serde_json::from_value(rule.recipients.clone()).unwrap_or_default();

    // Render subject + body from templates.
    let ctx = TemplateContext {
        metric_name: &metric.name,
        value: evaluated,
        threshold: rule.threshold,
        operator: &rule.operator,
        rows: &rows_value,
        row_count: qr.row_count,
    };
    let subject = render_template(
        if rule.subject_template.trim().is_empty() {
            "[预警] {{metric_name}}"
        } else {
            &rule.subject_template
        },
        &ctx,
    );
    let body_tpl = rule
        .body_template
        .clone()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| default_body_template());
    let body = render_template(&body_tpl, &ctx);

    // Track delivery across every configured channel. The alert is considered
    // "sent" if at least one channel succeeds; per-channel errors are collected
    // into the outcome message so failures stay visible in the logs.
    let mut delivered: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut attempted = false;

    // ── Email channel ──
    let email_enabled = smtp.as_ref().map(|s| s.enabled).unwrap_or(false);
    if !recipients.is_empty() && email_enabled {
        attempted = true;
        let smtp_cfg = smtp.as_ref().unwrap();

        // Build optional Excel attachment.
        let mut attachments = Vec::new();
        if rule.include_excel {
            match excel::build_metric_xlsx(&metric.name, &rows_value) {
                Ok(bytes) => attachments.push(EmailAttachment {
                    filename: format!(
                        "{}_{}.xlsx",
                        sanitize_filename(&metric.name),
                        Utc::now().format("%Y%m%d_%H%M%S")
                    ),
                    content_type:
                        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
                    bytes,
                }),
                Err(e) => tracing::warn!("Failed to build Excel attachment: {}", e),
            }
        }

        match email::send_email(smtp_cfg, &recipients, &subject, &body, attachments).await {
            Ok(()) => delivered.push(format!("email×{}", recipients.len())),
            Err(e) => errors.push(format!("email: {}", e)),
        }
    }

    // ── Feishu channel ──
    if rule.notify_feishu {
        let feishu_cfg = sqlx::query_as::<_, FeishuConfig>("SELECT * FROM feishu_config WHERE id = 1")
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        match feishu_cfg {
            Some(cfg) if cfg.enabled && !cfg.webhook_url.trim().is_empty() => {
                attempted = true;
                let card = build_alert_card(&metric.name, &ctx);
                match feishu::send_card(&cfg, card).await {
                    Ok(()) => delivered.push("feishu".to_string()),
                    Err(e) => errors.push(format!("feishu: {}", e)),
                }
            }
            _ => errors.push("feishu: not configured or disabled".to_string()),
        }
    }

    // No channel was even attempted — the rule has no usable delivery target.
    if !attempted {
        return Err(
            "No delivery channel available: enable SMTP with recipients, or enable Feishu for this rule."
                .to_string(),
        );
    }

    if delivered.is_empty() {
        // Every attempted channel failed.
        return Err(errors.join("; "));
    }

    let mut message = format!("Delivered via {}", delivered.join(", "));
    if !errors.is_empty() {
        message.push_str(&format!(" (partial failures: {})", errors.join("; ")));
    }

    Ok(AlertOutcome {
        triggered,
        evaluated_value: evaluated,
        status: "sent".to_string(),
        message,
    })
}

/// Build the Feishu interactive card for a triggered alert.
fn build_alert_card(metric_name: &str, ctx: &TemplateContext) -> serde_json::Value {
    let value_str = ctx
        .value
        .map(format_num)
        .unwrap_or_else(|| "—".to_string());
    let condition = format!(
        "{} {} {}",
        value_str,
        operator_symbol(ctx.operator),
        format_num(ctx.threshold)
    );
    let fields = vec![
        feishu::CardField { label: "指标".to_string(), value: metric_name.to_string() },
        feishu::CardField { label: "当前值".to_string(), value: value_str },
        feishu::CardField { label: "触发条件".to_string(), value: condition },
        feishu::CardField { label: "数据行数".to_string(), value: ctx.row_count.to_string() },
        feishu::CardField {
            label: "触发时间".to_string(),
            value: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        },
    ];
    feishu::build_card(
        &format!("⚠️ 指标预警：{}", metric_name),
        "red",
        &fields,
        Some("本卡片由 LingxiBI 自动推送"),
    )
}

/// Persist an alert log entry.
pub async fn log_outcome(state: &AppState, rule: &AlertRule, outcome: &AlertOutcome, error: Option<&str>) {
    let _ = sqlx::query(
        "INSERT INTO alert_logs (alert_rule_id, evaluated_value, triggered, status, message, error, recipients) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(rule.id)
    .bind(outcome.evaluated_value)
    .bind(outcome.triggered)
    .bind(&outcome.status)
    .bind(&outcome.message)
    .bind(error)
    .bind(&rule.recipients)
    .execute(&state.db)
    .await;
}

// ── Condition evaluation ──

/// Pull a numeric value out of the result set. If `column` is provided, read it
/// from the first row; otherwise use the first numeric column found.
fn extract_value(rows: &Value, column: Option<&str>) -> Option<f64> {
    let arr = rows.as_array()?;
    let first = arr.first()?.as_object()?;

    if let Some(col) = column {
        return value_to_f64(first.get(col)?);
    }

    // First numeric column.
    for (_k, v) in first {
        if let Some(n) = value_to_f64(v) {
            return Some(n);
        }
    }
    None
}

fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn compare(value: f64, operator: &str, threshold: f64) -> bool {
    match operator {
        "gt" => value > threshold,
        "gte" => value >= threshold,
        "lt" => value < threshold,
        "lte" => value <= threshold,
        "eq" => (value - threshold).abs() < f64::EPSILON,
        "ne" => (value - threshold).abs() >= f64::EPSILON,
        _ => false,
    }
}

// ── Template rendering ──

struct TemplateContext<'a> {
    metric_name: &'a str,
    value: Option<f64>,
    threshold: f64,
    operator: &'a str,
    rows: &'a Value,
    row_count: usize,
}

/// Replace `{{placeholder}}` tokens. Supported placeholders:
/// metric_name, value, threshold, operator, condition, time, row_count, table
fn render_template(template: &str, ctx: &TemplateContext) -> String {
    let value_str = ctx
        .value
        .map(|v| format_num(v))
        .unwrap_or_else(|| "—".to_string());
    let condition = format!("{} {} {}", value_str, operator_symbol(ctx.operator), format_num(ctx.threshold));

    template
        .replace("{{metric_name}}", ctx.metric_name)
        .replace("{{value}}", &value_str)
        .replace("{{threshold}}", &format_num(ctx.threshold))
        .replace("{{operator}}", operator_symbol(ctx.operator))
        .replace("{{condition}}", &condition)
        .replace("{{time}}", &Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .replace("{{row_count}}", &ctx.row_count.to_string())
        .replace("{{table}}", &rows_to_html_table(ctx.rows))
}

fn operator_symbol(op: &str) -> &str {
    match op {
        "gt" => ">",
        "gte" => "≥",
        "lt" => "<",
        "lte" => "≤",
        "eq" => "=",
        "ne" => "≠",
        _ => op,
    }
}

fn format_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{:.2}", n)
    }
}

/// Render the first up-to-100 result rows as a simple styled HTML table.
fn rows_to_html_table(rows: &Value) -> String {
    let arr = match rows.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return "<p>(无数据)</p>".to_string(),
    };

    let mut columns: Vec<String> = Vec::new();
    for rec in arr {
        if let Some(obj) = rec.as_object() {
            for k in obj.keys() {
                if !columns.iter().any(|c| c == k) {
                    columns.push(k.clone());
                }
            }
        }
    }

    let mut html = String::from(
        "<table style=\"border-collapse:collapse;width:100%;font-size:13px;font-family:Arial,sans-serif;\">",
    );
    html.push_str("<thead><tr>");
    for col in &columns {
        html.push_str(&format!(
            "<th style=\"border:1px solid #ddd;padding:6px 10px;background:#1f1f28;color:#d4a853;text-align:left;\">{}</th>",
            html_escape(col)
        ));
    }
    html.push_str("</tr></thead><tbody>");

    for (i, rec) in arr.iter().take(100).enumerate() {
        let bg = if i % 2 == 0 { "#ffffff" } else { "#f7f7f9" };
        html.push_str(&format!("<tr style=\"background:{};\">", bg));
        let obj = rec.as_object();
        for col in &columns {
            let cell = obj
                .and_then(|o| o.get(col))
                .map(json_cell_to_string)
                .unwrap_or_default();
            html.push_str(&format!(
                "<td style=\"border:1px solid #ddd;padding:6px 10px;color:#333;\">{}</td>",
                html_escape(&cell)
            ));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table>");
    html
}

fn json_cell_to_string(v: &Value) -> String {
    match v {
        Value::Null => "—".to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() { "metric".to_string() } else { trimmed.chars().take(40).collect() }
}

/// The default email body used when no custom/AI template is set.
pub fn default_body_template() -> String {
    r#"<div style="font-family:Arial,sans-serif;max-width:680px;margin:0 auto;background:#0d0d14;padding:24px;border-radius:12px;color:#e5e7eb;">
  <h2 style="color:#d4a853;margin:0 0 8px;">⚠️ 指标预警：{{metric_name}}</h2>
  <p style="color:#9ca3af;margin:0 0 16px;">触发时间：{{time}}</p>
  <div style="background:#12121a;border:1px solid #1f1f28;border-radius:8px;padding:16px;margin-bottom:16px;">
    <p style="margin:0;font-size:15px;">当前值 <strong style="color:#d4a853;font-size:22px;">{{value}}</strong> 已满足预警条件 <strong>{{condition}}</strong>。</p>
  </div>
  <h3 style="color:#e5e7eb;font-size:14px;margin:0 0 8px;">数据明细（共 {{row_count}} 行）</h3>
  {{table}}
  <p style="color:#6b7280;font-size:11px;margin-top:20px;">本邮件由 LingxiBI 自动发送，完整数据见附件 Excel。</p>
</div>"#
        .to_string()
}

/// Compute the next scheduled run for an alert rule.
pub fn compute_next_run(
    schedule_type: &str,
    cron_expr: Option<&str>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = Utc::now();
    match schedule_type {
        "hourly" => Some(now + chrono::Duration::hours(1)),
        "daily" => {
            let tomorrow = (now + chrono::Duration::days(1)).date_naive();
            Some(tomorrow.and_hms_opt(0, 0, 0).unwrap().and_utc())
        }
        "weekly" => Some(now + chrono::Duration::weeks(1)),
        "monthly" => Some(now + chrono::Duration::days(30)),
        "cron" => {
            if let Some(expr) = cron_expr {
                if let Some(secs) = parse_cron_interval_secs(expr) {
                    return Some(now + chrono::Duration::seconds(secs));
                }
            }
            Some(now + chrono::Duration::minutes(1))
        }
        _ => None,
    }
}

fn parse_cron_interval_secs(expr: &str) -> Option<i64> {
    let trimmed = expr.trim();
    if let Some(stripped) = trimmed.strip_suffix('s') {
        if let Ok(n) = stripped.trim().parse::<i64>() {
            if n > 0 {
                return Some(n);
            }
        }
    }
    if let Some(stripped) = trimmed.strip_suffix('m') {
        if let Ok(n) = stripped.trim().parse::<i64>() {
            if n > 0 {
                return Some(n * 60);
            }
        }
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() == 6 {
        if let Some(secs) = parse_step_field(parts[0]) {
            return Some(secs);
        }
    }
    if parts.len() == 5 {
        if let Some(mins) = parse_step_field(parts[0]) {
            return Some(mins * 60);
        }
    }
    None
}

fn parse_step_field(field: &str) -> Option<i64> {
    if let Some(stripped) = field.strip_prefix("*/") {
        if let Ok(n) = stripped.parse::<i64>() {
            if n > 0 {
                return Some(n);
            }
        }
    }
    None
}
