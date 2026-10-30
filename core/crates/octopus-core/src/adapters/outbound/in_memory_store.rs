use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, RwLock};

use crate::domain::models::{ExecutionState, LiveEvent};
use crate::domain::ports::IStateStore;

pub const DEFAULT_STORED_EXECUTIONS: usize = 1000;

struct StoreInner {
    states: HashMap<String, ExecutionState>,
    order: Vec<String>,
    limit: usize,
    subscribers: HashMap<String, Vec<(u64, SyncSender<LiveEvent>)>>,
}

impl StoreInner {
    fn evict(&mut self) {
        while self.order.len() > self.limit {
            let oldest = self.order.remove(0);
            self.states.remove(&oldest);
        }
    }
}

pub struct InMemoryStateStore {
    inner: Arc<RwLock<StoreInner>>,
    next_sub_id: AtomicU64,
}

impl InMemoryStateStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_limit(&self, n: usize) {
        let mut guard = self.inner.write().unwrap();
        guard.limit = if n == 0 { DEFAULT_STORED_EXECUTIONS } else { n };
        guard.evict();
    }
}

impl Default for InMemoryStateStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StoreInner {
                states: HashMap::new(),
                order: Vec::new(),
                limit: DEFAULT_STORED_EXECUTIONS,
                subscribers: HashMap::new(),
            })),
            next_sub_id: AtomicU64::new(1),
        }
    }
}

impl IStateStore for InMemoryStateStore {
    fn save_state(&self, state: ExecutionState) -> Result<(), String> {
        let mut guard = self.inner.write().unwrap();
        if guard.limit == 0 {
            guard.limit = DEFAULT_STORED_EXECUTIONS;
        }
        let job_id = state.job_id.clone();
        if !guard.states.contains_key(&job_id) {
            guard.order.push(job_id.clone());
        }
        guard.states.insert(job_id, state);
        guard.evict();
        Ok(())
    }

    fn get_state(&self, job_id: &str) -> Result<ExecutionState, String> {
        let guard = self.inner.read().unwrap();
        guard
            .states
            .get(job_id)
            .cloned()
            .ok_or_else(|| format!("state for job '{}' not found", job_id))
    }

    fn list_states(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String> {
        let guard = self.inner.read().unwrap();
        let mut result: Vec<ExecutionState> = guard
            .states
            .values()
            .filter(|s| workflow_name.is_empty() || s.workflow_name == workflow_name)
            .cloned()
            .collect();

        result.sort_by(|a, b| b.started_at.cmp(&a.started_at));

        let lim = if limit == 0 { 100 } else { limit };
        if result.len() > lim {
            result.truncate(lim);
        }
        Ok(result)
    }

    fn publish_event(&self, job_id: &str, event: LiveEvent) -> Result<(), String> {
        let guard = self.inner.read().unwrap();
        if let Some(subs) = guard.subscribers.get(job_id) {
            for (_, tx) in subs {
                let _ = tx.try_send(event.clone());
            }
        }
        Ok(())
    }

    fn has_listeners(&self, job_id: &str) -> bool {
        self.inner.read().unwrap().subscribers.contains_key(job_id)
    }

    fn subscribe_events(
        &self,
        job_id: &str,
    ) -> Result<(Receiver<LiveEvent>, Box<dyn FnOnce() + Send>), String> {
        let mut guard = self.inner.write().unwrap();
        let (tx, rx) = mpsc::sync_channel(50);
        let sub_id = self.next_sub_id.fetch_add(1, Ordering::SeqCst);
        guard
            .subscribers
            .entry(job_id.to_string())
            .or_default()
            .push((sub_id, tx));

        let inner_clone = Arc::clone(&self.inner);
        let jid = job_id.to_string();
        let cleanup = Box::new(move || {
            let mut g = inner_clone.write().unwrap();
            if let Some(subs) = g.subscribers.get_mut(&jid) {
                subs.retain(|(id, _)| *id != sub_id);
                if subs.is_empty() {
                    g.subscribers.remove(&jid);
                }
            }
        });

        Ok((rx, cleanup))
    }
}
