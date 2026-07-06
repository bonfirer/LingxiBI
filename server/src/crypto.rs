//! Authenticated encryption for secrets at rest (AES-256-GCM).
//!
//! Data-source passwords, the LLM API key, the SMTP password and the Feishu
//! signing secret are all encrypted before they touch the metadata database and
//! decrypted transparently at the point of use. A leak of the metadata DB alone
//! no longer exposes any downstream credentials.
//!
//! ## Format
//! Encrypted values are stored as an ASCII string:
//!
//! ```text
//! enc:v1:<base64( nonce(12 bytes) || ciphertext+tag )>
//! ```
//!
//! The `enc:v1:` prefix lets [`decrypt`] transparently pass through legacy
//! plaintext values (from before this feature existed), so upgrades are
//! seamless — existing rows keep working and are re-encrypted on their next
//! write (or eagerly by [`encrypt_existing_secrets`] at startup).
//!
//! ## Key management
//! The 32-byte key is derived (SHA-256) from the `ENCRYPTION_KEY` env var if
//! set, otherwise from `JWT_SECRET` (which is already required at startup).
//! Using a dedicated `ENCRYPTION_KEY` is recommended for production so the
//! encryption key can be rotated independently of the auth signing key.
//!
//! > ⚠️ Rotating the underlying key material invalidates already-stored
//! > secrets — they must be re-entered. Empty values are never encrypted.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Marker prefix identifying a value produced by [`encrypt`].
const PREFIX: &str = "enc:v1:";

static KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Derive (once) the 32-byte AES key from `ENCRYPTION_KEY` or `JWT_SECRET`.
fn key() -> &'static [u8; 32] {
    KEY.get_or_init(|| {
        let material = std::env::var("ENCRYPTION_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("JWT_SECRET").ok())
            .unwrap_or_default();
        // Domain-separated so the derived key can never collide with any other
        // use of the same secret material (e.g. JWT signing).
        let mut hasher = Sha256::new();
        hasher.update(b"lingxibi::credential-encryption::v1::");
        hasher.update(material.as_bytes());
        let digest = hasher.finalize();
        let mut k = [0u8; 32];
        k.copy_from_slice(&digest);
        k
    })
}

/// Whether `value` is already an encrypted ciphertext produced by [`encrypt`].
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(PREFIX)
}

/// Encrypt a secret for storage. Idempotent: empty values and
/// already-encrypted values are returned unchanged.
pub fn encrypt(plaintext: &str) -> String {
    if plaintext.is_empty() || is_encrypted(plaintext) {
        return plaintext.to_string();
    }

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key()));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 96-bit, unique per message
    match cipher.encrypt(&nonce, plaintext.as_bytes()) {
        Ok(ciphertext) => {
            let mut blob = Vec::with_capacity(nonce.len() + ciphertext.len());
            blob.extend_from_slice(nonce.as_slice());
            blob.extend_from_slice(&ciphertext);
            format!(
                "{}{}",
                PREFIX,
                base64::engine::general_purpose::STANDARD.encode(&blob)
            )
        }
        Err(e) => {
            // AES-GCM encryption effectively never fails for valid inputs. If it
            // somehow does, refuse to silently persist plaintext.
            tracing::error!("Credential encryption failed: {}", e);
            String::new()
        }
    }
}

/// Decrypt a stored secret. Legacy plaintext (no [`PREFIX`]) is returned as-is,
/// so this is safe to call on any stored value.
pub fn decrypt(stored: &str) -> String {
    if !is_encrypted(stored) {
        return stored.to_string();
    }
    let b64 = &stored[PREFIX.len()..];
    let blob = match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Credential decode failed: {}", e);
            return String::new();
        }
    };
    if blob.len() < 12 {
        tracing::error!("Credential ciphertext too short");
        return String::new();
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key()));
    let nonce = Nonce::from_slice(nonce_bytes);
    match cipher.decrypt(nonce, ciphertext) {
        Ok(plaintext) => String::from_utf8_lossy(&plaintext).into_owned(),
        Err(e) => {
            // Wrong key (rotated ENCRYPTION_KEY/JWT_SECRET) or corrupted data.
            tracing::error!("Credential decryption failed (wrong key?): {}", e);
            String::new()
        }
    }
}

/// Eagerly encrypt any legacy plaintext credentials still sitting in the
/// metadata DB. Idempotent and best-effort: already-encrypted values are
/// skipped, and any per-table failure is logged without aborting startup.
pub async fn encrypt_existing_secrets(pool: &sqlx::MySqlPool) {
    // (table, id column, secret column)
    let targets = [
        ("datasources", "id", "password"),
        ("llm_config", "id", "api_key"),
        ("smtp_config", "id", "password"),
        ("feishu_config", "id", "secret"),
    ];

    for (table, id_col, col) in targets {
        let query = format!("SELECT {id_col} AS id, {col} AS secret FROM {table}");
        let rows: Vec<(i32, String)> = match sqlx::query_as(&query).fetch_all(pool).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Credential migration: skip {} ({})", table, e);
                continue;
            }
        };

        let mut migrated = 0u32;
        for (id, secret) in rows {
            if secret.is_empty() || is_encrypted(&secret) {
                continue;
            }
            let enc = encrypt(&secret);
            let update = format!("UPDATE {table} SET {col} = ? WHERE {id_col} = ?");
            if let Err(e) = sqlx::query(&update)
                .bind(&enc)
                .bind(id)
                .execute(pool)
                .await
            {
                tracing::warn!("Credential migration: {} id={} failed ({})", table, id, e);
            } else {
                migrated += 1;
            }
        }
        if migrated > 0 {
            tracing::info!("Encrypted {} legacy secret(s) in {}", migrated, table);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        std::env::set_var("ENCRYPTION_KEY", "unit-test-encryption-key-000000");
        let secret = "s3cr3t-p@ssw0rd";
        let enc = encrypt(secret);
        assert!(is_encrypted(&enc));
        assert_ne!(enc, secret);
        assert_eq!(decrypt(&enc), secret);
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(encrypt(""), "");
        assert_eq!(decrypt(""), "");
    }

    #[test]
    fn encrypt_is_idempotent() {
        std::env::set_var("ENCRYPTION_KEY", "unit-test-encryption-key-000000");
        let enc = encrypt("hello");
        // Re-encrypting an already-encrypted value must not double-wrap it.
        assert_eq!(encrypt(&enc), enc);
    }

    #[test]
    fn legacy_plaintext_passes_through() {
        // A value without the prefix is treated as legacy plaintext.
        assert_eq!(decrypt("legacy-plaintext-password"), "legacy-plaintext-password");
    }

    #[test]
    fn nonce_is_randomised() {
        std::env::set_var("ENCRYPTION_KEY", "unit-test-encryption-key-000000");
        // Same plaintext encrypts to different ciphertexts (unique nonce).
        assert_ne!(encrypt("same"), encrypt("same"));
    }
}
