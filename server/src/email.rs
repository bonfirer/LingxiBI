//! SMTP email delivery built on top of `lettre`.
//!
//! Supports plain-text/HTML bodies plus an optional binary attachment
//! (used to ship the metric data as an `.xlsx` file).

use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::models::SmtpConfig;

/// A single attachment to ship with the email.
pub struct EmailAttachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Parse a bare email address ("user@host") into a lettre `Address`, with a
/// friendly error that points at the offending value.
fn parse_address(raw: &str) -> Result<Address, String> {
    let trimmed = raw.trim();
    trimmed.parse::<Address>().map_err(|_| {
        format!(
            "Invalid email address '{}'. It must be a single address like name@example.com (no spaces).",
            trimmed
        )
    })
}

/// Send an HTML email (optionally with attachments) using the stored SMTP config.
pub async fn send_email(
    cfg: &SmtpConfig,
    to: &[String],
    subject: &str,
    html_body: &str,
    attachments: Vec<EmailAttachment>,
) -> Result<(), String> {
    if cfg.host.trim().is_empty() {
        return Err("SMTP host is not configured".to_string());
    }
    if cfg.from_email.trim().is_empty() {
        return Err("SMTP from_email is not configured".to_string());
    }
    let valid_to: Vec<&String> = to.iter().filter(|a| !a.trim().is_empty()).collect();
    if valid_to.is_empty() {
        return Err("No recipients provided".to_string());
    }

    // Build the From mailbox from a parsed address + optional display name, so a
    // display name with spaces/punctuation never corrupts the address itself.
    let from_addr = parse_address(&cfg.from_email).map_err(|e| format!("From address — {}", e))?;
    let from_name = cfg.from_name.trim();
    let from_mbox = if from_name.is_empty() {
        Mailbox::new(None, from_addr)
    } else {
        Mailbox::new(Some(from_name.to_string()), from_addr)
    };

    let mut builder = Message::builder()
        .from(from_mbox.clone())
        // Put the real recipients in Bcc so they can't see each other's
        // addresses; the visible To is just the sender itself.
        .to(from_mbox)
        .subject(subject);

    for addr in &valid_to {
        let parsed = parse_address(addr).map_err(|e| format!("Recipient — {}", e))?;
        builder = builder.bcc(Mailbox::new(None, parsed));
    }

    // Build the body: a multipart/mixed when attachments are present, otherwise
    // a single HTML part.
    let html_part = SinglePart::builder()
        .header(ContentType::TEXT_HTML)
        .body(html_body.to_string());


    let email = if attachments.is_empty() {
        builder
            .singlepart(html_part)
            .map_err(|e| format!("Failed to build email: {}", e))?
    } else {
        let mut multipart = MultiPart::mixed().singlepart(html_part);
        for att in attachments {
            let content_type = ContentType::parse(&att.content_type)
                .unwrap_or(ContentType::parse("application/octet-stream").unwrap());
            multipart = multipart.singlepart(
                Attachment::new(att.filename).body(att.bytes, content_type),
            );
        }
        builder
            .multipart(multipart)
            .map_err(|e| format!("Failed to build email: {}", e))?
    };

    let creds = Credentials::new(cfg.username.clone(), crate::crypto::decrypt(&cfg.password));

    // Port 465 → implicit TLS (SMTPS). Port 587 → STARTTLS. Otherwise plaintext.
    let transport = if cfg.use_tls && cfg.port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
            .map_err(|e| format!("SMTP relay setup failed: {}", e))?
            .port(cfg.port as u16)
            .credentials(creds)
            .build()
    } else if cfg.use_tls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
            .map_err(|e| format!("SMTP STARTTLS setup failed: {}", e))?
            .port(cfg.port as u16)
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
            .port(cfg.port as u16)
            .credentials(creds)
            .build()
    };

    let mode = if cfg.use_tls && cfg.port == 465 {
        "implicit-TLS"
    } else if cfg.use_tls {
        "STARTTLS"
    } else {
        "plaintext"
    };
    tracing::info!(
        "Sending email via {}:{} ({}), from={}, to={:?}, subject={:?}",
        cfg.host,
        cfg.port,
        mode,
        cfg.from_email,
        valid_to,
        subject
    );

    let response = transport
        .send(email)
        .await
        .map_err(|e| {
            tracing::warn!("SMTP send failed: {}", e);
            format!("SMTP send failed: {}", e)
        })?;

    tracing::info!(
        "SMTP server accepted message: code={:?}, message={:?}",
        response.code(),
        response.message().collect::<Vec<_>>()
    );

    Ok(())
}
