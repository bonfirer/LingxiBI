use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor, MySqlPool, PgPool};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::DataSource;
use crate::routes::query::QUERY_TIMEOUT_SECS;

/// Maximum connections per data source pool.
const MAX_CONNECTIONS: u32 = 10;

/// Per-connection session settings applied to every **data source** pool.
///
/// This is the database-level half of the read-only guarantee. `validate_sql`
/// inspects statement text, which is a best-effort layer: a hand-written lexer
/// can always disagree with the server's own parser, and it says nothing about
/// what a function like `pg_read_file()` or `dblink_exec()` does. Asking the
/// server to refuse writes outright does not depend on out-parsing it.
///
/// Applied at connect time rather than per query, so it covers every path that
/// uses these pools (ad-hoc queries, metric refreshes, introspection, column
/// profiling, the snapshot and alert schedulers) with no risk of leaking an open
/// transaction back into the pool.
///
/// Best-effort by design: a server too old to understand a statement logs a
/// warning instead of breaking the data source. The real guarantee is still a
/// read-only database account, which is what `.env.example` tells operators to
/// use — this makes a misconfigured account much less dangerous.
///
/// NOTE: only data source pools get this. The application's own metadata pool
/// (built in `main.rs`) must stay writable.
fn statement_timeout_ms() -> u64 {
    QUERY_TIMEOUT_SECS * 1000
}

/// Cached connection pools for user data sources, keyed by datasource ID.
/// This avoids creating a fresh pool on every query — pools are reused
/// and lazily evicted on connection test failure or explicit removal.
#[derive(Clone, Default)]
pub struct PoolCache {
    inner: Arc<RwLock<HashMap<i32, PoolCacheEntry>>>,
}

enum PoolCacheEntry {
    Mysql(MySqlPool),
    Postgres(PgPool),
    Oracle(oracle::pool::Pool),
}

impl PoolCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a MySQL pool for the given datasource.
    pub async fn get_mysql(&self, ds: &DataSource) -> Result<MySqlPool, String> {
        // Fast path: read from cache
        {
            let cache = self.inner.read().await;
            if let Some(PoolCacheEntry::Mysql(pool)) = cache.get(&ds.id) {
                if !pool.is_closed() {
                    return Ok(pool.clone());
                }
            }
        }

        // Slow path: create new pool
        let opts = MySqlConnectOptions::new()
            .host(&ds.host)
            .port(ds.port as u16)
            .username(&ds.username)
            .password(&crate::crypto::decrypt(&ds.password))
            .database(&ds.database_name);

        let pool = MySqlPoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // Read-only for every transaction on this connection. With
                    // autocommit on, each statement is its own transaction, so a
                    // write is rejected by the server (ER_CANT_EXECUTE_IN_READ_ONLY
                    // _TRANSACTION). MySQL 5.6+ / MariaDB 10.0+.
                    if let Err(e) = conn.execute("SET SESSION TRANSACTION READ ONLY").await {
                        tracing::warn!(
                            "Data source connection could not be set READ ONLY ({}). \
                             Queries fall back to text-level validation only — use a \
                             SELECT-only database account for this data source.",
                            e
                        );
                    }
                    // Server-side cap so a slow query actually stops server-side.
                    // The app's own timeout only abandons the future; it cannot
                    // cancel work already running on the server.
                    // `max_execution_time` is MySQL 5.7.8+; MariaDB spells it
                    // `max_statement_time` (seconds). Try both, ignore mismatches.
                    let ms = super::db_pool::statement_timeout_ms();
                    let _ = conn
                        .execute(&*format!("SET SESSION max_execution_time = {}", ms))
                        .await;
                    let _ = conn
                        .execute(&*format!(
                            "SET SESSION max_statement_time = {}",
                            ms as f64 / 1000.0
                        ))
                        .await;
                    Ok(())
                })
            })
            .connect_with(opts)
            .await
            .map_err(|e| format!("MySQL connection failed: {}", e))?;

        let mut cache = self.inner.write().await;
        cache.insert(ds.id, PoolCacheEntry::Mysql(pool.clone()));
        Ok(pool)
    }

    /// Get or create a PostgreSQL pool for the given datasource.
    pub async fn get_postgres(&self, ds: &DataSource) -> Result<PgPool, String> {
        {
            let cache = self.inner.read().await;
            if let Some(PoolCacheEntry::Postgres(pool)) = cache.get(&ds.id) {
                if !pool.is_closed() {
                    return Ok(pool.clone());
                }
            }
        }

        let opts = PgConnectOptions::new()
            .host(&ds.host)
            .port(ds.port as u16)
            .username(&ds.username)
            .password(&crate::crypto::decrypt(&ds.password))
            .database(&ds.database_name);

        let pool = PgPoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // Every transaction on this connection starts read-only, so a
                    // write raises "cannot execute ... in a read-only transaction".
                    // This also blocks the write-capable *functions* the text
                    // validator doesn't know about (lo_import, dblink_exec, ...).
                    // Note `SET` itself is rejected by validate_sql, so a query
                    // cannot turn this back off.
                    if let Err(e) = conn.execute("SET default_transaction_read_only = on").await {
                        tracing::warn!(
                            "Data source connection could not be set READ ONLY ({}). \
                             Queries fall back to text-level validation only — use a \
                             SELECT-only database account for this data source.",
                            e
                        );
                    }
                    let ms = super::db_pool::statement_timeout_ms();
                    let _ = conn
                        .execute(&*format!("SET statement_timeout = {}", ms))
                        .await;
                    // Don't let an abandoned query hold a snapshot open forever.
                    let _ = conn
                        .execute("SET idle_in_transaction_session_timeout = 60000")
                        .await;
                    Ok(())
                })
            })
            .connect_with(opts)
            .await
            .map_err(|e| format!("PostgreSQL connection failed: {}", e))?;

        let mut cache = self.inner.write().await;
        cache.insert(ds.id, PoolCacheEntry::Postgres(pool.clone()));
        Ok(pool)
    }

    /// Get or create an Oracle session pool for the given datasource.
    /// The pool is built on a blocking thread since the oracle crate is synchronous.
    pub async fn get_oracle(&self, ds: &DataSource) -> Result<oracle::pool::Pool, String> {
        // Fast path: read from cache
        {
            let cache = self.inner.read().await;
            if let Some(PoolCacheEntry::Oracle(pool)) = cache.get(&ds.id) {
                return Ok(pool.clone());
            }
        }

        // Slow path: build a new session pool (blocking)
        let conn_str = format!("//{}:{}/{}", ds.host, ds.port, ds.database_name);
        let username = ds.username.clone();
        let password = crate::crypto::decrypt(&ds.password);

        let pool = tokio::task::spawn_blocking(move || {
            oracle::pool::PoolBuilder::new(username, password, conn_str)
                .min_connections(0)
                .max_connections(10)
                .build()
                .map_err(|e| format!("Oracle pool build failed: {}", e))
        })
        .await
        .map_err(|e| format!("Oracle pool spawn failed: {}", e))??;

        let mut cache = self.inner.write().await;
        cache.insert(ds.id, PoolCacheEntry::Oracle(pool.clone()));
        Ok(pool)
    }

    /// Evict a cached pool (call when datasource config changes or on connection error).
    pub async fn evict(&self, ds_id: i32) {
        let mut cache = self.inner.write().await;
        if let Some(entry) = cache.remove(&ds_id) {
            match entry {
                PoolCacheEntry::Mysql(pool) => pool.close().await,
                PoolCacheEntry::Postgres(pool) => pool.close().await,
                PoolCacheEntry::Oracle(pool) => {
                    // Oracle pool close is blocking
                    let _ = tokio::task::spawn_blocking(move || {
                        let _ = pool.close(&oracle::pool::CloseMode::Default);
                    })
                    .await;
                }
            }
        }
    }

    /// Remove all cached pools and close them.
    pub async fn clear(&self) {
        let mut cache = self.inner.write().await;
        for (_, entry) in cache.drain() {
            match entry {
                PoolCacheEntry::Mysql(pool) => pool.close().await,
                PoolCacheEntry::Postgres(pool) => pool.close().await,
                PoolCacheEntry::Oracle(pool) => {
                    let _ = tokio::task::spawn_blocking(move || {
                        let _ = pool.close(&oracle::pool::CloseMode::Default);
                    })
                    .await;
                }
            }
        }
    }
}
