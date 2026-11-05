use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::domain::models::ExecutionState;

struct CacheEntry {
    state: ExecutionState,
    expires_at: Instant,
}

struct InnerCache {
    store: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
}

pub struct IdempotencyManager {
    inner: Mutex<InnerCache>,
    max_entries: usize,
    ttl: Duration,
}

impl IdempotencyManager {
    pub fn new(max_entries: usize, ttl: Duration) -> Self {
        let max_entries = if max_entries == 0 { 10000 } else { max_entries };
        let ttl = if ttl.is_zero() {
            Duration::from_secs(3600)
        } else {
            ttl
        };
        Self {
            inner: Mutex::new(InnerCache {
                store: HashMap::new(),
                order: VecDeque::new(),
            }),
            max_entries,
            ttl,
        }
    }

    pub fn get(&self, key: &str) -> Option<ExecutionState> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.store.get(key) {
            if Instant::now() > entry.expires_at {
                inner.store.remove(key);
                inner.order.retain(|k| k != key);
                return None;
            }
            return Some(entry.state.clone());
        }
        None
    }

    pub fn set(&self, key: &str, state: ExecutionState) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.store.contains_key(key) {
            while inner.order.len() >= self.max_entries {
                if let Some(oldest) = inner.order.pop_front() {
                    inner.store.remove(&oldest);
                } else {
                    break;
                }
            }
            inner.order.push_back(key.to_string());
        }
        inner.store.insert(
            key.to_string(),
            CacheEntry {
                state,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    pub fn invalidate(&self, key: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.store.remove(key);
        inner.order.retain(|k| k != key);
    }
}
