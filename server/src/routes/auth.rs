use axum::{
    extract::State,
    http::StatusCode,
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

/// Decode a JWT and return `(user_id, scope, role)` if the signature and expiry
/// are valid. `scope` is `None` for full session tokens, `Some("embed")` for the
/// short-lived read-only tokens used by report iframes. `role` comes from the
/// session token ("admin"/"member"); embed tokens carry no role.
fn decode_claims(token: &str) -> Result<(i32, Option<String>, Option<String>), ()> {
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
    Ok((user_id, scope, role))
}

/// Validate a full-session bearer token and return `(user_id, is_admin)`.
///
/// Narrowly-scoped embed tokens are REJECTED here, so a leaked embed token
/// (which may travel through URLs, browser history, and access logs) can never
/// be used against admin/mutation routes — only the report iframe endpoints.
pub fn validate_session(token: &str) -> Result<(i32, bool), ()> {
    let (user_id, scope, role) = decode_claims(token)?;
    match scope.as_deref() {
        Some("embed") => Err(()),
        _ => Ok((user_id, role.as_deref() == Some("admin"))),
    }
}

/// Validate a full-session bearer token and return just the user id.
pub fn validate_token(token: &str) -> Result<i32, ()> {
    validate_session(token).map(|(id, _)| id)
}

/// Validate a token for the report iframe endpoints (`/html`, `/data`).
/// Accepts either a full session token or a short-lived embed token.
/// Embed tokens are never treated as admin.
pub fn validate_embed_or_session(token: &str) -> Result<(i32, bool), ()> {
    let (user_id, scope, role) = decode_claims(token)?;
    let is_admin = scope.as_deref() != Some("embed") && role.as_deref() == Some("admin");
    Ok((user_id, is_admin))
}

/// Mint a short-lived, read-only token for embedding report data into iframes.
/// Lifetime is intentionally short and the token is scope-limited so it cannot
/// reach any endpoint other than the report html/data readers.
pub fn create_embed_token(user_id: i32) -> Result<String, ()> {
    let secret = jwt_secret();
    if secret.is_empty() {
        return Err(());
    }
    let claims = serde_json::json!({
        "sub": user_id,
        "scope": "embed",
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

/// `GET /api/embed-token` — issue a short-lived embed token for the current
/// user. The `require_auth` middleware has already verified a valid full
/// session token in the `Authorization` header.
pub async fn embed_token(
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user_id = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| validate_token(t).ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))?;

    let token = create_embed_token(user_id).map_err(|_| internal("embed token encode failed"))?;
    Ok(Json(serde_json::json!({
        "token": token,
        "expires_in": EMBED_TOKEN_TTL_SECS,
    })))
}

/// Axum middleware: require a valid `Authorization: Bearer <jwt>` header.
/// On success, injects `AuthUser` into request extensions so handlers can scope
/// data by owner. Applied to all protected routes; public routes bypass this.
pub async fn require_auth(
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let auth = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| validate_session(t).ok());

    match auth {
        Some((id, is_admin)) => {
            req.extensions_mut().insert(AuthUser { id, is_admin });
            Ok(next.run(req).await)
        }
        None => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Like `require_auth`, but also accepts the token from a `?token=` query param.
/// Used for endpoints loaded directly by the browser (iframes, embedded fetches)
/// where an Authorization header cannot be set. Also injects `AuthUser`.
pub async fn require_auth_flexible(
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    // 1. Try the Authorization header
    let header_auth = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| validate_embed_or_session(t).ok());

    // 2. Fall back to a ?token= query parameter
    let auth = header_auth.or_else(|| {
        req.uri()
            .query()
            .and_then(url_decode_token)
            .and_then(|t| validate_embed_or_session(&t).ok())
    });

    match auth {
        Some((id, is_admin)) => {
            req.extensions_mut().insert(AuthUser { id, is_admin });
            Ok(next.run(req).await)
        }
        None => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Extract and percent-decode the `token` parameter from a raw query string.
fn url_decode_token(query: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some(val) = pair.strip_prefix("token=") {
            // Minimal percent-decoding for the token value
            let decoded = val
                .replace('+', " ")
                .replace("%2B", "+")
                .replace("%2F", "/")
                .replace("%3D", "=");
            return Some(decoded);
        }
    }
    None
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
        "SELECT id, username, password_hash, display_name, role FROM users WHERE username = ?"
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

    // Generate JWT
    let secret = jwt_secret();
    let claims = serde_json::json!({
        "sub": user.id,
        "username": user.username,
        "role": user.role.as_deref().unwrap_or("admin"),
        "exp": chrono::Utc::now().timestamp() + 86400 * 7, // 7 days
    });

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(internal)?;

    let role = user.role.as_deref().unwrap_or("admin").to_string();

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

/// GET /api/auth/me — validate token and return user info
pub async fn me(
    State(state): State<Arc<AppState>>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let token = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "Missing token".to_string()))?;

    let secret = jwt_secret();
    let data = jsonwebtoken::decode::<serde_json::Value>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )
    .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token".to_string()))?;

    let user_id = data.claims.get("sub").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, display_name, role FROM users WHERE id = ?"
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::UNAUTHORIZED, "User not found".to_string()))?;

    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "display_name": user.display_name,
        "role": user.role,
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
        "SELECT id, username, password_hash, display_name, role FROM users WHERE id = ?",
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
    let password_hash = match body.password.as_deref() {
        Some(p) if !p.is_empty() => {
            if p.len() < 8 {
                return Err((StatusCode::BAD_REQUEST, "Password must be at least 8 characters.".to_string()));
            }
            bcrypt::hash(p, 12).map_err(internal)?
        }
        _ => existing.password_hash.clone(),
    };

    sqlx::query("UPDATE users SET display_name = ?, role = ?, password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(display)
        .bind(new_role)
        .bind(&password_hash)
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
        "SELECT id, username, password_hash, display_name, role FROM users WHERE id = ?",
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

/// GET /api/auth/github — returns the GitHub OAuth authorization URL.
/// The frontend opens this URL to start the OAuth flow.
pub async fn github_login() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client_id = std::env::var("GITHUB_CLIENT_ID")
        .map_err(|_| (StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()))?;
    if client_id.is_empty() {
        return Err((StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()));
    }

    // The redirect_uri isn't hard-coded; we let the browser's origin handle it.
    // GitHub will redirect to the callback URL configured in the OAuth App settings.
    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&scope=user:email",
        client_id
    );
    Ok(Json(serde_json::json!({ "url": url })))
}

#[derive(serde::Deserialize)]
pub struct GitHubCallbackQuery {
    pub code: String,
}

/// GitHub user info from their API.
#[derive(serde::Deserialize)]
struct GitHubUser {
    id: i64,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}

/// GET /api/auth/github/callback?code=... — exchange the code for a token,
/// fetch user info, find-or-create a local user, issue a JWT, and redirect
/// the browser back to the app with the token in the URL fragment.
pub async fn github_callback(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<GitHubCallbackQuery>,
) -> Result<axum::response::Redirect, (StatusCode, String)> {
    let client_id = std::env::var("GITHUB_CLIENT_ID")
        .map_err(|_| (StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()))?;
    let client_secret = std::env::var("GITHUB_CLIENT_SECRET")
        .map_err(|_| (StatusCode::NOT_IMPLEMENTED, "GitHub OAuth not configured".to_string()))?;

    // Exchange the authorization code for an access token.
    let http = reqwest::Client::new();
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
        .header("User-Agent", "LingxiBI")
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub user fetch failed: {}", e)))?;

    let gh_user: GitHubUser = user_res
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub user parse failed: {}", e)))?;

    // Find or create the local user by github_id.
    let existing: Option<(i32, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, username, display_name, role FROM users WHERE github_id = ?",
    )
    .bind(gh_user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let (user_id, username, role) = if let Some((id, uname, _display, role)) = existing {
        (id, uname, role.unwrap_or_else(|| "member".to_string()))
    } else {
        // New user — if no users exist yet, make them admin; otherwise member.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&state.db)
            .await
            .map_err(internal)?;
        let new_role = if count.0 == 0 { "admin" } else { "member" };

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

        (result.last_insert_id() as i32, final_username, new_role.to_string())
    };

    // Issue JWT (same as normal login).
    let secret = jwt_secret();
    let claims = serde_json::json!({
        "sub": user_id,
        "username": username,
        "role": role,
        "exp": chrono::Utc::now().timestamp() + 86400 * 7,
    });
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(internal)?;

    // Redirect back to the SPA with the token in a query param so the frontend
    // can pick it up and store it. Using a fragment (#) would be more secure but
    // the SPA router may eat it; a short-lived query param is acceptable here
    // because the JWT is session-scoped and the URL is on the user's own browser.
    Ok(axum::response::Redirect::temporary(&format!(
        "/?github_token={}&user_id={}&username={}&role={}",
        urlencoding::encode(&token),
        user_id,
        urlencoding::encode(&username),
        urlencoding::encode(&role),
    )))
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
        let embed = create_embed_token(7).unwrap();
        // Critical: a leaked read-only embed token must never authorize a full
        // session (and thus never reach admin/mutation routes).
        assert!(validate_session(&embed).is_err());
        assert!(validate_token(&embed).is_err());
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
