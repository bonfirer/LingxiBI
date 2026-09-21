use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::AppState;

/// Log an internal error and return a generic message — never leak DB/internal
/// detail to unauthenticated callers (login/register/setup are public).
fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("auth internal error: {}", e);
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
}

// ── Simple in-memory login rate limiter (per username) ──
// Locks out a username after too many failed attempts within a window. This is
// best-effort brute-force mitigation; it is per-process (resets on restart) and
// keyed by username, which is sufficient for a single-instance admin tool.

const MAX_FAILED_ATTEMPTS: u32 = 5;
const LOCKOUT_WINDOW: Duration = Duration::from_secs(300); // 5 minutes

struct AttemptRecord {
    failures: u32,
    window_start: Instant,
}

fn login_attempts() -> &'static Mutex<HashMap<String, AttemptRecord>> {
    static ATTEMPTS: OnceLock<Mutex<HashMap<String, AttemptRecord>>> = OnceLock::new();
    ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns Err with seconds-remaining if the username is currently locked out.
fn check_rate_limit(username: &str) -> Result<(), u64> {
    // Recover from a poisoned lock instead of panicking: a single panic while
    // holding the lock must not permanently break authentication.
    let map = login_attempts().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(rec) = map.get(username) {
        if rec.window_start.elapsed() < LOCKOUT_WINDOW && rec.failures >= MAX_FAILED_ATTEMPTS {
            let remaining = LOCKOUT_WINDOW.as_secs().saturating_sub(rec.window_start.elapsed().as_secs());
            return Err(remaining);
        }
    }
    Ok(())
}

fn record_failure(username: &str) {
    let mut map = login_attempts().lock().unwrap_or_else(|e| e.into_inner());
    let rec = map.entry(username.to_string()).or_insert(AttemptRecord {
        failures: 0,
        window_start: Instant::now(),
    });
    // Reset the counter if the previous window has expired.
    if rec.window_start.elapsed() >= LOCKOUT_WINDOW {
        rec.failures = 0;
        rec.window_start = Instant::now();
    }
    rec.failures += 1;
}

fn clear_failures(username: &str) {
    login_attempts()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(username);
}

/// Read the JWT signing secret from the environment.
/// The server refuses to start (see `main.rs`) if this is unset, so by the time
/// any request is handled we always have a real secret here.
pub fn jwt_secret() -> String {
    std::env::var("JWT_SECRET").unwrap_or_default()
}

/// The authenticated caller, injected into request extensions by the auth
/// middleware and pulled into handlers via `Extension<AuthUser>`.
#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub id: i32,
    pub is_admin: bool,
}

/// Decoded, signature- and expiry-verified claims of a token we issued.
pub struct TokenClaims {
    pub user_id: i32,
    /// `None` for full session tokens, `Some("embed")` for the short-lived
    /// read-only tokens used by report iframes.
    pub scope: Option<String>,
    /// "admin"/"member" from a session token; embed tokens carry no role.
    pub role: Option<String>,
    /// `users.token_version` at issue time. Tokens minted before this claim
    /// existed have `None` and are treated as version 0.
    pub token_version: i64,
}

/// Decode a JWT and return its claims if the signature and expiry are valid.
fn decode_claims_full(token: &str) -> Result<TokenClaims, ()> {
    let (user_id, scope, role, token_version) = decode_claims(token)?;
    Ok(TokenClaims { user_id, scope, role, token_version })
}

/// Decode a JWT and return `(user_id, scope, role, token_version)` if the
/// signature and expiry are valid.
fn decode_claims(token: &str) -> Result<(i32, Option<String>, Option<String>, i64), ()> {
    let secret = jwt_secret();
    if secret.is_empty() {
        return Err(());
    }
    let data = jsonwebtoken::decode::<serde_json::Value>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )
    .map_err(|_| ())?;

    let user_id = data.claims.get("sub").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    if user_id <= 0 {
        return Err(());
    }
    let scope = data
        .claims
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let role = data
        .claims
        .get("role")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let token_version = data.claims.get("tv").and_then(|v| v.as_i64()).unwrap_or(0);
    Ok((user_id, scope, role, token_version))
}

/// Validate a full-session bearer token and return `(user_id, is_admin)`.
///
/// Narrowly-scoped embed tokens are REJECTED here, so a leaked embed token
/// (which may travel through URLs, browser history, and access logs) can never
/// be used against admin/mutation routes — only the report iframe endpoints.
pub fn validate_session(token: &str) -> Result<(i32, bool), ()> {
    let (user_id, scope, role, _tv) = decode_claims(token)?;
    match scope.as_deref() {
        Some("embed") => Err(()),
        _ => Ok((user_id, role.as_deref() == Some("admin"))),
    }
}

/// Confirm a decoded token still corresponds to a live session.
///
/// Signature + expiry alone are not enough: a JWT keeps asserting whatever it
/// was minted with, so without this check a deleted user kept full API access,
/// a password reset didn't kick out existing sessions, and a demoted admin kept
/// `role: admin` until the token expired. Bumping `users.token_version`
/// invalidates every token issued before the bump.
async fn session_is_current(state: &AppState, claims: &TokenClaims) -> bool {
    let row: Option<(i64,)> = sqlx::query_as("SELECT token_version FROM users WHERE id = ?")
        .bind(claims.user_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    match row {
        // User still exists and the token was issued at the current version.
        Some((current,)) => current == claims.token_version,
        // User deleted -> every token for it is dead.
        None => false,
    }
}

/// Mint a short-lived, read-only token for embedding report data into iframes.
/// Lifetime is intentionally short and the token is scope-limited so it cannot
/// reach any endpoint other than the report html/data readers.
pub fn create_embed_token(user_id: i32, token_version: i64) -> Result<String, ()> {
    let secret = jwt_secret();
    if secret.is_empty() {
        return Err(());
    }
    let claims = serde_json::json!({
        "sub": user_id,
        "scope": "embed",
        // Carries the same revocation marker as a session token, so revoking a
        // user's sessions also kills their outstanding embed tokens.
        "tv": token_version,
        "exp": chrono::Utc::now().timestamp() + EMBED_TOKEN_TTL_SECS,
    });
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| ())
}

/// Lifetime of an embed token (30 minutes).
const EMBED_TOKEN_TTL_SECS: i64 = 60 * 30;

/// Lifetime of a full session token (24 hours).
///
/// Was 7 days, which is a long time for a credential that is also accepted from
/// a URL query parameter. Revocation now exists (`users.token_version`), but a
/// shorter window still limits the damage from a leaked token.
const SESSION_TOKEN_TTL_SECS: i64 = 60 * 60 * 24;

/// Mint an embed token stamped with the user's current `token_version`.
pub async fn create_embed_token_for(state: &AppState, user_id: i32) -> Result<String, ()> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT token_version FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| ())?;
    let (token_version,) = row.ok_or(())?;
    create_embed_token(user_id, token_version)
}

/// `GET /api/embed-token` — issue a short-lived embed token for the current
/// user. The `require_auth` middleware has already verified a valid, current
/// full session token, and injected the caller as `AuthUser`.
pub async fn embed_token(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthUser>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let token = create_embed_token_for(&state, user.id)
        .await
        .map_err(|_| internal("embed token encode failed"))?;
    Ok(Json(serde_json::json!({
        "token": token,
        "expires_in": EMBED_TOKEN_TTL_SECS,
    })))
}

/// Axum middleware: require a valid `Authorization: Bearer <jwt>` header.
/// On success, injects `AuthUser` into request extensions so handlers can scope
/// data by owner. Applied to all protected routes; public routes bypass this.
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let claims = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| decode_claims_full(t).ok())
        // Embed tokens are read-only and travel through URLs; they must never
        // authorize a full session.
        .filter(|c| c.scope.as_deref() != Some("embed"))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !session_is_current(&state, &claims).await {
        return Err(StatusCode::UNAUTHORIZED);
    }

    req.extensions_mut().insert(AuthUser {
        id: claims.user_id,
        is_admin: claims.role.as_deref() == Some("admin"),
    });
    Ok(next.run(req).await)
}

/// Like `require_auth`, but also accepts the token from a `?token=` query param.
/// Used for endpoints loaded directly by the browser (iframes, embedded fetches)
/// where an Authorization header cannot be set. Also injects `AuthUser`.
pub async fn require_auth_flexible(
    State(state): State<Arc<AppState>>,
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    // 1. Try the Authorization header
    let header_claims = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| decode_claims_full(t).ok());

    // 2. Fall back to a ?token= query parameter
    let claims = header_claims
        .or_else(|| {
            req.uri()
                .query()
                .and_then(url_decode_token)
                .and_then(|t| decode_claims_full(&t).ok())
        })
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !session_is_current(&state, &claims).await {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Embed tokens are accepted here, but are never treated as admin.
    let is_admin =
        claims.scope.as_deref() != Some("embed") && claims.role.as_deref() == Some("admin");
    req.extensions_mut().insert(AuthUser {
        id: claims.user_id,
        is_admin,
    });
    Ok(next.run(req).await)
}

/// A JWT is always `base64url(header).base64url(payload).base64url(signature)`,
/// so a legitimate token only ever contains this character set. Rejecting
/// anything else here means a token value can never carry HTML/JS syntax into
/// whatever consumes it downstream.
fn is_wellformed_jwt(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 4096
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Extract and percent-decode the `token` parameter from a raw query string.
///
/// Deliberately strict:
/// - A query string carrying more than one `token=` is rejected. Different
///   parsers disagree on which occurrence wins (this function took the first,
///   while axum's `Query<HashMap<_,_>>` keeps the last), and that mismatch let a
///   request authenticate with one value while a handler consumed another.
/// - The decoded value must look like a JWT, so it can never smuggle markup.
fn url_decode_token(query: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for pair in query.split('&') {
        if let Some(val) = pair.strip_prefix("token=") {
            if found.is_some() {
                return None;
            }
            let decoded = urlencoding::decode(val)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| val.to_string());
            found = Some(decoded);
        }
    }
    found.filter(|t| is_wellformed_jwt(t))
}

#[derive(serde::Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(serde::Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub id: i32,
    pub username: String,
    pub display_name: Option<String>,
    pub role: String,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    pub id: i32,
    pub username: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub token_version: i64,
}

/// POST /api/auth/login
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    // Reject early if this username is temporarily locked out.
    if let Err(secs) = check_rate_limit(&body.username) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many failed attempts. Try again in {} seconds.", secs),
        ));
    }

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, display_name, role, token_version FROM users WHERE username = ?"
    )
    .bind(&body.username)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    // Verify password only if the user exists. On any failure, count it and
    // return the same generic message (don't leak which usernames exist).
    let user = match user {
        Some(u) if bcrypt::verify(&body.password, &u.password_hash).unwrap_or(false) => u,
        _ => {
            record_failure(&body.username);
            return Err((StatusCode::UNAUTHORIZED, "Invalid username or password".to_string()));
        }
    };

    // Successful login — clear the failure counter.
    clear_failures(&body.username);

    // Generate JWT. `tv` pins the token to the account's current token_version
    // so it can be revoked; a missing/NULL role resolves to the LEAST privilege,
    // never admin.
    let secret = jwt_secret();
    let claims = serde_json::json!({
        "sub": user.id,
        "username": user.username,
        "role": normalize_role(user.role.as_deref()),
        "tv": user.token_version,
        "exp": chrono::Utc::now().timestamp() + SESSION_TOKEN_TTL_SECS,
    });

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(internal)?;

    let role = normalize_role(user.role.as_deref()).to_string();

    Ok(Json(LoginResponse {
        token,
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        role,
    }))
}

/// POST /api/auth/register (for initial setup only)
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    // Check if any users exist — only allow registration if no users
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await
        .map_err(internal)?;

    if count.0 > 0 {
        return Err((StatusCode::FORBIDDEN, "Registration disabled. Users already exist.".to_string()));
    }

    // Basic input hardening for the initial admin account.
    if body.username.trim().len() < 3 || body.username.len() > 64 {
        return Err((StatusCode::BAD_REQUEST, "Username must be 3–64 characters.".to_string()));
    }
    if body.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "Password must be at least 8 characters.".to_string()));
    }

    let hash = bcrypt::hash(&body.password, 12)
        .map_err(internal)?;

    sqlx::query("INSERT INTO users (username, password_hash, display_name, role) VALUES (?, ?, ?, 'admin')")
        .bind(&body.username)
        .bind(&hash)
        .bind(&body.username)
        .execute(&state.db)
        .await
        .map_err(internal)?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "message": "User created" }))))
}

/// GET /api/auth/me — validate token and return user info.
///
/// Uses exactly the same acceptance rules as `require_auth`: it previously
/// decoded the token by hand and so accepted read-only embed tokens (leaking the
/// account's username/role to anything holding one) and ignored revocation.
pub async fn me(
    State(state): State<Arc<AppState>>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let unauthorized = || (StatusCode::UNAUTHORIZED, "Invalid token".to_string());

    let claims = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| decode_claims_full(t).ok())
        .filter(|c| c.scope.as_deref() != Some("embed"))
        .ok_or_else(unauthorized)?;

    if !session_is_current(&state, &claims).await {
        return Err(unauthorized());
    }

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, display_name, role, token_version FROM users WHERE id = ?"
    )
    .bind(claims.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?
    .ok_or_else(unauthorized)?;

    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "display_name": user.display_name,
        "role": normalize_role(user.role.as_deref()),
    })))
}

/// Check if the system has any registered users (for initial setup flow).
pub async fn check_setup(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await
        .map_err(internal)?;

    Ok(Json(serde_json::json!({ "has_users": count.0 > 0 })))
}

// ── Admin-only user management ──
//
// These endpoints let an admin add/manage additional users. They live behind
// `require_auth`, and each re-checks `AuthUser::is_admin` so members cannot
// reach them even if the route is discovered.

fn require_admin(user: &AuthUser) -> Result<(), (StatusCode, String)> {
    if user.is_admin {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "Admin privileges required".to_string()))
    }
}

fn normalize_role(role: Option<&str>) -> &'static str {
    match role {
        Some("admin") => "admin",
        _ => "member",
    }
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct UserSummary {
    pub id: i32,
    pub username: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// GET /api/users — list all users (admin only).
pub async fn list_users(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthUser>,
) -> Result<Json<Vec<UserSummary>>, (StatusCode, String)> {
    require_admin(&user)?;
    let users = sqlx::query_as::<_, UserSummary>(
        "SELECT id, username, display_name, role, created_at FROM users ORDER BY id ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;
    Ok(Json(users))
}

#[derive(serde::Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
}

/// POST /api/users — create a new user (admin only).
pub async fn create_user(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthUser>,
    Json(body): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserSummary>), (StatusCode, String)> {
    require_admin(&user)?;

    if body.username.trim().len() < 3 || body.username.len() > 64 {
        return Err((StatusCode::BAD_REQUEST, "Username must be 3–64 characters.".to_string()));
    }
    if body.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "Password must be at least 8 characters.".to_string()));
    }

    let role = normalize_role(body.role.as_deref());
    let hash = bcrypt::hash(&body.password, 12).map_err(internal)?;
    let display = body.display_name.as_deref().unwrap_or(body.username.trim());

    let result = sqlx::query(
        "INSERT INTO users (username, password_hash, display_name, role) VALUES (?, ?, ?, ?)",
    )
    .bind(body.username.trim())
    .bind(&hash)
    .bind(display)
    .bind(role)
    .execute(&state.db)
    .await
    .map_err(|e| {
        // Duplicate username (1062 / 23000) → 409 instead of a generic 500.
        let dup = e
            .as_database_error()
            .map(|d| d.code().as_deref() == Some("23000"))
            .unwrap_or(false);
        if dup {
            (StatusCode::CONFLICT, "Username already exists".to_string())
        } else {
            internal(e)
        }
    })?;

    let created = sqlx::query_as::<_, UserSummary>(
        "SELECT id, username, display_name, role, created_at FROM users WHERE id = ?",
    )
    .bind(result.last_insert_id() as i32)
    .fetch_one(&state.db)
    .await
    .map_err(internal)?;

    Ok((StatusCode::CREATED, Json(created)))
}

#[derive(serde::Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub role: Option<String>,
    /// If present and non-empty, resets the user's password.
    pub password: Option<String>,
}

/// PUT /api/users/{id} — update a user's display name, role, or password (admin only).
pub async fn update_user(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthUser>,
    axum::extract::Path(id): axum::extract::Path<i32>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<UserSummary>, (StatusCode, String)> {
    require_admin(&user)?;

    let existing = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, display_name, role, token_version FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    // Guard: don't allow demoting the last remaining admin (would lock everyone
    // out of user management).
    let new_role = match body.role.as_deref() {
        Some(r) => normalize_role(Some(r)),
        None => normalize_role(existing.role.as_deref()),
    };
    if existing.role.as_deref() == Some("admin") && new_role != "admin" {
        let admin_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(&state.db)
            .await
            .map_err(internal)?;
        if admin_count.0 <= 1 {
            return Err((StatusCode::BAD_REQUEST, "Cannot demote the last admin.".to_string()));
        }
    }

    let display = body.display_name.as_deref().unwrap_or(existing.display_name.as_deref().unwrap_or(&existing.username));

    // Reset password only when a non-empty value is supplied.
    let password_reset = matches!(body.password.as_deref(), Some(p) if !p.is_empty());
    let password_hash = match body.password.as_deref() {
        Some(p) if !p.is_empty() => {
            if p.len() < 8 {
                return Err((StatusCode::BAD_REQUEST, "Password must be at least 8 characters.".to_string()));
            }
            bcrypt::hash(p, 12).map_err(internal)?
        }
        _ => existing.password_hash.clone(),
    };

    // Invalidate existing sessions when the credential changes or privileges
    // shrink — otherwise the old token keeps working (with its old role) until
    // it expires, which makes a password reset or demotion cosmetic.
    let role_changed = existing.role.as_deref().unwrap_or("member") != new_role;
    let revoke_sessions = password_reset || role_changed;

    sqlx::query(
        "UPDATE users SET display_name = ?, role = ?, password_hash = ?,
                token_version = token_version + ?, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?",
    )
        .bind(display)
        .bind(new_role)
        .bind(&password_hash)
        .bind(i64::from(revoke_sessions))
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal)?;

    let updated = sqlx::query_as::<_, UserSummary>(
        "SELECT id, username, display_name, role, created_at FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .map_err(internal)?;

    Ok(Json(updated))
}

/// DELETE /api/users/{id} — delete a user (admin only).
/// Refuses to delete yourself or the last admin.
pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthUser>,
    axum::extract::Path(id): axum::extract::Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    require_admin(&user)?;

    if id == user.id {
        return Err((StatusCode::BAD_REQUEST, "You cannot delete your own account.".to_string()));
    }

    let target = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, display_name, role, token_version FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    if target.role.as_deref() == Some("admin") {
        let admin_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin'")
            .fetch_one(&state.db)
            .await
            .map_err(internal)?;
        if admin_count.0 <= 1 {
            return Err((StatusCode::BAD_REQUEST, "Cannot delete the last admin.".to_string()));
        }
    }

    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal)?;

    Ok(StatusCode::NO_CONTENT)
}

// ── GitHub OAuth ──

/// Whether GitHub OAuth is configured (both env vars present and non-empty).
pub fn github_oauth_enabled() -> bool {
    std::env::var("GITHUB_CLIENT_ID").map(|v| !v.is_empty()).unwrap_or(false)
        && std::env::var("GITHUB_CLIENT_SECRET").map(|v| !v.is_empty()).unwrap_or(false)
}

/// Name of the cookie holding the OAuth `state` value.
const OAUTH_STATE_COOKIE: &str = "oauth_state";

/// Read a single cookie value out of a request's `Cookie` header.
fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_string())
    })
}

/// Whether the request reached us over TLS, so the state cookie can be marked
/// `Secure` when (and only when) that won't stop the browser from sending it.
fn request_is_https(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|p| p.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

/// Length-checked, non-short-circuiting comparison for opaque tokens.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// GET /api/auth/github — returns the GitHub OAuth authorization URL.
/// The frontend opens this URL to start the OAuth flow.
///
/// Also issues a random `state` value, returned both inside the authorization
/// URL and as an HttpOnly cookie. The callback requires the two to match, which
/// is what stops an attacker from feeding a victim a pre-baked callback URL and
/// silently signing them into the attacker's GitHub account (login CSRF /
/// account fixation). A signed-but-unbound state would not help here, since the
/// attacker can simply request one of their own.
pub async fn github_login(
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let client_id = std::env::var("GITHUB_CLIENT_ID")
        .map_err(|_| (StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()))?;
    if client_id.is_empty() {
        return Err((StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()));
    }

    let state = uuid::Uuid::new_v4().to_string().replace('-', "");

    // The redirect_uri isn't hard-coded; we let the browser's origin handle it.
    // GitHub will redirect to the callback URL configured in the OAuth App settings.
    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&scope=user:email&state={}",
        client_id, state
    );

    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Lax; Path=/api/auth; Max-Age=600{}",
        OAUTH_STATE_COOKIE,
        state,
        if request_is_https(&headers) { "; Secure" } else { "" }
    );
    let mut out = axum::http::HeaderMap::new();
    out.insert(
        axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_str(&cookie).map_err(internal)?,
    );

    Ok((out, Json(serde_json::json!({ "url": url }))).into_response())
}

#[derive(serde::Deserialize)]
pub struct GitHubCallbackQuery {
    pub code: String,
    pub state: Option<String>,
}

/// Whether `login` may self-provision a local account.
///
/// Historically any GitHub account on the internet got a `member` account here,
/// which silently bypassed "registration disabled" and was enough to reach the
/// chat assistant. Signup is now opt-in:
/// - `GITHUB_ALLOWED_LOGINS` — comma-separated allowlist of GitHub logins.
/// - `GITHUB_ALLOW_SIGNUP=true` — allow any GitHub account (previous behaviour).
///
/// Bootstrapping is unaffected: the very first user of an empty instance is
/// always allowed, so an OAuth-only deployment can still be set up.
fn github_signup_allowed(login: &str, is_first_user: bool) -> bool {
    if is_first_user {
        return true;
    }
    let allowlist = std::env::var("GITHUB_ALLOWED_LOGINS").unwrap_or_default();
    let listed = allowlist
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .any(|allowed| allowed.eq_ignore_ascii_case(login));
    if listed {
        return true;
    }
    std::env::var("GITHUB_ALLOW_SIGNUP")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

/// GitHub user info from their API. Only the fields we persist are captured;
/// serde ignores the rest of the payload.
#[derive(serde::Deserialize)]
struct GitHubUser {
    id: i64,
    login: String,
    name: Option<String>,
}

/// GET /api/auth/github/callback?code=... — exchange the code for a token,
/// fetch user info, find-or-create a local user, issue a JWT, and redirect
/// the browser back to the app with the token in the URL fragment.
pub async fn github_callback(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<GitHubCallbackQuery>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let client_id = std::env::var("GITHUB_CLIENT_ID")
        .map_err(|_| (StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()))?;
    let client_secret = std::env::var("GITHUB_CLIENT_SECRET")
        .map_err(|_| (StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()))?;

    // CSRF: the `state` echoed back by GitHub must match the cookie we set when
    // this flow started. Missing either side means the callback wasn't initiated
    // from this browser, so refuse it.
    let cookie_state = cookie_value(&headers, OAUTH_STATE_COOKIE).ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth state missing or expired. Start the sign-in again.".to_string(),
    ))?;
    let query_state = params.state.as_deref().unwrap_or_default();
    if query_state.is_empty() || !constant_time_eq(&cookie_state, query_state) {
        return Err((
            StatusCode::BAD_REQUEST,
            "OAuth state mismatch. Start the sign-in again.".to_string(),
        ));
    }

    // Exchange the authorization code for an access token.
    let http = crate::http::client();
    let token_res = http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": params.code,
        }))
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub token exchange failed: {}", e)))?;

    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: Option<String>,
        error_description: Option<String>,
    }

    let token_body: TokenResponse = token_res
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub token parse failed: {}", e)))?;

    let access_token = token_body.access_token.ok_or_else(|| {
        let desc = token_body.error_description.unwrap_or_else(|| "unknown error".to_string());
        (StatusCode::BAD_REQUEST, format!("GitHub auth failed: {}", desc))
    })?;

    // Fetch the authenticated GitHub user's profile.
    let user_res = http
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", "HISENSE LingxiBI")
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub user fetch failed: {}", e)))?;

    let gh_user: GitHubUser = user_res
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub user parse failed: {}", e)))?;

    // Find or create the local user by github_id.
    let existing: Option<(i32, String, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, username, display_name, role, token_version FROM users WHERE github_id = ?",
    )
    .bind(gh_user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let (user_id, username, role, token_version) = if let Some((id, uname, _display, role, tv)) = existing {
        (id, uname, normalize_role(role.as_deref()).to_string(), tv)
    } else {
        // New user — if no users exist yet, make them admin; otherwise member.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&state.db)
            .await
            .map_err(internal)?;
        let is_first_user = count.0 == 0;
        let new_role = if is_first_user { "admin" } else { "member" };

        if !github_signup_allowed(&gh_user.login, is_first_user) {
            tracing::warn!(
                "Rejected GitHub sign-up for '{}' (not allowlisted and GITHUB_ALLOW_SIGNUP is off)",
                gh_user.login
            );
            return Err((
                StatusCode::FORBIDDEN,
                "This GitHub account is not authorized for this instance. Ask an administrator to add it."
                    .to_string(),
            ));
        }

        // Use github login as username; append suffix if collision.
        let base_username = gh_user.login.clone();
        let mut final_username = base_username.clone();
        let mut attempt = 0;
        loop {
            let dup: Option<(i32,)> = sqlx::query_as("SELECT id FROM users WHERE username = ?")
                .bind(&final_username)
                .fetch_optional(&state.db)
                .await
                .map_err(internal)?;
            if dup.is_none() {
                break;
            }
            attempt += 1;
            final_username = format!("{}_{}", base_username, attempt);
        }

        let display = gh_user.name.unwrap_or_else(|| gh_user.login.clone());
        // No password hash for OAuth users (can't login via password).
        let result = sqlx::query(
            "INSERT INTO users (username, password_hash, display_name, role, github_id) VALUES (?, '', ?, ?, ?)",
        )
        .bind(&final_username)
        .bind(&display)
        .bind(new_role)
        .bind(gh_user.id)
        .execute(&state.db)
        .await
        .map_err(internal)?;

        (result.last_insert_id() as i32, final_username, new_role.to_string(), 0)
    };

    // Issue JWT (same as normal login).
    let secret = jwt_secret();
    let claims = serde_json::json!({
        "sub": user_id,
        "username": username,
        "role": role,
        "tv": token_version,
        "exp": chrono::Utc::now().timestamp() + SESSION_TOKEN_TTL_SECS,
    });
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(internal)?;

    // Hand the token back in the URL *fragment*, not the query string. Fragments
    // are never sent to the server, so the JWT stays out of reverse-proxy access
    // logs and out of `Referer` headers. The SPA reads it from location.hash and
    // immediately scrubs it from history.
    let mut out = axum::http::HeaderMap::new();
    // The state cookie is single-use — expire it now that the flow is complete.
    out.insert(
        axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_str(&format!(
            "{}=; HttpOnly; SameSite=Lax; Path=/api/auth; Max-Age=0",
            OAUTH_STATE_COOKIE
        ))
        .map_err(internal)?,
    );

    let redirect = axum::response::Redirect::temporary(&format!(
        "/#github_token={}&user_id={}&username={}&role={}",
        urlencoding::encode(&token),
        user_id,
        urlencoding::encode(&username),
        urlencoding::encode(&role),
    ));

    Ok((out, redirect).into_response())
}

/// GET /api/auth/github/enabled — quick check for the frontend to show/hide the button.
pub async fn github_enabled() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "enabled": github_oauth_enabled() }))
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    const TEST_SECRET: &str = "test-jwt-secret-at-least-16-chars-long";

    fn session_token(user_id: i32, role: &str) -> String {
        std::env::set_var("JWT_SECRET", TEST_SECRET);
        let claims = serde_json::json!({
            "sub": user_id,
            "role": role,
            "exp": chrono::Utc::now().timestamp() + 3600,
        });
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn normalize_role_defaults_to_member() {
        // Only an exact "admin" grants admin; everything else is a member.
        assert_eq!(normalize_role(Some("admin")), "admin");
        assert_eq!(normalize_role(Some("member")), "member");
        assert_eq!(normalize_role(Some("root")), "member");
        assert_eq!(normalize_role(Some("ADMIN")), "member");
        assert_eq!(normalize_role(None), "member");
    }

    #[test]
    fn session_token_carries_role() {
        assert_eq!(validate_session(&session_token(1, "admin")), Ok((1, true)));
        assert_eq!(validate_session(&session_token(2, "member")), Ok((2, false)));
        // Any non-admin role resolves to non-admin.
        assert_eq!(validate_session(&session_token(3, "superuser")), Ok((3, false)));
    }

    #[test]
    fn embed_token_is_rejected_as_a_full_session() {
        std::env::set_var("JWT_SECRET", TEST_SECRET);
        let embed = create_embed_token(7, 0).unwrap();
        // Critical: a leaked read-only embed token must never authorize a full
        // session (and thus never reach admin/mutation routes).
        assert!(validate_session(&embed).is_err());
        // `require_auth` / `me` reject it by scope before any DB lookup.
        let claims = decode_claims_full(&embed).unwrap();
        assert_eq!(claims.scope.as_deref(), Some("embed"));
        assert_eq!(claims.user_id, 7);
    }

    #[test]
    fn session_tokens_carry_the_revocation_version() {
        std::env::set_var("JWT_SECRET", TEST_SECRET);
        // Embed tokens are stamped with the version they were minted at, so a
        // token_version bump invalidates them along with session tokens.
        let embed = create_embed_token(7, 42).unwrap();
        assert_eq!(decode_claims_full(&embed).unwrap().token_version, 42);
        // Tokens minted before the claim existed decode as version 0.
        assert_eq!(decode_claims_full(&session_token(1, "admin")).unwrap().token_version, 0);
    }

    #[test]
    fn garbage_and_tampered_tokens_rejected() {
        std::env::set_var("JWT_SECRET", TEST_SECRET);
        assert!(validate_session("not-a-jwt").is_err());
        assert!(validate_session("").is_err());
        let mut tampered = session_token(1, "admin");
        tampered.push('x'); // corrupt the signature
        assert!(validate_session(&tampered).is_err());
    }

    #[test]
    fn token_signed_with_other_secret_rejected() {
        std::env::set_var("JWT_SECRET", TEST_SECRET);
        let claims = serde_json::json!({
            "sub": 1, "role": "admin", "exp": chrono::Utc::now().timestamp() + 3600,
        });
        let forged = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"a-totally-different-secret-value"),
        )
        .unwrap();
        assert!(validate_session(&forged).is_err());
    }

    #[test]
    fn non_positive_subject_rejected() {
        std::env::set_var("JWT_SECRET", TEST_SECRET);
        let claims = serde_json::json!({
            "sub": 0, "role": "admin", "exp": chrono::Utc::now().timestamp() + 3600,
        });
        let tok = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .unwrap();
        assert!(validate_session(&tok).is_err());
    }

    #[test]
    fn duplicate_token_params_are_rejected() {
        // Regression: this function took the FIRST `token=` while axum's
        // `Query<HashMap<_,_>>` keeps the LAST one, so a request could
        // authenticate with a valid token while a handler consumed an
        // attacker-supplied second value. Ambiguous queries are now refused.
        assert_eq!(url_decode_token("token=abc"), Some("abc".to_string()));
        assert_eq!(url_decode_token("id=1&token=abc"), Some("abc".to_string()));
        assert_eq!(url_decode_token("token=abc&token=def"), None);
        assert_eq!(url_decode_token("token=abc&x=1&token=def"), None);
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        // A token value must look like a JWT, so it can never carry markup into
        // a consumer that interpolates it.
        assert_eq!(url_decode_token("token=a.b-c_D9"), Some("a.b-c_D9".to_string()));
        assert_eq!(url_decode_token("token="), None);
        assert_eq!(url_decode_token("token=%3C/script%3E%3Csvg+onload%3Dx%3E"), None);
        assert_eq!(url_decode_token("token=has space"), None);
        assert_eq!(url_decode_token("token=quote\"inside"), None);
        assert_eq!(url_decode_token("nothing=here"), None);
    }

    #[test]
    fn cookie_value_picks_the_named_cookie() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            axum::http::header::COOKIE,
            "a=1; oauth_state=abc123; b=2".parse().unwrap(),
        );
        assert_eq!(cookie_value(&h, OAUTH_STATE_COOKIE), Some("abc123".to_string()));
        assert_eq!(cookie_value(&h, "missing"), None);
        assert_eq!(cookie_value(&axum::http::HeaderMap::new(), "a"), None);
    }

    #[test]
    fn constant_time_eq_matches_only_identical_values() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc", "abc123"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn github_bootstrap_signup_is_always_allowed() {
        // An empty instance must still be able to onboard its first admin over
        // OAuth, regardless of allowlist configuration.
        assert!(github_signup_allowed("anyone", true));
    }

    #[test]
    fn github_signup_requires_allowlist_or_optin() {
        std::env::set_var("GITHUB_ALLOWED_LOGINS", " alice , Bob ");
        std::env::remove_var("GITHUB_ALLOW_SIGNUP");
        assert!(github_signup_allowed("alice", false));
        // Allowlist comparison is case-insensitive on the GitHub login.
        assert!(github_signup_allowed("BOB", false));
        // Not listed and signup not enabled -> rejected (previously any GitHub
        // account on the internet got a member account here).
        assert!(!github_signup_allowed("mallory", false));

        std::env::set_var("GITHUB_ALLOW_SIGNUP", "true");
        assert!(github_signup_allowed("mallory", false));

        std::env::remove_var("GITHUB_ALLOWED_LOGINS");
        std::env::remove_var("GITHUB_ALLOW_SIGNUP");
    }

    #[test]
    fn rate_limiter_locks_out_after_max_attempts() {
        let user = "ratelimit-test-user-unique";
        clear_failures(user);
        assert!(check_rate_limit(user).is_ok());
        for _ in 0..MAX_FAILED_ATTEMPTS {
            record_failure(user);
        }
        assert!(check_rate_limit(user).is_err(), "should lock out after {MAX_FAILED_ATTEMPTS} failures");
        clear_failures(user);
        assert!(check_rate_limit(user).is_ok(), "clear_failures should reset the lockout");
    }
}
