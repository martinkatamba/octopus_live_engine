//! What a host tells a middleware about a request, and what the middleware
//! tells the host to do. The core decides; the host reads the request from its
//! framework and applies the decision.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::OctopusMiddlewareError;
use crate::domain::models::MiddlewareExceptionConfig;
use crate::domain::ports::{IActivityExecutor, IAsyncActivityDispatcher};
use crate::json::{Value, ValueMap};
use crate::vmap;

/// The request facts a middleware decides on.
pub struct RequestMeta<'a> {
    /// Upper case.
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a BTreeMap<String, String>,
    pub query: &'a BTreeMap<String, Vec<String>>,
    /// The client address after the host applied its trusted proxies.
    pub client_ip: &'a str,
    pub body: &'a [u8],
    /// Values stored on the request by earlier middleware and handlers
    /// (`auth_claims`, `job_id`, ...).
    pub context: &'a ValueMap,
}

impl RequestMeta<'_> {
    /// The header's value, matched case-insensitively; empty when absent.
    pub fn header(&self, name: &str) -> String {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    /// The first value of a query parameter; empty when absent.
    pub fn query(&self, key: &str) -> String {
        self.query
            .get(key)
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.context.get(key)
    }
}

/// How the request was answered, for middleware that act afterwards.
pub struct ResponseMeta<'a> {
    pub status: u16,
    pub body: &'a [u8],
    /// From the middleware's `before` to the end of the chain.
    pub duration_ms: f64,
}

/// An answer a middleware gives in place of the handler.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// What the host does with a request after a middleware's `before`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decision {
    /// Response headers to set, whether or not the request goes on.
    pub headers: BTreeMap<String, String>,
    /// Values to store on the request context before going on.
    pub context: ValueMap,
    /// Answer with this and stop the chain; `None` means go on.
    pub respond: Option<Response>,
}

impl Decision {
    /// Go on, changing nothing.
    pub fn next() -> Self {
        Self::default()
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }

    pub fn set(mut self, key: &str, value: Value) -> Self {
        self.context.insert(key.to_string(), value);
        self
    }

    pub fn is_next(&self) -> bool {
        self.respond.is_none()
    }
}

/// A middleware's decisions. Implementations keep their own state (counters,
/// caches) and are shared by every request, so they must be thread-safe.
pub trait Middleware: Send + Sync {
    fn before(&self, req: &RequestMeta) -> Decision;

    /// Runs after the rest of the chain, unless `before` answered. `req.context`
    /// holds what the handler left there.
    fn after(&self, _req: &RequestMeta, _resp: &ResponseMeta) {}
}

/// Turns middleware errors into responses, applying the blueprint's
/// `exceptions:` overrides and running their activities.
#[derive(Clone, Default)]
pub struct ErrorResponder {
    pub exceptions: Vec<MiddlewareExceptionConfig>,
    pub executor: Option<Arc<dyn IActivityExecutor>>,
    pub dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
}

impl ErrorResponder {
    pub fn new(
        exceptions: Vec<MiddlewareExceptionConfig>,
        executor: Option<Arc<dyn IActivityExecutor>>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Self {
        Self {
            exceptions,
            executor,
            dispatcher,
        }
    }

    pub fn reject(&self, req: &RequestMeta, err: OctopusMiddlewareError) -> Decision {
        let mut status_code = err.code;
        let mut message = err.message.clone();
        let mut details = err.details.clone();
        let mut headers = err.headers.clone();

        let matched_cfg = self
            .exceptions
            .iter()
            .find(|ec| ec.exception.eq_ignore_ascii_case(&err.name));

        if let Some(matched) = matched_cfg {
            if matched.code > 0 {
                status_code = matched.code;
            }
            if !matched.message.is_empty() {
                message = matched.message.clone();
            }
            for (k, v) in &matched.headers {
                headers.insert(k.clone(), v.clone());
            }

            if !matched.activity.is_empty() {
                if let Some(exec) = &self.executor {
                    let mut act_payload = vmap! {
                        "exception" => err.name.clone(),
                        "error" => err.name.clone(),
                        "message" => message.clone(),
                        "original_error" => err.message.clone(),
                        "status_code" => status_code as i64,
                        "path" => req.path,
                        "route" => req.path,
                        "method" => req.method,
                        "client_ip" => req.client_ip,
                        "client_host" => req.client_ip,
                        "details" => Value::Object(details.clone()),
                    };
                    if let Some(claims) = req.get("auth_claims") {
                        act_payload.insert("auth_claims".to_string(), claims.clone());
                    }

                    if matched.get_is_async() {
                        if let Some(disp) = &self.dispatcher {
                            let mut job_id = req
                                .get("job_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            for hdr in ["X-Job-ID", "X-Request-ID", "X-Correlation-ID"] {
                                if job_id.is_empty() {
                                    job_id = req.header(hdr);
                                }
                            }
                            if job_id.is_empty() {
                                job_id = format!("sec_audit_{}", unix_nanos());
                            }
                            let step_id = format!("exc_{}", matched.activity.to_lowercase());
                            disp.dispatch(
                                &matched.activity,
                                act_payload,
                                &job_id,
                                &step_id,
                                Arc::clone(exec),
                            );
                        } else {
                            let exec_clone = Arc::clone(exec);
                            let act_name = matched.activity.clone();
                            crate::workers::spawn(move || {
                                let _ = exec_clone.execute_activity(&act_name, &act_payload);
                            });
                        }
                    } else if let Ok(res) = exec.execute_activity(&matched.activity, &act_payload) {
                        for (k, v) in res {
                            if k != "status_code" && k != "code" && k != "message" && k != "detail"
                            {
                                details.insert(k, v);
                            }
                        }
                    }
                }
            }
        }

        let body = Value::Object(vmap! {
            "error" => err.name,
            "detail" => message,
            "status_code" => status_code as i64,
            "details" => Value::Object(details),
        });
        Decision {
            headers,
            context: ValueMap::new(),
            respond: Some(Response {
                status: status_code as u16,
                content_type: JSON_CONTENT_TYPE.to_string(),
                body: body.to_json_string().into_bytes(),
            }),
        }
    }
}

pub const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

pub(crate) fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
