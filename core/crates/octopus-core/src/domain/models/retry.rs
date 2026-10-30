use crate::domain::models::models::WorkflowStep;
use crate::json::Value;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_interval: Duration,
    pub backoff_coefficient: f64,
    pub maximum_interval: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            initial_interval: Duration::from_millis(200),
            backoff_coefficient: 2.0,
            maximum_interval: Duration::ZERO,
        }
    }
}

pub fn step_retry_policy(step: Option<&WorkflowStep>, def: RetryPolicy) -> RetryPolicy {
    let mut rp = def;
    let Some(s) = step else { return rp };
    let Some(ref map) = s.retry_policy else {
        return rp;
    };

    if let Some(v) = map.get("max_attempts") {
        let n = to_int(v);
        if n > 0 {
            rp.max_attempts = n as usize;
        }
    }
    if let Some(v) = map.get("initial_interval_ms") {
        let ms = to_float(v);
        if ms > 0.0 {
            rp.initial_interval = Duration::from_micros((ms * 1000.0) as u64);
        }
    }
    if let Some(v) = map.get("backoff_coefficient") {
        let c = to_float(v);
        if c >= 1.0 {
            rp.backoff_coefficient = c;
        }
    }
    if let Some(v) = map.get("maximum_interval_ms") {
        let ms = to_float(v);
        if ms > 0.0 {
            rp.maximum_interval = Duration::from_micros((ms * 1000.0) as u64);
        }
    }
    rp
}

pub fn to_int(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

pub fn to_float(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}
