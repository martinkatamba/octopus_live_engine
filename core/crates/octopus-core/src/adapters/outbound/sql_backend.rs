//! Database drivers behind the relational state store.
//!
//! Each backend speaks the dialect's SQL as written ("?" or "$n" placeholders)
//! and exchanges values as [`SqlValue`], so the store stays driver-agnostic.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const DEFAULT_POOL_SIZE: usize = 8;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound for one statement or transaction where the driver supports it.
#[allow(dead_code)]
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs a blocking connect on its own thread and gives up after `timeout`.
/// Driver timeouts only cover opening the TCP socket; a server (or a port
/// proxy such as Docker's) that accepts and then stalls in the handshake
/// would otherwise block the caller for minutes. On timeout the attempt is
/// abandoned and finishes, or fails, on its own thread.
#[allow(dead_code)]
pub(crate) fn within<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("octopus-db-connect".to_string())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(timeout)
        .unwrap_or_else(|_| Err(format!("connect timeout after {:?}", timeout)))
}

#[derive(Clone, Debug, PartialEq)]
pub enum SqlValue {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
}

impl SqlValue {
    pub fn text(s: &str) -> Self {
        SqlValue::Text(s.to_string())
    }

    pub fn opt_text(s: Option<&str>) -> Self {
        s.map(SqlValue::text).unwrap_or(SqlValue::Null)
    }

    pub fn as_string(&self) -> String {
        match self {
            SqlValue::Null => String::new(),
            SqlValue::Int(i) => i.to_string(),
            SqlValue::Float(f) => f.to_string(),
            SqlValue::Text(s) => s.clone(),
        }
    }

    pub fn as_opt_string(&self) -> Option<String> {
        match self {
            SqlValue::Null => None,
            other => Some(other.as_string()),
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            SqlValue::Int(i) => Some(*i),
            SqlValue::Float(f) => Some(*f as i64),
            SqlValue::Text(s) => s.trim().parse().ok(),
            SqlValue::Null => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SqlValue::Int(i) => Some(*i as f64),
            SqlValue::Float(f) => Some(*f),
            SqlValue::Text(s) => s.trim().parse().ok(),
            SqlValue::Null => None,
        }
    }
}

pub type SqlRow = Vec<SqlValue>;
pub type SqlStatement = (String, Vec<SqlValue>);

pub trait SqlBackend: Send + Sync {
    /// Runs a script of one or more statements without parameters (DDL).
    fn exec_batch(&self, sql: &str) -> Result<(), String>;
    /// Runs the statements in order inside a single transaction.
    fn transaction(&self, statements: &[SqlStatement]) -> Result<(), String>;
    fn query(&self, sql: &str, params: &[SqlValue]) -> Result<Vec<SqlRow>, String>;
}

// ============================================================================
// Connection pool shared by the blocking drivers
// ============================================================================

// Unused only in a build with no database driver compiled in.
#[allow(dead_code)]
pub(crate) struct Pool<C> {
    idle: Mutex<(Vec<C>, usize)>,
    available: Condvar,
    max: usize,
    connect: Box<dyn Fn() -> Result<C, String> + Send + Sync>,
}

#[allow(dead_code)]
impl<C: Send> Pool<C> {
    pub(crate) fn new(
        max: usize,
        connect: Box<dyn Fn() -> Result<C, String> + Send + Sync>,
    ) -> Self {
        Self {
            idle: Mutex::new((Vec::new(), 0)),
            available: Condvar::new(),
            max: max.max(1),
            connect,
        }
    }

    /// Checks out a connection, runs `f`, and returns the connection to the
    /// pool. A connection whose call failed is dropped so the next caller
    /// reconnects instead of reusing a broken socket.
    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut C) -> Result<T, String>) -> Result<T, String> {
        let mut conn = {
            let mut guard = self.idle.lock().unwrap();
            loop {
                if let Some(c) = guard.0.pop() {
                    break c;
                }
                if guard.1 < self.max {
                    guard.1 += 1;
                    drop(guard);
                    match (self.connect)() {
                        Ok(c) => break c,
                        Err(e) => {
                            self.idle.lock().unwrap().1 -= 1;
                            self.available.notify_one();
                            return Err(e);
                        }
                    }
                }
                guard = self.available.wait(guard).unwrap();
            }
        };
        let res = f(&mut conn);
        let mut guard = self.idle.lock().unwrap();
        if res.is_ok() {
            guard.0.push(conn);
        } else {
            guard.1 -= 1;
        }
        drop(guard);
        self.available.notify_one();
        res
    }
}

#[allow(dead_code)]
fn split_statements(script: &str) -> impl Iterator<Item = &str> {
    script.split(';').map(str::trim).filter(|s| !s.is_empty())
}

/// Rewrites "?" placeholders as "@P1", "@P2", ... for SQL Server.
pub fn number_placeholders(sql: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 16);
    let mut n = 0;
    for ch in sql.chars() {
        if ch == '?' {
            n += 1;
            out.push_str(prefix);
            out.push_str(&n.to_string());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Opens a backend for `dialect` ("postgres", "mysql", "sqlserver", "sqlite").
/// Fails fast when the driver was not compiled in or the server is unreachable.
pub fn connect(dialect: &str, dsn: &str, pool_size: usize) -> Result<Arc<dyn SqlBackend>, String> {
    let pool_size = if pool_size == 0 {
        DEFAULT_POOL_SIZE
    } else {
        pool_size
    };
    match dialect {
        "postgres" => postgres_backend::connect(dsn, pool_size),
        "mysql" => mysql_backend::connect(dsn, pool_size),
        "sqlserver" => mssql_backend::connect(dsn, pool_size),
        "sqlite" => sqlite_backend::connect(dsn),
        other => Err(format!("unsupported SQL dialect '{}'", other)),
    }
}

// ============================================================================
// PostgreSQL (postgres crate)
// ============================================================================

#[cfg(feature = "postgres")]
mod postgres_backend {
    use super::*;
    use postgres::types::{ToSql, Type};
    use postgres::{Client, NoTls, Statement};
    use std::collections::HashMap;

    struct Conn {
        client: Client,
        statements: HashMap<String, Statement>,
    }

    impl Conn {
        fn prepare(&mut self, sql: &str) -> Result<Statement, String> {
            if let Some(st) = self.statements.get(sql) {
                return Ok(st.clone());
            }
            let st = self.client.prepare(sql).map_err(|e| e.to_string())?;
            self.statements.insert(sql.to_string(), st.clone());
            Ok(st)
        }
    }

    struct PostgresBackend {
        pool: Pool<Conn>,
    }

    pub fn connect(dsn: &str, pool_size: usize) -> Result<Arc<dyn SqlBackend>, String> {
        let mut config: postgres::Config =
            dsn.parse().map_err(|e| format!("invalid DSN: {}", e))?;
        if config.get_connect_timeout().is_none() {
            config.connect_timeout(CONNECT_TIMEOUT);
        }
        let connect = Box::new(move || {
            let config = config.clone();
            within(CONNECT_TIMEOUT, move || {
                config
                    .connect(NoTls)
                    .map(|client| Conn {
                        client,
                        statements: HashMap::new(),
                    })
                    .map_err(|e| e.to_string())
            })
        });
        let backend = PostgresBackend {
            pool: Pool::new(pool_size, connect),
        };
        backend.pool.with(|_| Ok(()))?;
        Ok(Arc::new(backend))
    }

    fn bind(ty: &Type, v: &SqlValue) -> Box<dyn ToSql + Sync> {
        match *ty {
            Type::INT2 => Box::new(v.as_i64().map(|i| i as i16)),
            Type::INT4 => Box::new(v.as_i64().map(|i| i as i32)),
            Type::INT8 => Box::new(v.as_i64()),
            Type::FLOAT4 => Box::new(v.as_f64().map(|f| f as f32)),
            Type::FLOAT8 | Type::NUMERIC => Box::new(v.as_f64()),
            _ => Box::new(v.as_opt_string()),
        }
    }

    fn read(row: &postgres::Row, idx: usize) -> SqlValue {
        let ty = row.columns()[idx].type_();
        let v = match *ty {
            Type::INT2 => row
                .get::<_, Option<i16>>(idx)
                .map(|i| SqlValue::Int(i as i64)),
            Type::INT4 => row
                .get::<_, Option<i32>>(idx)
                .map(|i| SqlValue::Int(i as i64)),
            Type::INT8 => row.get::<_, Option<i64>>(idx).map(SqlValue::Int),
            Type::FLOAT4 => row
                .get::<_, Option<f32>>(idx)
                .map(|f| SqlValue::Float(f as f64)),
            Type::FLOAT8 => row.get::<_, Option<f64>>(idx).map(SqlValue::Float),
            _ => row
                .try_get::<_, Option<String>>(idx)
                .ok()
                .flatten()
                .map(SqlValue::Text),
        };
        v.unwrap_or(SqlValue::Null)
    }

    fn run(conn: &mut Conn, sql: &str, params: &[SqlValue]) -> Result<Vec<postgres::Row>, String> {
        let st = conn.prepare(sql)?;
        let bound: Vec<Box<dyn ToSql + Sync>> = st
            .params()
            .iter()
            .zip(params)
            .map(|(ty, v)| bind(ty, v))
            .collect();
        let refs: Vec<&(dyn ToSql + Sync)> = bound.iter().map(|b| b.as_ref()).collect();
        conn.client.query(&st, &refs).map_err(|e| e.to_string())
    }

    impl SqlBackend for PostgresBackend {
        fn exec_batch(&self, sql: &str) -> Result<(), String> {
            self.pool
                .with(|c| c.client.batch_execute(sql).map_err(|e| e.to_string()))
        }

        fn transaction(&self, statements: &[SqlStatement]) -> Result<(), String> {
            self.pool.with(|c| {
                c.client.batch_execute("BEGIN").map_err(|e| e.to_string())?;
                for (sql, params) in statements {
                    if let Err(e) = run(c, sql, params) {
                        let _ = c.client.batch_execute("ROLLBACK");
                        return Err(e);
                    }
                }
                c.client.batch_execute("COMMIT").map_err(|e| e.to_string())
            })
        }

        fn query(&self, sql: &str, params: &[SqlValue]) -> Result<Vec<SqlRow>, String> {
            self.pool.with(|c| {
                let rows = run(c, sql, params)?;
                Ok(rows
                    .iter()
                    .map(|r| (0..r.len()).map(|i| read(r, i)).collect())
                    .collect())
            })
        }
    }
}

#[cfg(not(feature = "postgres"))]
mod postgres_backend {
    use super::*;
    pub fn connect(_dsn: &str, _pool_size: usize) -> Result<Arc<dyn SqlBackend>, String> {
        Err("the Postgres driver is not compiled in (enable the `postgres` feature)".to_string())
    }
}

// ============================================================================
// MySQL / MariaDB (mysql crate, which pools connections itself)
// ============================================================================

#[cfg(feature = "mysql")]
mod mysql_backend {
    use super::*;
    use mysql::prelude::Queryable;
    use mysql::{Opts, OptsBuilder, PoolConstraints, PoolOpts, TxOpts, Value};

    struct MySqlBackend {
        pool: mysql::Pool,
    }

    /// Accepts "mysql://user:pass@host:port/db" and the Go driver form
    /// "user:pass@tcp(host:port)/db?parseTime=true".
    pub fn normalize_dsn(raw: &str) -> String {
        let raw = raw.trim();
        let url = if raw.starts_with("mysql://") || raw.starts_with("mariadb://") {
            raw.replacen("mariadb://", "mysql://", 1)
        } else if let Some((user, rest)) = raw.split_once("@tcp(") {
            match rest.split_once(')') {
                Some((host, tail)) => format!("mysql://{}@{}{}", user, host, tail),
                None => format!("mysql://{}", raw),
            }
        } else {
            format!("mysql://{}", raw)
        };
        // Drop options the Rust driver doesn't understand (e.g. parseTime).
        match url.split_once('?') {
            Some((base, query)) => {
                let kept: Vec<&str> = query
                    .split('&')
                    .filter(|kv| {
                        let key = kv.split('=').next().unwrap_or("");
                        !matches!(key, "parseTime" | "charset" | "loc" | "timeout")
                    })
                    .collect();
                if kept.is_empty() {
                    base.to_string()
                } else {
                    format!("{}?{}", base, kept.join("&"))
                }
            }
            None => url,
        }
    }

    pub fn connect(dsn: &str, pool_size: usize) -> Result<Arc<dyn SqlBackend>, String> {
        let opts =
            Opts::from_url(&normalize_dsn(dsn)).map_err(|e| format!("invalid DSN: {}", e))?;
        let constraints = PoolConstraints::new(0, pool_size).unwrap_or_default();
        // Read/write timeouts bound the handshake and every later query, so a
        // stalled server fails a call instead of hanging it.
        let builder = OptsBuilder::from_opts(opts)
            .tcp_connect_timeout(Some(CONNECT_TIMEOUT))
            .read_timeout(Some(QUERY_TIMEOUT))
            .write_timeout(Some(QUERY_TIMEOUT))
            .pool_opts(PoolOpts::default().with_constraints(constraints));
        let pool = within(CONNECT_TIMEOUT, move || {
            let pool = mysql::Pool::new(builder).map_err(|e| e.to_string())?;
            // get_conn() keeps retrying a failing server; try_get_conn gives up.
            pool.try_get_conn(CONNECT_TIMEOUT)
                .map_err(|e| e.to_string())?;
            Ok(pool)
        })?;
        Ok(Arc::new(MySqlBackend { pool }))
    }

    fn to_mysql(v: &SqlValue) -> Value {
        match v {
            SqlValue::Null => Value::NULL,
            SqlValue::Int(i) => Value::Int(*i),
            SqlValue::Float(f) => Value::Double(*f),
            SqlValue::Text(s) => Value::Bytes(s.as_bytes().to_vec()),
        }
    }

    fn from_mysql(v: Value) -> SqlValue {
        match v {
            Value::NULL => SqlValue::Null,
            Value::Int(i) => SqlValue::Int(i),
            Value::UInt(u) => SqlValue::Int(u as i64),
            Value::Float(f) => SqlValue::Float(f as f64),
            Value::Double(f) => SqlValue::Float(f),
            Value::Bytes(b) => SqlValue::Text(String::from_utf8_lossy(&b).into_owned()),
            other => SqlValue::Text(other.as_sql(true).trim_matches('\'').to_string()),
        }
    }

    fn params(values: &[SqlValue]) -> mysql::Params {
        if values.is_empty() {
            mysql::Params::Empty
        } else {
            mysql::Params::Positional(values.iter().map(to_mysql).collect())
        }
    }

    impl SqlBackend for MySqlBackend {
        fn exec_batch(&self, sql: &str) -> Result<(), String> {
            let mut conn = self.pool.get_conn().map_err(|e| e.to_string())?;
            for stmt in split_statements(sql) {
                conn.query_drop(stmt).map_err(|e| e.to_string())?;
            }
            Ok(())
        }

        fn transaction(&self, statements: &[SqlStatement]) -> Result<(), String> {
            let mut conn = self.pool.get_conn().map_err(|e| e.to_string())?;
            let mut tx = conn
                .start_transaction(TxOpts::default())
                .map_err(|e| e.to_string())?;
            for (sql, values) in statements {
                tx.exec_drop(sql.as_str(), params(values))
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())
        }

        fn query(&self, sql: &str, values: &[SqlValue]) -> Result<Vec<SqlRow>, String> {
            let mut conn = self.pool.get_conn().map_err(|e| e.to_string())?;
            let rows: Vec<mysql::Row> =
                conn.exec(sql, params(values)).map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|r| r.unwrap().into_iter().map(from_mysql).collect())
                .collect())
        }
    }
}

#[cfg(not(feature = "mysql"))]
mod mysql_backend {
    use super::*;
    pub fn connect(_dsn: &str, _pool_size: usize) -> Result<Arc<dyn SqlBackend>, String> {
        Err("the MySQL driver is not compiled in (enable the `mysql` feature)".to_string())
    }
}

// ============================================================================
// SQLite (rusqlite, bundled)
// ============================================================================

#[cfg(feature = "sqlite")]
mod sqlite_backend {
    use super::*;
    use rusqlite::types::{Value, ValueRef};
    use rusqlite::Connection;

    struct SqliteBackend {
        conn: Mutex<Connection>,
    }

    /// Accepts a plain path, "sqlite://path" or "file:path".
    pub fn normalize_path(dsn: &str) -> String {
        let d = dsn.trim();
        let d = d
            .strip_prefix("sqlite3://")
            .or_else(|| d.strip_prefix("sqlite://"))
            .unwrap_or(d);
        d.split('?').next().unwrap_or(d).to_string()
    }

    pub fn connect(dsn: &str) -> Result<Arc<dyn SqlBackend>, String> {
        let path = normalize_path(dsn);
        if let Some(parent) = std::path::Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        // WAL lets readers proceed while a job is being written.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(None, "foreign_keys", "ON");
        Ok(Arc::new(SqliteBackend {
            conn: Mutex::new(conn),
        }))
    }

    fn to_sqlite(v: &SqlValue) -> Value {
        match v {
            SqlValue::Null => Value::Null,
            SqlValue::Int(i) => Value::Integer(*i),
            SqlValue::Float(f) => Value::Real(*f),
            SqlValue::Text(s) => Value::Text(s.clone()),
        }
    }

    fn from_sqlite(v: ValueRef<'_>) -> SqlValue {
        match v {
            ValueRef::Null => SqlValue::Null,
            ValueRef::Integer(i) => SqlValue::Int(i),
            ValueRef::Real(f) => SqlValue::Float(f),
            ValueRef::Text(t) | ValueRef::Blob(t) => {
                SqlValue::Text(String::from_utf8_lossy(t).into_owned())
            }
        }
    }

    impl SqlBackend for SqliteBackend {
        fn exec_batch(&self, sql: &str) -> Result<(), String> {
            self.conn
                .lock()
                .unwrap()
                .execute_batch(sql)
                .map_err(|e| e.to_string())
        }

        fn transaction(&self, statements: &[SqlStatement]) -> Result<(), String> {
            let mut conn = self.conn.lock().unwrap();
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            for (sql, values) in statements {
                let mut st = tx.prepare_cached(sql).map_err(|e| e.to_string())?;
                let params: Vec<Value> = values.iter().map(to_sqlite).collect();
                st.execute(rusqlite::params_from_iter(params))
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())
        }

        fn query(&self, sql: &str, values: &[SqlValue]) -> Result<Vec<SqlRow>, String> {
            let conn = self.conn.lock().unwrap();
            let mut st = conn.prepare_cached(sql).map_err(|e| e.to_string())?;
            let cols = st.column_count();
            let params: Vec<Value> = values.iter().map(to_sqlite).collect();
            let mut rows = st
                .query(rusqlite::params_from_iter(params))
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let mut r = Vec::with_capacity(cols);
                for i in 0..cols {
                    r.push(from_sqlite(row.get_ref(i).map_err(|e| e.to_string())?));
                }
                out.push(r);
            }
            Ok(out)
        }
    }
}

#[cfg(not(feature = "sqlite"))]
mod sqlite_backend {
    use super::*;
    pub fn connect(_dsn: &str) -> Result<Arc<dyn SqlBackend>, String> {
        Err("the SQLite driver is not compiled in (enable the `sqlite` feature)".to_string())
    }
}

// ============================================================================
// SQL Server (tiberius on a private tokio runtime)
// ============================================================================

#[cfg(feature = "mssql")]
mod mssql_backend {
    use super::*;
    use tiberius::{AuthMethod, Client, ColumnData, Config, Query};
    use tokio::net::TcpStream;
    use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

    type Conn = Client<Compat<TcpStream>>;

    struct MsSqlBackend {
        pool: Pool<Conn>,
        rt: Arc<tokio::runtime::Runtime>,
    }

    /// Builds a tiberius config from an ADO string
    /// ("Server=host,1433;Database=db;User Id=sa;Password=...") or a URL
    /// ("sqlserver://user:pass@host:1433?database=db").
    pub fn parse_config(dsn: &str) -> Result<Config, String> {
        let d = dsn.trim();
        let url = d
            .strip_prefix("sqlserver://")
            .or_else(|| d.strip_prefix("mssql://"));
        let Some(rest) = url else {
            return Config::from_ado_string(d).map_err(|e| e.to_string());
        };
        let (auth_host, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (auth, host) = match auth_host.rsplit_once('@') {
            Some((a, h)) => (Some(a), h),
            None => (None, auth_host),
        };
        let host = host.trim_end_matches('/');
        let (h, port) = match host.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().unwrap_or(1433)),
            None => (host, 1433),
        };
        let mut cfg = Config::new();
        cfg.host(h);
        cfg.port(port);
        if let Some(a) = auth {
            let (u, p) = a.split_once(':').unwrap_or((a, ""));
            cfg.authentication(AuthMethod::sql_server(
                crate::url::url_decode(u),
                crate::url::url_decode(p),
            ));
        }
        let mut trust = true;
        for kv in query.split('&').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            match k.to_ascii_lowercase().as_str() {
                "database" => cfg.database(crate::url::url_decode(v)),
                "trustservercertificate" => trust = !v.eq_ignore_ascii_case("false"),
                "encrypt" if v.eq_ignore_ascii_case("disable") => {
                    cfg.encryption(tiberius::EncryptionLevel::NotSupported)
                }
                _ => {}
            }
        }
        if trust {
            cfg.trust_cert();
        }
        Ok(cfg)
    }

    /// The timeout covers the whole connect, TLS and login included: a
    /// server that accepts TCP and then stalls must not hang the caller.
    async fn open(cfg: Config) -> Result<Conn, String> {
        let connect = async {
            let tcp = TcpStream::connect(cfg.get_addr())
                .await
                .map_err(|e| e.to_string())?;
            tcp.set_nodelay(true).map_err(|e| e.to_string())?;
            Client::connect(cfg, tcp.compat_write())
                .await
                .map_err(|e| e.to_string())
        };
        tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| format!("connect timeout after {:?}", CONNECT_TIMEOUT))?
    }

    pub fn connect(dsn: &str, pool_size: usize) -> Result<Arc<dyn SqlBackend>, String> {
        let cfg = parse_config(dsn)?;
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("octopus-mssql")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?,
        );
        let rt_connect = Arc::clone(&rt);
        let connect = Box::new(move || rt_connect.block_on(open(cfg.clone())));
        let backend = MsSqlBackend {
            pool: Pool::new(pool_size, connect),
            rt,
        };
        backend.pool.with(|_| Ok(()))?;
        Ok(Arc::new(backend))
    }

    fn build<'a>(sql: &str, params: &'a [SqlValue]) -> Query<'a> {
        let mut q = Query::new(number_placeholders(sql, "@P"));
        for p in params {
            match p {
                SqlValue::Null => q.bind(Option::<String>::None),
                SqlValue::Int(i) => q.bind(*i),
                SqlValue::Float(f) => q.bind(*f),
                SqlValue::Text(s) => q.bind(s.as_str()),
            }
        }
        q
    }

    fn from_column(col: ColumnData<'_>) -> SqlValue {
        match col {
            ColumnData::U8(v) => v.map(|i| SqlValue::Int(i as i64)),
            ColumnData::I16(v) => v.map(|i| SqlValue::Int(i as i64)),
            ColumnData::I32(v) => v.map(|i| SqlValue::Int(i as i64)),
            ColumnData::I64(v) => v.map(SqlValue::Int),
            ColumnData::F32(v) => v.map(|f| SqlValue::Float(f as f64)),
            ColumnData::F64(v) => v.map(SqlValue::Float),
            ColumnData::Bit(v) => v.map(|b| SqlValue::Int(b as i64)),
            ColumnData::String(v) => v.map(|s| SqlValue::Text(s.into_owned())),
            _ => None,
        }
        .unwrap_or(SqlValue::Null)
    }

    impl SqlBackend for MsSqlBackend {
        fn exec_batch(&self, sql: &str) -> Result<(), String> {
            self.pool.with(|c| {
                self.rt.block_on(async {
                    c.simple_query(sql)
                        .await
                        .map_err(|e| e.to_string())?
                        .into_results()
                        .await
                        .map_err(|e| e.to_string())
                        .map(|_| ())
                })
            })
        }

        fn transaction(&self, statements: &[SqlStatement]) -> Result<(), String> {
            self.pool.with(|c| {
                self.rt.block_on(async {
                    c.simple_query("BEGIN TRANSACTION")
                        .await
                        .map_err(|e| e.to_string())?
                        .into_results()
                        .await
                        .map_err(|e| e.to_string())?;
                    for (sql, params) in statements {
                        if let Err(e) = build(sql, params).execute(c).await {
                            if let Ok(s) = c.simple_query("ROLLBACK TRANSACTION").await {
                                let _ = s.into_results().await;
                            }
                            return Err(e.to_string());
                        }
                    }
                    c.simple_query("COMMIT TRANSACTION")
                        .await
                        .map_err(|e| e.to_string())?
                        .into_results()
                        .await
                        .map_err(|e| e.to_string())
                        .map(|_| ())
                })
            })
        }

        fn query(&self, sql: &str, params: &[SqlValue]) -> Result<Vec<SqlRow>, String> {
            self.pool.with(|c| {
                self.rt.block_on(async {
                    let rows = build(sql, params)
                        .query(c)
                        .await
                        .map_err(|e| e.to_string())?
                        .into_first_result()
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(rows
                        .into_iter()
                        .map(|r| r.into_iter().map(from_column).collect())
                        .collect())
                })
            })
        }
    }
}

#[cfg(not(feature = "mssql"))]
mod mssql_backend {
    use super::*;
    pub fn connect(_dsn: &str, _pool_size: usize) -> Result<Arc<dyn SqlBackend>, String> {
        Err("the SQL Server driver is not compiled in (enable the `mssql` feature)".to_string())
    }
}

#[cfg(feature = "mssql")]
pub use mssql_backend::parse_config as parse_mssql_config;
#[cfg(feature = "mysql")]
pub use mysql_backend::normalize_dsn as normalize_mysql_dsn;
#[cfg(feature = "sqlite")]
pub use sqlite_backend::normalize_path as normalize_sqlite_path;
