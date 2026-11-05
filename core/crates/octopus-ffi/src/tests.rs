//! The C ABI used the way a host uses it: through the exported functions,
//! with `extern "C"` callbacks and NUL-terminated strings.

use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use octopus_core::json::{parse, Value};

const BLUEPRINT: &str = r#"
workflow_name: greet
version: "1.0.0"
middlewares:
  - name: ApiKeyAuthMiddleware
    config:
      api_keys: [k1]
api:
  - route: /api/greet/{name}
    method: GET
    steps:
      - id: hello
        activity: Hello
        is_output: true
steps:
  - id: hello
    activity: Hello
    is_output: true
"#;

/// Refused at registration: the rule does not parse.
const BAD_RULE: &str = "workflow_name: bad\nsteps:\n  - id: a\n    activity: A\n    condition:\n      rule: \"amount >>> 5 orr\"\n";

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn take(buf: OctopusBuf) -> String {
    let text = unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(buf.ptr, buf.len)) }
        .into_owned();
    unsafe { octopus_buf_free(buf) };
    text
}

fn take_json(buf: OctopusBuf) -> Value {
    parse(&take(buf)).unwrap()
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.as_object()
        .and_then(|m| m.get(key))
        .unwrap_or(&Value::Null)
}

fn last_error() -> Value {
    take_json(octopus_last_error())
}

/// Greets `name`; fails for "nobody". `user_data` counts the calls.
unsafe extern "C" fn hello(
    user_data: *mut c_void,
    input: *const u8,
    input_len: usize,
    out: *mut OctopusOut,
) -> i32 {
    (*(user_data as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
    let input =
        parse(std::str::from_utf8(std::slice::from_raw_parts(input, input_len)).unwrap()).unwrap();
    let name = field(&input, "name").as_str().unwrap_or("?").to_string();
    let (status, text) = if name == "nobody" {
        (1, "no one to greet".to_string())
    } else {
        (
            OCTOPUS_OK,
            format!(r#"{{"greeting": "hi {}", "secret": "s"}}"#, name),
        )
    };
    octopus_out_write(out, text.as_ptr(), text.len());
    status
}

fn new_engine(calls: &AtomicUsize) -> *mut OctopusEngine {
    unsafe {
        let engine =
            octopus_engine_new(c(r#"{"options": {"allow_client_job_ids": true}}"#).as_ptr());
        assert!(!engine.is_null());
        assert_eq!(
            octopus_register_blueprint(engine, c(BLUEPRINT).as_ptr()),
            OCTOPUS_OK
        );
        let ud = calls as *const AtomicUsize as *mut c_void;
        assert_eq!(
            octopus_register_activity(engine, c("Hello").as_ptr(), Some(hello), ud),
            OCTOPUS_OK
        );
        engine
    }
}

#[test]
fn version_round_trips_through_a_buffer() {
    assert_eq!(take(octopus_core_version()), octopus_core::VERSION);
    assert_eq!(octopus_contract_version(), octopus_core::CONTRACT_VERSION);
    assert_eq!(take(octopus_last_error()), "");
}

#[test]
fn executes_with_a_host_activity_and_reads_the_job_back() {
    let calls = AtomicUsize::new(0);
    let engine = new_engine(&calls);
    unsafe {
        let mut out = OctopusBuf::from_vec(Vec::new());
        let status = octopus_execute(
            engine,
            c("greet").as_ptr(),
            c(r#"{"name": "Ada"}"#).as_ptr(),
            &mut out,
        );
        assert_eq!(status, OCTOPUS_OK);
        let state = take_json(out);
        assert_eq!(field(&state, "status").as_str(), Some("COMPLETED"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let job_id = c(field(&state, "job_id").as_str().unwrap());
        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_get_state(engine, job_id.as_ptr(), &mut out),
            OCTOPUS_OK
        );
        let stored = take_json(out);
        assert_eq!(
            field(field(&stored, "data"), "greeting").as_str(),
            Some("hi Ada")
        );

        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_list_states(engine, std::ptr::null(), 10, &mut out),
            OCTOPUS_OK
        );
        assert_eq!(take_json(out).as_array().map(|a| a.len()), Some(1));

        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_compile_diagram(engine, c("greet").as_ptr(), job_id.as_ptr(), &mut out),
            OCTOPUS_OK
        );
        assert!(field(&take_json(out), "nodes").as_array().is_some());
        octopus_engine_free(engine);
    }
}

#[test]
fn failures_set_a_status_and_the_last_error() {
    let calls = AtomicUsize::new(0);
    let engine = new_engine(&calls);
    unsafe {
        let mut out = OctopusBuf::from_vec(Vec::new());
        let status = octopus_execute(
            engine,
            c("greet").as_ptr(),
            c(r#"{"name": "nobody"}"#).as_ptr(),
            &mut out,
        );
        assert_eq!(status, OCTOPUS_ERR_WORKFLOW);
        assert_eq!(field(&take_json(out), "status").as_str(), Some("FAILED"));
        let err = last_error();
        assert_eq!(
            field(&err, "code"),
            &Value::Int(OCTOPUS_ERR_WORKFLOW as i64)
        );
        assert!(field(&err, "message")
            .as_str()
            .unwrap()
            .contains("no one to greet"));

        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_execute(
                engine,
                c("greet").as_ptr(),
                c("{not json").as_ptr(),
                &mut out
            ),
            OCTOPUS_ERR_ARGUMENT
        );
        assert_eq!(
            field(&last_error(), "type").as_str(),
            Some("InvalidArgument")
        );
        assert_eq!(
            octopus_get_state(engine, c("nope").as_ptr(), &mut out),
            OCTOPUS_ERR_NOT_FOUND
        );
        assert_eq!(
            octopus_register_blueprint(engine, c(BAD_RULE).as_ptr()),
            OCTOPUS_ERR_ENGINE
        );
        assert_eq!(
            octopus_execute(
                std::ptr::null(),
                c("greet").as_ptr(),
                std::ptr::null(),
                &mut out
            ),
            OCTOPUS_ERR_ARGUMENT
        );
        assert!(octopus_engine_new(c("[1").as_ptr()).is_null());
        octopus_engine_free(engine);
    }
}

#[test]
fn declared_routes_answer_with_the_route_output() {
    let calls = AtomicUsize::new(0);
    let engine = new_engine(&calls);
    unsafe {
        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(octopus_declared_routes(engine, &mut out), OCTOPUS_OK);
        let routes = take_json(out);
        assert_eq!(
            field(&routes.as_array().unwrap()[0], "route").as_str(),
            Some("/api/greet/{name}")
        );

        let req = r#"{"payload": {"name": "Bo"}, "job_id_candidates": ["job-7"], "client_ip": "10.1.1.1"}"#;
        let mut out = OctopusBuf::from_vec(Vec::new());
        let status = octopus_call_route(
            engine,
            c("get").as_ptr(),
            c("/api/greet/{name}").as_ptr(),
            c(req).as_ptr(),
            &mut out,
        );
        assert_eq!(status, OCTOPUS_OK);
        let answer = take_json(out);
        assert_eq!(field(&answer, "status"), &Value::Int(200));
        assert_eq!(
            field(field(&answer, "headers"), "X-Job-ID").as_str(),
            Some("job-7")
        );
        assert_eq!(
            field(field(&answer, "body"), "greeting").as_str(),
            Some("hi Bo")
        );

        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_call_route(
                engine,
                c("GET").as_ptr(),
                c("/nope").as_ptr(),
                std::ptr::null(),
                &mut out
            ),
            OCTOPUS_ERR_NOT_FOUND
        );
        octopus_engine_free(engine);
    }
}

#[test]
fn middleware_and_access_decisions() {
    let calls = AtomicUsize::new(0);
    let engine = new_engine(&calls);
    unsafe {
        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(octopus_middleware_chain(engine, &mut out), OCTOPUS_OK);
        let chain = take_json(out);
        assert_eq!(
            field(&chain.as_array().unwrap()[0], "built_in"),
            &Value::Bool(true)
        );

        let denied =
            r#"{"method": "GET", "path": "/api/greet/x", "headers": {"X-API-Key": "bad"}}"#;
        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_middleware_before(engine, 0, c(denied).as_ptr(), &mut out),
            OCTOPUS_OK
        );
        let d = take_json(out);
        assert_eq!(field(field(&d, "respond"), "status"), &Value::Int(401));

        let allowed =
            r#"{"method": "GET", "path": "/api/greet/x", "headers": {"x-api-key": "k1"}}"#;
        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_middleware_before(engine, 0, c(allowed).as_ptr(), &mut out),
            OCTOPUS_OK
        );
        assert_eq!(field(&take_json(out), "respond"), &Value::Null);
        assert_eq!(
            octopus_middleware_after(
                engine,
                0,
                c(allowed).as_ptr(),
                c(r#"{"status": 200}"#).as_ptr()
            ),
            OCTOPUS_OK
        );
        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_middleware_before(engine, 9, c(allowed).as_ptr(), &mut out),
            OCTOPUS_ERR_NOT_FOUND
        );

        let mut out = OctopusBuf::from_vec(Vec::new());
        let req = r#"{"has_authorizer": false, "client_is_loopback": false}"#;
        assert_eq!(
            octopus_tooling_decision(engine, c(req).as_ptr(), &mut out),
            OCTOPUS_OK
        );
        assert_eq!(field(&take_json(out), "decision").as_str(), Some("deny"));
        assert!(octopus_api_key_allows(
            engine,
            std::ptr::null(),
            std::ptr::null()
        ));

        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_visualizer_html(c("/api").as_ptr(), c("").as_ptr(), &mut out),
            OCTOPUS_OK
        );
        assert!(take(out).contains("<html"));
        octopus_engine_free(engine);
    }
}

/// Counts events; `user_data` is an `AtomicUsize`.
unsafe extern "C" fn count_event(user_data: *mut c_void, _event: *const u8, _len: usize) {
    (*(user_data as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
}

#[test]
fn live_events_reach_a_subscriber() {
    let calls = AtomicUsize::new(0);
    let events = AtomicUsize::new(0);
    let engine = new_engine(&calls);
    unsafe {
        let job = r#"{"name": "Ev", "job_id": "job-ev"}"#;
        let id = octopus_subscribe_events(
            engine,
            c("job-ev").as_ptr(),
            Some(count_event),
            &events as *const AtomicUsize as *mut c_void,
        );
        assert!(id > 0);
        let mut out = OctopusBuf::from_vec(Vec::new());
        assert_eq!(
            octopus_execute(engine, c("greet").as_ptr(), c(job).as_ptr(), &mut out),
            OCTOPUS_OK
        );
        let state = take_json(out);
        assert_eq!(field(&state, "job_id").as_str(), Some("job-ev"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while events.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(events.load(Ordering::SeqCst) > 0);
        octopus_unsubscribe(engine, id);
        octopus_engine_free(engine);
    }
}
