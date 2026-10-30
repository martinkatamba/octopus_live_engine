use std::sync::Arc;
use std::time::Duration;

use crate::adapters::outbound::in_memory_store::InMemoryStateStore;
use crate::adapters::outbound::redis_store::RedisStateStore;
use crate::adapters::outbound::relational_store::{
    MySqlDialect, PostgresDialect, RelationalStateStore, SqlDialect, SqlServerDialect,
    SqliteDialect,
};
use crate::adapters::outbound::sql_backend;
use crate::domain::ports::IStateStore;
use crate::json::{Value, ValueMap};

pub struct StoreFactory;

impl StoreFactory {
    pub fn new() -> Self {
        Self
    }

    pub fn create(&self, store_type: &str, config: Option<&ValueMap>) -> Arc<dyn IStateStore> {
        let mut st = store_type.trim().to_lowercase();
        let empty = ValueMap::new();
        let cfg = config.unwrap_or(&empty);

        if st.is_empty() {
            if let Some(s) = cfg
                .get("store")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                st = s.trim().to_lowercase();
            } else if let Some(s) = cfg
                .get("type")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                st = s.trim().to_lowercase();
            } else if let Some(s) = std::env::var("OCTOPUS_STATE_STORE")
                .ok()
                .filter(|s| !s.is_empty())
            {
                st = s.trim().to_lowercase();
            } else {
                st = "inmemory".to_string();
            }
        }

        let pool_size = cfg
            .get("pool_size")
            .and_then(|v| v.as_i64())
            .filter(|n| *n > 0)
            .unwrap_or(0) as usize;

        match st.as_str() {
            "postgres" | "postgresql" | "pgsql" => Self::relational(
                Box::new(PostgresDialect),
                get_dsn(cfg, &["POSTGRES_DSN"]),
                pool_size,
            ),
            "mysql" | "mariadb" => Self::relational(
                Box::new(MySqlDialect),
                get_dsn(cfg, &["MYSQL_DSN"]),
                pool_size,
            ),
            "sqlserver" | "mssql" | "tsql" => Self::relational(
                Box::new(SqlServerDialect),
                get_dsn(cfg, &["SQLSERVER_DSN", "MSSQL_DSN"]),
                pool_size,
            ),
            "sqlite" | "sqlite3" => {
                let path = ["path", "sqlite_path", "db_path"]
                    .iter()
                    .find_map(|k| {
                        cfg.get(*k)
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .map(|s| s.to_string())
                    .or_else(|| get_dsn(cfg, &["SQLITE_PATH"]))
                    .unwrap_or_else(|| DEFAULT_SQLITE_PATH.to_string());
                Self::relational(Box::new(SqliteDialect), Some(path), 1)
            }
            "redis" | "redis_cluster" => {
                let redis_url = get_dsn(cfg, &["REDIS_URL"])
                    .unwrap_or_else(|| "redis://localhost:6379".to_string());
                let prefix = cfg
                    .get("prefix")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("octopus:live");
                let mut ttl_success = Duration::from_secs(3600);
                let mut ttl_failure = Duration::from_secs(7 * 24 * 3600);
                let mut ttl_running = Duration::from_secs(2 * 3600);

                if let Some(Value::Object(ret_map)) = cfg.get("retention") {
                    let secs =
                        |k: &str| ret_map.get(k).and_then(|v| v.as_f64()).filter(|s| *s > 0.0);
                    if let Some(s) = secs("success_ttl_seconds") {
                        ttl_success = Duration::from_secs_f64(s);
                    }
                    if let Some(f) = secs("failure_ttl_seconds") {
                        ttl_failure = Duration::from_secs_f64(f);
                    }
                    if let Some(r) = secs("running_ttl_seconds") {
                        ttl_running = Duration::from_secs_f64(r);
                    }
                }

                match RedisStateStore::new(
                    &redis_url,
                    ttl_success,
                    ttl_failure,
                    ttl_running,
                    prefix,
                ) {
                    Ok(r_store) => r_store,
                    Err(e) => {
                        eprintln!(
                            "⚠️ [STORE FACTORY] Redis at {} unavailable ({}); jobs are kept in memory only",
                            redis_url, e
                        );
                        InMemoryStateStore::new()
                    }
                }
            }
            _ => InMemoryStateStore::new(),
        }
    }

    /// Connects a relational store, or returns a memory-only one (with a
    /// warning) when no DSN is configured or the database can't be reached.
    fn relational(
        dialect: Box<dyn SqlDialect>,
        dsn: Option<String>,
        pool_size: usize,
    ) -> Arc<dyn IStateStore> {
        let name = dialect.name().to_string();
        let Some(dsn) = dsn else {
            eprintln!(
                "⚠️ [STORE FACTORY] store '{}' has no dsn; jobs are kept in memory only",
                name
            );
            return RelationalStateStore::new(Some(dialect));
        };
        let connected = sql_backend::connect(&name, &dsn, pool_size)
            .and_then(|backend| RelationalStateStore::with_backend(dialect, backend));
        match connected {
            Ok(store) => store,
            Err(e) => {
                eprintln!(
                    "⚠️ [STORE FACTORY] {} unavailable ({}); jobs are kept in memory only",
                    name, e
                );
                RelationalStateStore::new(None)
            }
        }
    }
}

/// Where `store: sqlite` writes when no path is configured (same as Python).
pub const DEFAULT_SQLITE_PATH: &str = "data/octopus_state.sqlite";

/// `dsn`, `url` or `connection_string` from the blueprint, else the first set env var.
fn get_dsn(cfg: &ValueMap, env_keys: &[&str]) -> Option<String> {
    ["dsn", "url", "connection_string"]
        .iter()
        .find_map(|k| {
            cfg.get(*k)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .map(|s| s.to_string())
        .or_else(|| {
            env_keys
                .iter()
                .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        })
}

impl Default for StoreFactory {
    fn default() -> Self {
        Self::new()
    }
}
