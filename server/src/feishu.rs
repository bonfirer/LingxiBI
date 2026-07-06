//! Feishu (Lark) custom-bot webhook delivery.
//!
//! Pushes metric alerts to a Feishu group chat as an interactive message card.
//! Supports the optional "signed request" (加签) security mode: when a secret is
//! configured, each request carries a `timestamp` + HMAC-SHA256 `sign` so Feishu
//! can verify the caller.
//!
//! This is a self-contained notification channel that mirrors the SMTP/email
//! module, and serves as the template for future chat integrations (DingTalk,
//! WeChat Work, Slack, ...).

use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::models::FeishuConfig;

type HmacSha256 = Hmac<Sha256>;

/// Apex domains that legitimately host Feishu / Lark custom-bot webhooks.
/// Any other host is rejected to prevent the webhook from being used as an
/// SSRF primitive (pointing the server at internal services / metadata).
const ALLOWED_WEBHOOK_APEX: &[&str] = &["feishu.cn", "larksuite.com", "larkoffice.com"];

/// Validate that a webhook URL is an HTTPS Feishu/Lark endpoint.
///
/// This is the SSRF guard for the Feishu channel: it must be `https` and its
/// host must be (a subdomain of) an official Feishu/Lark domain.
pub fn validate_webhook_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|_| "Invalid webhook URL".to_string())?;

    if parsed.scheme() != "https" {
        return Err("Feishu webhook must use https".to_string());
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "Webhook URL has no host".to_string())?
        .to_lowercase();

    let allowed = ALLOWED_WEBHOOK_APEX
        .iter()
        .any(|apex| host == *apex || host.ends_with(&format!(".{}", apex)));

    if !allowed {
        return Err(format!(
            "Webhook host '{}' is not an allowed Feishu/Lark domain",
            host
        ));
    }
    Ok(())
}

/// A single key/value line rendered in the alert card body.
pub struct CardField {
    pub label: String,
    pub value: String,
}

/// Compute the Feishu signature for a given unix `timestamp` and `secret`.
///
/// Algorithm (per Feishu docs): the string `"{timestamp}\n{secret}"` is used as
/// the HMAC-SHA256 *key* over an empty message, then base64-encoded.
fn sign(timestamp: i64, secret: &str) -> Result<String, String> {
    let string_to_sign = format!("{}\n{}", timestamp, secret);
    let mut mac = HmacSha256::new_from_slice(string_to_sign.as_bytes())
        .map_err(|e| format!("Failed to init HMAC: {}", e))?;
    // The message is intentionally empty; the secret material lives in the key.
    mac.update(b"");
    let result = mac.finalize().into_bytes();
    Ok(base64::engine::general_purpose::STANDARD.encode(result))
}

/// Build an interactive card payload from a title, colored header, body lines
/// and an optional footer note.
pub fn build_card(
    title: &str,
    header_color: &str,
    fields: &[CardField],
    footer: Option<&str>,
) -> Value {
    let mut content = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        content.push_str(&format!("**{}：** {}", f.label, f.value));
    }
    if content.is_empty() {
        content.push_str(" ");
    }

    let mut elements = vec![json!({
        "tag": "div",
        "text": { "tag": "lark_md", "content": content }
    })];

    if let Some(note) = footer {
        elements.push(json!({ "tag": "hr" }));
        elements.push(json!({
            "tag": "note",
            "elements": [{ "tag": "plain_text", "content": note }]
        }));
    }

    json!({
        "config": { "wide_screen_mode": true },
        "header": {
            "template": header_color,
            "title": { "tag": "plain_text", "content": title }
        },
        "elements": elements
    })
}

/// Send a pre-built interactive card to the configured webhook.
pub async fn send_card(cfg: &FeishuConfig, card: Value) -> Result<(), String> {
    if cfg.webhook_url.trim().is_empty() {
        return Err("Feishu webhook URL is not configured".to_string());
    }
    // SSRF guard: refuse to send anywhere that isn't an official Feishu/Lark host.
    validate_webhook_url(&cfg.webhook_url)?;

    let mut body = json!({
        "msg_type": "interactive",
        "card": card,
    });

    // Attach signature fields when a secret is configured.
    let secret = crate::crypto::decrypt(&cfg.secret);
    if !secret.trim().is_empty() {
        let timestamp = chrono::Utc::now().timestamp();
        let signature = sign(timestamp, secret.trim())?;
        body["timestamp"] = json!(timestamp.to_string());
        body["sign"] = json!(signature);
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(cfg.webhook_url.trim())
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Feishu request failed: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("Feishu HTTP {}: {}", status, text));
    }

    // Feishu always returns HTTP 200, signalling errors in the JSON body via a
    // non-zero `code` (or legacy `StatusCode`). Parse and surface those.
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        let code = v.get("code").and_then(|c| c.as_i64());
        let legacy = v.get("StatusCode").and_then(|c| c.as_i64());
        let ok = matches!(code, Some(0) | None) && matches!(legacy, Some(0) | None);
        if !ok {
            let msg = v
                .get("msg")
                .and_then(|m| m.as_str())
                .or_else(|| v.get("StatusMessage").and_then(|m| m.as_str()))
                .unwrap_or("unknown error");
            return Err(format!("Feishu rejected message: {} ({})", msg, text));
        }
    }

    tracing::info!("Feishu card delivered (HTTP {})", status);
    Ok(())
}

/// Send a plain-text message (used by the "test" button).
pub async fn send_text(cfg: &FeishuConfig, text: &str) -> Result<(), String> {
    let card = build_card(
        "✅ LingxiBI — 飞书测试",
        "green",
        &[CardField {
            label: "状态".to_string(),
            value: text.to_string(),
        }],
        Some("如果你在群里看到这张卡片，说明飞书告警通道已正常工作。"),
    );
    send_card(cfg, card).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_deterministic_and_base64() {
        let s1 = sign(1599360473, "secret").unwrap();
        let s2 = sign(1599360473, "secret").unwrap();
        assert_eq!(s1, s2);
        // base64 of a 32-byte HMAC-SHA256 digest is 44 chars.
        assert_eq!(s1.len(), 44);
        // A different timestamp yields a different signature.
        assert_ne!(s1, sign(1599360474, "secret").unwrap());
    }

    #[test]
    fn card_contains_title_and_fields() {
        let card = build_card(
            "标题",
            "red",
            &[CardField { label: "当前值".into(), value: "42".into() }],
            Some("footer"),
        );
        let s = card.to_string();
        assert!(s.contains("标题"));
        assert!(s.contains("当前值"));
        assert!(s.contains("42"));
        assert!(s.contains("footer"));
    }

    #[test]
    fn webhook_url_ssrf_guard() {
        // Valid official endpoints.
        assert!(validate_webhook_url("https://open.feishu.cn/open-apis/bot/v2/hook/abc").is_ok());
        assert!(validate_webhook_url("https://open.larksuite.com/open-apis/bot/v2/hook/abc").is_ok());
        // Rejected: non-https.
        assert!(validate_webhook_url("http://open.feishu.cn/hook/abc").is_err());
        // Rejected: internal / metadata / arbitrary hosts (SSRF attempts).
        assert!(validate_webhook_url("https://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_webhook_url("https://localhost/hook").is_err());
        assert!(validate_webhook_url("https://evil.example.com/open-apis/bot/v2/hook/abc").is_err());
        // Rejected: look-alike domain trying to suffix-match.
        assert!(validate_webhook_url("https://feishu.cn.evil.com/hook").is_err());
    }
}
