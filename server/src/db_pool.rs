use sqlx::mysql::MySqlConnectOptions;
use sqlx::postgres::PgConnectOptions;
use sqlx::{MySqlPool, PgPool};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::DataSource;

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

        let pool = MySqlPool::connect_with(opts)
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

        let pool = PgPool::connect_with(opts)
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
