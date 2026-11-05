//! Minimal Redis GET/SET client for the response cache middleware.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long to stop trying Redis after a failure, so a Redis outage costs one
/// connect timeout per window instead of one per request.
#[cfg_attr(not(feature = "redis"), allow(dead_code))]
const RETRY_AFTER: Duration = Duration::from_secs(10);

pub struct RedisKv {
    url: String,
    #[cfg_attr(not(feature = "redis"), allow(dead_code))]
    unavailable_until: Mutex<Option<Instant>>,
    #[cfg(feature = "redis")]
    pool: Option<crate::adapters::outbound::sql_backend::Pool<redis::Connection>>,
}

impl RedisKv {
    /// Accepts "redis://host:port/db", "rediss://..." or a bare "host:port".
    pub fn new(url: &str) -> Self {
        let url = if url.contains("://") {
            url.to_string()
        } else {
            format!("redis://{}", url)
        };
        #[cfg(feature = "redis")]
        let pool = redis::Client::open(url.as_str()).ok().map(|client| {
            crate::adapters::outbound::sql_backend::Pool::new(
                4,
                Box::new(move || {
                    let timeout = Duration::from_secs(1);
                    let c = client
                        .get_connection_with_timeout(timeout)
                        .map_err(|e| e.to_string())?;
                    c.set_read_timeout(Some(timeout))
                        .map_err(|e| e.to_string())?;
                    c.set_write_timeout(Some(timeout))
                        .map_err(|e| e.to_string())?;
                    Ok(c)
                }),
            )
        });
        Self {
            url,
            unavailable_until: Mutex::new(None),
            #[cfg(feature = "redis")]
            pool,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    #[cfg(feature = "redis")]
    fn run<T>(
        &self,
        f: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
    ) -> Result<T, String> {
        let pool = self.pool.as_ref().ok_or("invalid Redis URL")?;
        {
            let mut until = self.unavailable_until.lock().unwrap();
            match *until {
                Some(t) if Instant::now() < t => return Err("redis unavailable".to_string()),
                Some(_) => *until = None,
                None => {}
            }
        }
        let res = pool.with(|c| f(c).map_err(|e| e.to_string()));
        if res.is_err() {
            *self.unavailable_until.lock().unwrap() = Some(Instant::now() + RETRY_AFTER);
        }
        res
    }

    /// Returns the value and its remaining TTL, or `Ok(None)` on a miss.
    pub fn get_with_ttl(&self, key: &str) -> Result<Option<(String, Duration)>, String> {
        #[cfg(feature = "redis")]
        {
            let (val, pttl): (Option<String>, i64) = self.run(|c| {
                redis::pipe()
                    .cmd("GET")
                    .arg(key)
                    .cmd("PTTL")
                    .arg(key)
                    .query(c)
            })?;
            Ok(val.map(|v| (v, Duration::from_millis(pttl.max(0) as u64))))
        }
        #[cfg(not(feature = "redis"))]
        {
            let _ = key;
            Err("the Redis driver is not compiled in".to_string())
        }
    }

    pub fn set_ex(&self, key: &str, value: &str, ttl: Duration) -> Result<(), String> {
        #[cfg(feature = "redis")]
        {
            let secs = ttl.as_secs().max(1);
            self.run(|c| {
                redis::cmd("SET")
                    .arg(key)
                    .arg(value)
                    .arg("EX")
                    .arg(secs)
                    .query::<()>(c)
            })
        }
        #[cfg(not(feature = "redis"))]
        {
            let _ = (key, value, ttl);
            Err("the Redis driver is not compiled in".to_string())
        }
    }
}
