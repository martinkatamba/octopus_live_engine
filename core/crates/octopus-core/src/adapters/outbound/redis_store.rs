use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

use crate::adapters::outbound::in_memory_store::InMemoryStateStore;
use crate::domain::models::{ExecutionState, ExecutionStatus, LiveEvent};
use crate::domain::ports::IStateStore;

/// Redis-backed state store using the Go engine's key layout
/// (`{prefix}:state:{job}`, `{prefix}:traces:{job}`, `{prefix}:events:{job}`)
/// with per-status retention. Live events are fanned out locally and over
/// Redis pub/sub, so an SSE stream on one instance sees jobs run on another.
pub struct RedisStateStore {
    pub redis_url: String,
    pub prefix: String,
    pub ttl_success: Duration,
    pub ttl_failure: Duration,
    pub ttl_running: Duration,
    fallback: Arc<InMemoryStateStore>,
    #[cfg(feature = "redis")]
    conn: Option<backend::Conn>,
    degraded: AtomicBool,
}

impl RedisStateStore {
    /// Connects to `redis_url` and checks it with a PING. Fails when Redis is
    /// unreachable or the `redis` feature is disabled.
    pub fn new(
        redis_url: &str,
        ttl_success: Duration,
        ttl_failure: Duration,
        ttl_running: Duration,
        prefix: &str,
    ) -> Result<Arc<Self>, String> {
        let mut store = Self::unconnected(redis_url, ttl_success, ttl_failure, ttl_running, prefix);
        #[cfg(feature = "redis")]
        {
            let conn = backend::Conn::open(redis_url, &store.prefix, Arc::clone(&store.fallback))?;
            store.conn = Some(conn);
            Ok(Arc::new(store))
        }
        #[cfg(not(feature = "redis"))]
        {
            let _ = &mut store;
            Err("the Redis driver is not compiled in (enable the `redis` feature)".to_string())
        }
    }

    fn unconnected(
        redis_url: &str,
        ttl_success: Duration,
        ttl_failure: Duration,
        ttl_running: Duration,
        prefix: &str,
    ) -> Self {
        let or = |d: Duration, def: u64| {
            if d.is_zero() {
                Duration::from_secs(def)
            } else {
                d
            }
        };
        Self {
            redis_url: redis_url.to_string(),
            prefix: if prefix.is_empty() {
                "octopus:live".to_string()
            } else {
                prefix.to_string()
            },
            ttl_success: or(ttl_success, 3600),
            ttl_failure: or(ttl_failure, 7 * 24 * 3600),
            ttl_running: or(ttl_running, 2 * 3600),
            fallback: InMemoryStateStore::new(),
            #[cfg(feature = "redis")]
            conn: None,
            degraded: AtomicBool::new(false),
        }
    }

    pub fn compute_ttl(&self, status: &ExecutionStatus) -> Duration {
        let st = status.as_str().to_uppercase();
        if st == "FAILED" || st == "ERROR" || st == "TIMEOUT" || st == "ROLLED_BACK" {
            self.ttl_failure
        } else if st == "COMPLETED" || st == "SUCCESS" {
            self.ttl_success
        } else {
            self.ttl_running
        }
    }

    pub fn state_key(&self, job_id: &str) -> String {
        format!("{}:state:{}", self.prefix, job_id)
    }

    pub fn trace_key(&self, job_id: &str) -> String {
        format!("{}:traces:{}", self.prefix, job_id)
    }

    pub fn event_channel(&self, job_id: &str) -> String {
        format!("{}:events:{}", self.prefix, job_id)
    }

    #[allow(dead_code)]
    fn report<T>(&self, op: &str, res: Result<T, String>) -> Result<T, String> {
        match &res {
            Ok(_) => {
                if self.degraded.swap(false, Ordering::Relaxed) {
                    eprintln!(
                        "✅ [REDIS STORE] connection to {} recovered",
                        self.redis_url
                    );
                }
            }
            Err(e) => {
                if !self.degraded.swap(true, Ordering::Relaxed) {
                    eprintln!(
                        "⚠️ [REDIS STORE] {} failed, serving from memory until Redis recovers: {}",
                        op, e
                    );
                }
            }
        }
        res
    }
}

#[cfg(feature = "redis")]
mod backend {
    use super::*;
    use crate::adapters::outbound::sql_backend::Pool;
    use crate::json::{self, Value};
    use redis::{Commands, Connection};

    const IO_TIMEOUT: Duration = Duration::from_secs(5);
    const POOL_SIZE: usize = 8;

    pub(super) struct Conn {
        pool: Pool<Connection>,
        instance_id: String,
    }

    fn connect(client: &redis::Client) -> Result<Connection, String> {
        let conn = client
            .get_connection_with_timeout(IO_TIMEOUT)
            .map_err(|e| e.to_string())?;
        conn.set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|e| e.to_string())?;
        conn.set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|e| e.to_string())?;
        Ok(conn)
    }

    impl Conn {
        pub(super) fn open(
            url: &str,
            prefix: &str,
            local: Arc<InMemoryStateStore>,
        ) -> Result<Self, String> {
            let client =
                redis::Client::open(url).map_err(|e| format!("invalid Redis URL: {}", e))?;
            let pool_client = client.clone();
            let pool = Pool::new(POOL_SIZE, Box::new(move || connect(&pool_client)));
            pool.with(|c| {
                redis::cmd("PING")
                    .query::<String>(c)
                    .map_err(|e| e.to_string())
            })
            .map_err(|e| format!("redis ping failed: {}", e))?;
            let instance_id = crate::crypto::new_job_id();
            spawn_event_listener(client, prefix.to_string(), instance_id.clone(), local);
            Ok(Self { pool, instance_id })
        }

        pub(super) fn with<T>(
            &self,
            f: impl FnOnce(&mut Connection) -> redis::RedisResult<T>,
        ) -> Result<T, String> {
            self.pool.with(|c| f(c).map_err(|e| e.to_string()))
        }
    }

    /// Relays events published by other instances to this process's
    /// subscribers; reconnects with a short pause if the connection drops.
    fn spawn_event_listener(
        client: redis::Client,
        prefix: String,
        instance_id: String,
        local: Arc<InMemoryStateStore>,
    ) {
        let pattern = format!("{}:events:*", prefix);
        let chan_prefix = format!("{}:events:", prefix);
        std::thread::Builder::new()
            .name("octopus-redis-events".to_string())
            .spawn(move || loop {
                if let Ok(mut conn) = client.get_connection_with_timeout(IO_TIMEOUT) {
                    let mut ps = conn.as_pubsub();
                    if ps.psubscribe(&pattern).is_ok() {
                        while let Ok(msg) = ps.get_message() {
                            let Some(job_id) = msg.get_channel_name().strip_prefix(&chan_prefix)
                            else {
                                continue;
                            };
                            let Ok(payload) = msg.get_payload::<String>() else {
                                continue;
                            };
                            let Ok(Value::Object(m)) = json::parse(&payload) else {
                                continue;
                            };
                            if m.get("origin").and_then(|v| v.as_str()) == Some(&instance_id) {
                                continue;
                            }
                            let event = LiveEvent {
                                event: m
                                    .get("event")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("message")
                                    .to_string(),
                                data: m.get("data").cloned().unwrap_or(Value::Null),
                            };
                            let _ = local.publish_event(job_id, event);
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            })
            .expect("spawn redis event listener");
    }

    pub(super) fn parse_state(raw: &str) -> Option<ExecutionState> {
        json::parse(raw)
            .ok()
            .and_then(|v| ExecutionState::from_value(&v))
    }

    impl RedisStateStore {
        pub(super) fn conn(&self) -> &Conn {
            self.conn.as_ref().expect("connected store")
        }

        pub(super) fn redis_save(&self, state: &ExecutionState) -> Result<(), String> {
            let ttl = self.compute_ttl(&state.status).as_secs().max(1);
            let state_json = state.to_value().to_json_string();
            let traces_json = (!state.traces.is_empty()).then(|| {
                Value::Array(state.traces.iter().map(|t| t.to_value()).collect()).to_json_string()
            });
            let (sk, tk) = (self.state_key(&state.job_id), self.trace_key(&state.job_id));
            self.conn().with(|c| {
                let mut p = redis::pipe();
                p.atomic().set_ex(&sk, state_json, ttl).ignore();
                if let Some(t) = traces_json {
                    p.set_ex(&tk, t, ttl).ignore();
                }
                p.query::<()>(c)
            })
        }

        pub(super) fn redis_get(&self, job_id: &str) -> Result<Option<ExecutionState>, String> {
            let keys = [self.state_key(job_id), self.trace_key(job_id)];
            let vals: Vec<Option<String>> = self.conn().with(|c| c.mget(&keys[..]))?;
            let Some(mut state) = vals
                .first()
                .cloned()
                .flatten()
                .and_then(|s| parse_state(&s))
            else {
                return Ok(None);
            };
            if let Some(Ok(Value::Array(arr))) =
                vals.get(1).cloned().flatten().map(|s| json::parse(&s))
            {
                let traces: Vec<_> = arr
                    .iter()
                    .filter_map(crate::domain::models::StepExecutionTrace::from_value)
                    .collect();
                if !traces.is_empty() {
                    state.traces = traces;
                }
            }
            Ok(Some(state))
        }

        /// SCAN (never KEYS, which blocks Redis) + batched MGET.
        pub(super) fn redis_list(
            &self,
            workflow_name: &str,
            limit: usize,
        ) -> Result<Vec<ExecutionState>, String> {
            let pattern = format!("{}:state:*", self.prefix);
            let keys: Vec<String> = self.conn().with(|c| {
                let mut out = Vec::new();
                let mut cursor = 0u64;
                loop {
                    let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                        .arg(cursor)
                        .arg("MATCH")
                        .arg(&pattern)
                        .arg("COUNT")
                        .arg(1000)
                        .query(c)?;
                    out.extend(batch);
                    if next == 0 {
                        break Ok(out);
                    }
                    cursor = next;
                }
            })?;
            let mut states = Vec::new();
            for chunk in keys.chunks(500) {
                let vals: Vec<Option<String>> = self.conn().with(|c| c.mget(chunk))?;
                states.extend(
                    vals.into_iter()
                        .flatten()
                        .filter_map(|s| parse_state(&s))
                        .filter(|s| workflow_name.is_empty() || s.workflow_name == workflow_name),
                );
            }
            states.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            states.truncate(limit);
            Ok(states)
        }

        pub(super) fn redis_publish(&self, job_id: &str, event: &LiveEvent) -> Result<(), String> {
            let mut m = crate::json::ValueMap::new();
            m.insert("event".to_string(), Value::String(event.event.clone()));
            m.insert("data".to_string(), event.data.clone());
            m.insert(
                "origin".to_string(),
                Value::String(self.conn().instance_id.clone()),
            );
            let payload = Value::Object(m).to_json_string();
            let channel = self.event_channel(job_id);
            self.conn()
                .with(|c| {
                    redis::cmd("PUBLISH")
                        .arg(&channel)
                        .arg(payload)
                        .query::<i64>(c)
                })
                .map(|_| ())
        }
    }
}

impl IStateStore for RedisStateStore {
    fn save_state(&self, state: ExecutionState) -> Result<(), String> {
        #[cfg(feature = "redis")]
        if self.conn.is_some() {
            let res = self.report("save", self.redis_save(&state));
            self.fallback.save_state(state)?;
            return res;
        }
        self.fallback.save_state(state)
    }

    fn get_state(&self, job_id: &str) -> Result<ExecutionState, String> {
        #[cfg(feature = "redis")]
        if self.conn.is_some() {
            // A miss falls through: the job may only be in memory if it was
            // saved while Redis was down.
            if let Ok(Some(st)) = self.report("read", self.redis_get(job_id)) {
                return Ok(st);
            }
        }
        self.fallback.get_state(job_id)
    }

    fn list_states(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String> {
        let limit = if limit == 0 { 100 } else { limit };
        #[cfg(feature = "redis")]
        if self.conn.is_some() {
            if let Ok(states) = self.report("list", self.redis_list(workflow_name, limit)) {
                return Ok(states);
            }
        }
        self.fallback.list_states(workflow_name, limit)
    }

    fn publish_event(&self, job_id: &str, event: LiveEvent) -> Result<(), String> {
        #[cfg(feature = "redis")]
        if self.conn.is_some() {
            let _ = self.report("publish", self.redis_publish(job_id, &event));
        }
        self.fallback.publish_event(job_id, event)
    }

    fn has_listeners(&self, job_id: &str) -> bool {
        #[cfg(feature = "redis")]
        if self.conn.is_some() {
            return true;
        }
        self.fallback.has_listeners(job_id)
    }

    fn subscribe_events(
        &self,
        job_id: &str,
    ) -> Result<(Receiver<LiveEvent>, Box<dyn FnOnce() + Send>), String> {
        self.fallback.subscribe_events(job_id)
    }
}
