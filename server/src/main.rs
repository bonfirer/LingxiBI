use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use std::sync::Arc;
use tokio::signal;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use tracing_subscriber;

mod crypto;
mod db_pool;
mod llm;
mod models;
mod routes;
mod snapshot_scheduler;
mod alert_scheduler;
mod alert_engine;
mod email;
mod feishu;
mod excel;
mod column_profiler;
pub mod ai_log;

use db_pool::PoolCache;

#[derive(Clone)]
pub struct AppState {
    pub db: MySqlPool,
    pub pool_cache: PoolCache,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .init();

    dotenvy::dotenv().ok();

    // Security: require a JWT signing secret. Refuse to start without one so we
    // never fall back to a guessable default that would let anyone forge tokens.
    match std::env::var("JWT_SECRET") {
        Ok(s) if s.len() >= 16 => {}
        Ok(_) => panic!("JWT_SECRET must be at least 16 characters. Set a strong random value."),
        Err(_) => panic!("JWT_SECRET is not set. Set a strong random value (>= 16 chars) before starting."),
    }

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://root:password@localhost:3306/ai_report".to_string());

    let pool = MySqlPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("Failed to connect to metadata database");

    // Run schema migrations, tracked in `schema_migrations` so each runs exactly
    // once and genuine failures abort startup (see `run_migrations`).
    run_migrations(&pool, MIGRATIONS).await;

    // Encrypt-at-rest: transparently upgrade any legacy plaintext credentials
    // (datasource passwords, LLM key, SMTP password, Feishu secret) to
    // AES-256-GCM ciphertext. Safe to run on every startup.
    crypto::encrypt_existing_secrets(&pool).await;

    let state = Arc::new(AppState {
        db: pool,
        pool_cache: PoolCache::new(),
    });

    let allowed_origin = std::env::var("CORS_ALLOWED_ORIGIN")
        .unwrap_or_else(|_| "*".to_string());

    let cors = if allowed_origin == "*" {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        CorsLayer::new()
            .allow_origin(
                allowed_origin
                    .parse::<axum::http::HeaderValue>()
                    .expect("Invalid CORS_ALLOWED_ORIGIN"),
            )
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let protected = Router::new()
        .route("/api/datasources", post(routes::datasources::create))
        .route("/api/datasources", get(routes::datasources::list))
        .route("/api/datasources/{id}", get(routes::datasources::get_one))
        .route("/api/datasources/{id}", put(routes::datasources::update))
        .route("/api/datasources/{id}", delete(routes::datasources::remove))
        .route("/api/datasources/{id}/test", post(routes::datasources::test_connection))
        .route("/api/datasources/{id}/introspect", post(routes::datasources::introspect))
        .route("/api/datasources/{id}/schema", get(routes::datasources::get_schema))
        .route("/api/datasources/{id}/profile", post(routes::datasources::profile))
        .route("/api/datasources/{id}/grants", get(routes::datasources::list_grants))
        .route("/api/datasources/{id}/grants", put(routes::datasources::set_grants))
        .route("/api/datasources/{id}/default", put(routes::datasources::set_default))
        .route("/api/datasources/{id}/table-descriptions", get(routes::table_descriptions::list))
        .route("/api/datasources/{id}/table-descriptions", post(routes::table_descriptions::upsert))
        .route("/api/datasources/{id}/column-descriptions", get(routes::table_descriptions::list_columns))
        .route("/api/datasources/{id}/column-descriptions", post(routes::table_descriptions::upsert_column))
        .route("/api/knowledge-graph/{ds_id}", get(routes::knowledge_graph::get_graph))
        .route("/api/knowledge-graph/{ds_id}/refresh", post(routes::knowledge_graph::refresh_graph))
        .route("/api/conversations", get(routes::conversations::list))
        .route("/api/conversations", post(routes::conversations::create))
        .route("/api/conversations/{id}", get(routes::conversations::get_messages))
        .route("/api/conversations/{id}/status", get(routes::conversations::get_status))
        .route("/api/conversations/{id}", delete(routes::conversations::delete))
        .route("/api/query/execute", post(routes::query::execute))
        .route("/api/query/{pool_id}", get(routes::query::get_pool))
        .route("/api/reports", get(routes::reports::list))
        .route("/api/reports", post(routes::reports::create))
        .route("/api/reports/{id}", get(routes::reports::get_one))
        .route("/api/reports/{id}/render", post(routes::reports::render))
        .route("/api/reports/{id}/status", get(routes::reports::get_status))
        .route("/api/reports/{id}/summary", get(routes::reports::get_summary))
        .route("/api/reports/{id}/summary", post(routes::reports::generate_summary))
        .route("/api/reports/{id}/ask", post(routes::reports::ask_report))
        .route("/api/reports/{id}", delete(routes::reports::delete))
        .route("/api/reports/{id}/move", put(routes::report_groups::move_report))
        .route("/api/reports/{id}/publish", put(routes::reports::publish))
        .route("/api/reports/{id}/rollback", post(routes::reports::rollback))
        .route("/api/reports/{id}/share", post(routes::reports::share))
        .route("/api/reports/{id}/datasources", get(routes::report_datasources::list))
        .route("/api/reports/{id}/datasources", post(routes::report_datasources::create))
        .route("/api/reports/{id}/datasources/{ds_id}", delete(routes::report_datasources::remove))
        .route("/api/reports/{id}/datasources/{ds_id}/refresh", post(routes::report_datasources::refresh))
        .route("/api/reports/{id}/datasources/{ds_id}/filters", put(routes::report_datasources::set_filters))
        .route("/api/reports/{id}/datasources/{ds_id}/ai-filters", post(routes::report_datasources::ai_filters))
        .route("/api/reports/{id}/filters", put(routes::reports::set_report_filters))
        .route("/api/reports/{id}/debug", get(routes::reports::debug_sql))
        .route("/api/reports/{id}/refresh-interval", put(routes::reports::update_refresh_interval))
        .route("/api/reports/{id}/style", put(routes::reports::update_style))
        .route("/api/reports/{id}/versions", get(routes::reports::list_versions))
        .route("/api/reports/{id}/versions/{vid}/restore", post(routes::reports::restore_version))
        .route("/api/reports/{id}/versions/{vid}", delete(routes::reports::delete_version))
        .route("/api/report-groups", get(routes::report_groups::list))
        .route("/api/report-groups", post(routes::report_groups::create))
        .route("/api/report-groups/{id}", put(routes::report_groups::update))
        .route("/api/report-groups/{id}", delete(routes::report_groups::remove))
        // Report Themes (user-curated, reusable dashboard styles)
        .route("/api/report-themes", get(routes::report_themes::list))
        .route("/api/report-themes", post(routes::report_themes::create))
        .route("/api/report-themes/{id}", delete(routes::report_themes::delete))
        .route("/api/metric-groups", get(routes::metric_groups::list))
        .route("/api/metric-groups", post(routes::metric_groups::create))
        .route("/api/metric-groups/{id}", put(routes::metric_groups::update))
        .route("/api/metric-groups/{id}", delete(routes::metric_groups::remove))
        .route("/api/metrics", get(routes::metric_pools::list))
        .route("/api/metrics", post(routes::metric_pools::create))
        .route("/api/metrics/ai-parameterize", post(routes::metric_pools::ai_parameterize))
        .route("/api/metrics/ai-generate", post(routes::metric_pools::ai_generate))
        .route("/api/metrics/{id}", get(routes::metric_pools::get_one))
        .route("/api/metrics/{id}", put(routes::metric_pools::update))
        .route("/api/metrics/{id}", delete(routes::metric_pools::remove))
        .route("/api/metrics/{id}/refresh", post(routes::metric_pools::refresh))
        .route("/api/metrics/{id}/move", put(routes::metric_pools::move_metric))
        // Snapshot schedules & data
        .route("/api/snapshot-schedules", get(routes::snapshots::list_schedules))
        .route("/api/snapshot-schedules", post(routes::snapshots::create_schedule))
        .route("/api/snapshot-schedules/{id}", put(routes::snapshots::update_schedule))
        .route("/api/snapshot-schedules/{id}", delete(routes::snapshots::delete_schedule))
        .route("/api/metrics/{id}/schedule", get(routes::snapshots::get_schedule))
        .route("/api/metrics/{id}/snapshots", get(routes::snapshots::list_snapshots))
        .route("/api/metrics/{id}/snapshots", post(routes::snapshots::take_snapshot))
        .route("/api/metrics/{id}/snapshots/{snapshot_id}", delete(routes::snapshots::delete_snapshot))
        .route("/api/metrics/{id}/snapshots/compare", get(routes::snapshots::compare_snapshots))
        // Email Alerts — SMTP config
        .route("/api/alerts/smtp", get(routes::alerts::get_smtp))
        .route("/api/alerts/smtp", put(routes::alerts::update_smtp))
        .route("/api/alerts/smtp/test", post(routes::alerts::test_smtp))
        // Alerts — Feishu config
        .route("/api/alerts/feishu", get(routes::alerts::get_feishu))
        .route("/api/alerts/feishu", put(routes::alerts::update_feishu))
        .route("/api/alerts/feishu/test", post(routes::alerts::test_feishu))
        // Email Alerts — rules
        .route("/api/alerts/rules", get(routes::alerts::list_rules))
        .route("/api/alerts/rules", post(routes::alerts::create_rule))
        .route("/api/alerts/rules/{id}", get(routes::alerts::get_rule))
        .route("/api/alerts/rules/{id}", put(routes::alerts::update_rule))
        .route("/api/alerts/rules/{id}", delete(routes::alerts::delete_rule))
        .route("/api/alerts/rules/{id}/trigger", post(routes::alerts::trigger_rule))
        .route("/api/alerts/rules/{id}/test", post(routes::alerts::test_rule))
        // Email Alerts — AI template + logs
        .route("/api/alerts/generate-template", post(routes::alerts::generate_template))
        .route("/api/alerts/logs", get(routes::alerts::list_logs))
        .route("/api/llm/config", get(routes::llm_config::get_config))
        .route("/api/llm/config", put(routes::llm_config::update_config))
        .route("/api/llm/config/test", post(routes::llm_config::test_connection))
        // Knowledge Base
        .route("/api/knowledge-base", get(routes::knowledge_base::list))
        .route("/api/knowledge-base", post(routes::knowledge_base::create))
        .route("/api/knowledge-base/{id}", put(routes::knowledge_base::update))
        .route("/api/knowledge-base/{id}", delete(routes::knowledge_base::delete))
        .route("/api/knowledge-base/datasource/{ds_id}", get(routes::knowledge_base::list_by_datasource))
        // AI Logs
        .route("/api/ai-logs", get(routes::ai_logs::list))
        .route("/api/ai-logs/{id}", get(routes::ai_logs::get_one))
        // AI Examples
        .route("/api/ai-examples", get(routes::ai_examples::list))
        .route("/api/ai-examples", post(routes::ai_examples::create))
        .route("/api/ai-examples/{id}", delete(routes::ai_examples::delete))
        .route("/api/ai-examples/datasource/{ds_id}", get(routes::ai_examples::list_by_datasource))
        // Achievements
        .route("/api/achievements", get(routes::achievements::list))
        // Feature wishes (feedback / feature requests)
        .route("/api/wishes", get(routes::wishes::list))
        .route("/api/wishes", post(routes::wishes::create))
        .route("/api/wishes/{id}", put(routes::wishes::update))
        .route("/api/wishes/{id}", delete(routes::wishes::delete))
        .route("/api/wishes/summary", get(routes::wishes::summary))
        // Short-lived, read-only token for embedding reports in iframes.
        .route("/api/embed-token", get(routes::auth::embed_token))
        // Admin-only user management (each handler re-checks admin role).
        .route("/api/users", get(routes::auth::list_users))
        .route("/api/users", post(routes::auth::create_user))
        .route("/api/users/{id}", put(routes::auth::update_user))
        .route("/api/users/{id}", delete(routes::auth::delete_user))
        // All routes above require a valid JWT.
        .route_layer(axum::middleware::from_fn(routes::auth::require_auth));

    // Routes loaded directly by the browser inside iframes / embedded fetches.
    // Auth accepted via Authorization header OR a ?token= query param.
    let flexible = Router::new()
        .route("/api/reports/{id}/html", get(routes::reports::get_html))
        .route("/api/reports/{id}/data", get(routes::reports::get_live_data))
        .route("/api/reports/{id}/versions/{vid}/html", get(routes::reports::get_version_html))
        .route_layer(axum::middleware::from_fn(routes::auth::require_auth_flexible));

    // Public routes — no auth required.
    // - health check
    // - auth endpoints (login/register/check; `me` validates its own token)
    // - public share links (guarded by an unguessable UUID token)
    // - chat WebSocket (validates token from the query string inside the handler,
    //   since browsers cannot set Authorization headers on WS connections)
    let public = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/chat", get(routes::chat::ws_handler))
        .route("/api/share/{token}", get(routes::reports::view_shared))
        .route("/api/share/{token}/html", get(routes::reports::view_shared_html))
        .route("/api/auth/login", post(routes::auth::login))
        .route("/api/auth/register", post(routes::auth::register))
        .route("/api/auth/me", get(routes::auth::me))
        .route("/api/auth/check", get(routes::auth::check_setup))
        // GitHub OAuth
        .route("/api/auth/github", get(routes::auth::github_login))
        .route("/api/auth/github/callback", get(routes::auth::github_callback))
        .route("/api/auth/github/enabled", get(routes::auth::github_enabled));

    let app = public
        .merge(protected)
        .merge(flexible)
        .layer(axum::middleware::from_fn(security_headers))
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)) // 10 MB max body
        .layer(cors)
        .with_state(state.clone());

    // Start background snapshot scheduler
    snapshot_scheduler::spawn(state.clone());

    // Start background email alert scheduler
    alert_scheduler::spawn(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001")
        .await
        .unwrap();
    tracing::info!("Server listening on http://0.0.0.0:3001");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

/// Attach a baseline set of security response headers to every response.
///
/// Deliberately conservative so it can't break the SPA, the iframe report
/// viewer, or the embed feature: it sets only universally-safe headers and
/// avoids a hard `X-Frame-Options`/CSP that would interfere with embedding.
async fn security_headers(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header::{HeaderName, HeaderValue};
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, name: &'static str, value: &'static str| {
        h.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    };
    // Stop browsers from MIME-sniffing responses into an unexpected type.
    set(h, "x-content-type-options", "nosniff");
    // Don't leak full URLs (which may carry ?token=) to cross-origin targets.
    set(h, "referrer-policy", "strict-origin-when-cross-origin");
    // Disable the legacy XSS auditor (modern guidance; avoids its own bugs).
    set(h, "x-xss-protection", "0");
    // Lock down a few powerful browser features by default.
    set(h, "permissions-policy", "geolocation=(), microphone=(), camera=()");
    res
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, draining connections...");
}

async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Quick connectivity check on metadata DB
    let db_ok = sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .is_ok();

    let status = if db_ok {
        (StatusCode::OK, Json(serde_json::json!({"status": "healthy", "db": "connected"})))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "unhealthy", "db": "disconnected"})),
        )
    };

    status
}

/// Ordered manifest of schema migrations: `(version, sql)`. The version is the
/// file stem and is recorded in `schema_migrations` once applied. Append new
/// migrations to the end — never reorder or rewrite an applied one.
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_init", include_str!("../migrations/001_init.sql")),
    ("002_groups_and_metrics", include_str!("../migrations/002_groups_and_metrics.sql")),
    ("003_report_canvas", include_str!("../migrations/003_report_canvas.sql")),
    ("004_report_html", include_str!("../migrations/004_report_html.sql")),
    ("005_refresh_interval", include_str!("../migrations/005_refresh_interval.sql")),
    ("006_knowledge_base", include_str!("../migrations/006_knowledge_base.sql")),
    ("007_generation_status", include_str!("../migrations/007_generation_status.sql")),
    ("008_published_html", include_str!("../migrations/008_published_html.sql")),
    ("009_users", include_str!("../migrations/009_users.sql")),
    ("010_ai_logs", include_str!("../migrations/010_ai_logs.sql")),
    ("011_ai_examples", include_str!("../migrations/011_ai_examples.sql")),
    ("012_ai_logs_params", include_str!("../migrations/012_ai_logs_params.sql")),
    ("013_report_style", include_str!("../migrations/013_report_style.sql")),
    ("014_report_score_and_achievements", include_str!("../migrations/014_report_score_and_achievements.sql")),
    ("015_report_versions", include_str!("../migrations/015_report_versions.sql")),
    ("016_metric_snapshots", include_str!("../migrations/016_metric_snapshots.sql")),
    ("017_column_profiling", include_str!("../migrations/017_column_profiling.sql")),
    ("018_table_descriptions", include_str!("../migrations/018_table_descriptions.sql")),
    ("019_column_descriptions", include_str!("../migrations/019_column_descriptions.sql")),
    ("020_conversation_generation_status", include_str!("../migrations/020_conversation_generation_status.sql")),
    ("021_email_alerts", include_str!("../migrations/021_email_alerts.sql")),
    ("022_feishu_alerts", include_str!("../migrations/022_feishu_alerts.sql")),
    ("023_report_themes", include_str!("../migrations/023_report_themes.sql")),
    ("024_report_summaries", include_str!("../migrations/024_report_summaries.sql")),
    ("025_resource_ownership", include_str!("../migrations/025_resource_ownership.sql")),
    ("026_datasource_grants", include_str!("../migrations/026_datasource_grants.sql")),
    ("027_github_oauth", include_str!("../migrations/027_github_oauth.sql")),
    ("028_report_datasource_filters", include_str!("../migrations/028_report_datasource_filters.sql")),
    ("029_report_filters", include_str!("../migrations/029_report_filters.sql")),
    ("030_default_datasource", include_str!("../migrations/030_default_datasource.sql")),
    ("031_metric_params", include_str!("../migrations/031_metric_params.sql")),
    ("032_data_pool_params", include_str!("../migrations/032_data_pool_params.sql")),
    ("033_wishes", include_str!("../migrations/033_wishes.sql")),
];

/// Whether a migration error means "this object was already applied" (safe to
/// tolerate when baselining an existing database on first upgrade) rather than
/// a genuine failure.
///
/// Matches primarily on the MySQL native error number (sqlx's `code()` returns
/// the SQLSTATE, not the 1050/1060/... number), with a SQLSTATE fallback.
fn is_already_applied_error(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .map(|db_err| {
            let mysql_num = db_err
                .as_error()
                .downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                .map(|me| me.number());
            // 1050 table exists, 1060 dup column, 1061 dup key name, 1062 dup
            // entry (seed data), 1068 multiple PK, 1091 can't DROP (object
            // missing), 1826 dup FK.
            matches!(mysql_num, Some(1050 | 1060 | 1061 | 1062 | 1068 | 1091 | 1826))
                // SQLSTATE fallback: 42S01 table exists, 42S21 dup column,
                // 23000 integrity/dup-entry.
                || matches!(db_err.code().unwrap_or_default().as_ref(), "42S01" | "42S21" | "23000")
        })
        .unwrap_or(false)
}

/// Apply the ordered migration manifest with version tracking.
///
/// - A `schema_migrations` table records which versions have run, so each
///   migration is applied exactly once and startup is fast on subsequent boots.
/// - A MySQL named lock serializes migrations across multiple instances.
/// - Existing pre-tracking databases are baselined on first upgrade: statements
///   that fail with an "already applied" error (table/column exists, etc.) are
///   tolerated, but any *other* error aborts startup instead of being silently
///   swallowed — a broken schema should never be served.
async fn run_migrations(pool: &MySqlPool, migrations: &[(&str, &str)]) {
    // Serialize concurrent instances so two servers can't race the same DDL.
    let lock: (Option<i32>,) = sqlx::query_as("SELECT GET_LOCK('lingxibi_migrations', 60)")
        .fetch_one(pool)
        .await
        .expect("Failed to acquire migration lock");
    if lock.0 != Some(1) {
        panic!("Timed out waiting for the migration lock; another instance may be stuck");
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version VARCHAR(255) NOT NULL PRIMARY KEY,
            applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await
    .expect("Failed to create schema_migrations table");

    for (version, sql) in migrations {
        let applied: Option<(String,)> =
            sqlx::query_as("SELECT version FROM schema_migrations WHERE version = ?")
                .bind(version)
                .fetch_optional(pool)
                .await
                .expect("Failed to query schema_migrations");
        if applied.is_some() {
            continue;
        }

        for statement in split_sql_statements(sql) {
            let trimmed = statement.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Err(e) = sqlx::query(trimmed).execute(pool).await {
                if is_already_applied_error(&e) {
                    // Baselining an existing DB: object already present.
                    continue;
                }
                // Genuine failure: release the lock and abort so ops notice.
                let _ = sqlx::query("SELECT RELEASE_LOCK('lingxibi_migrations')")
                    .execute(pool)
                    .await;
                panic!(
                    "Migration '{}' failed: {}\n  statement: {}",
                    version,
                    e,
                    &trimmed[..trimmed.len().min(120)]
                );
            }
        }

        sqlx::query("INSERT IGNORE INTO schema_migrations (version) VALUES (?)")
            .bind(version)
            .execute(pool)
            .await
            .expect("Failed to record migration");
        tracing::info!("Applied migration {}", version);
    }

    let _ = sqlx::query("SELECT RELEASE_LOCK('lingxibi_migrations')")
        .execute(pool)
        .await;
    tracing::info!("Migrations complete ({} tracked)", migrations.len());
}

/// Split SQL on `;` delimiters while respecting single-quoted strings.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut statements = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < len {
        let c = chars[i];

        // Skip block comments
        if c == '/' && i + 1 < len && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }

        // Skip line comments
        if c == '-' && i + 1 < len && chars[i + 1] == '-' {
            i += 2;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Skip single-quoted strings
        if c == '\'' {
            i += 1;
            while i < len {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                } else if chars[i] == '\'' {
                    // MySQL '' escape: two consecutive single quotes inside a string
                    if i + 1 < len && chars[i + 1] == '\'' {
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            continue;
        }

        if c == ';' {
            statements.push(chars[start..i].iter().collect());
            start = i + 1;
        }
        i += 1;
    }

    // Remaining text after last semicolon
    if start < len {
        let remaining: String = chars[start..].iter().collect();
        if !remaining.trim().is_empty() {
            statements.push(remaining);
        }
    }

    statements
}
