use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use octopus_live_engine::{vmap, HttpRequest, OctopusLiveEngine, Router, ValueMap};

static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(1);

fn new_keyed_engine(keys_block: &str) -> Router {
    let id = NEXT_DIR_ID.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("octopus_test_apikey_{}_{}", std::process::id(), id));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let bp = format!(
        "workflow_name: keyed\nversion: \"1.0.0\"\nmiddlewares:\n  - name: ApiKeyAuthMiddleware\n{}    excludes: [/health]\napi:\n  - route: /api/v1.0.0/keyed/ping\n    method: GET\n    steps:\n      - id: ping\n        activity: Ping\n        is_output: true\n",
        keys_block
    );
    fs::write(dir.join("keyed.yaml"), bp).unwrap();

    let mut engine = OctopusLiveEngine::new("");
    engine.register_activity("Ping", |_in_map: &ValueMap| -> Result<ValueMap, String> {
        Ok(vmap! { "pong" => true })
    });
    engine
        .load_blueprints_from_directory(dir.to_str().unwrap())
        .unwrap();
    let _ = fs::remove_dir_all(&dir);

    let mut router = Router::new();
    engine.mount_routes(&mut router);
    router
}

#[test]
fn test_api_key_middleware_accepts_only_configured_keys() {
    let cases = [
        (
            "under config",
            "    config:\n      valid_keys: [partner-key-1]\n      header_name: X-Partner-Key\n",
            "partner-key-1",
        ),
        (
            "top level",
            "    valid_keys: [partner-key-1]\n    header_name: X-Partner-Key\n",
            "partner-key-1",
        ),
        ("none declared", "", ""),
    ];

    for (name, block, accepted) in cases {
        let r = new_keyed_engine(block);
        let url = "/api/v1.0.0/keyed/ping";
        let header = if !accepted.is_empty() {
            "X-Partner-Key"
        } else {
            "X-API-Key"
        };

        for old in ["default-api-key", "secret-key", "katz-secret-key-12345"] {
            let w = r.serve_http(HttpRequest::new("GET", url).with_header(header, old));
            assert_eq!(w.code, 401, "case {}: {}", name, old);
        }

        let w_no_key = r.serve_http(HttpRequest::new("GET", url));
        assert_eq!(w_no_key.code, 401, "case {}: no key", name);

        if !accepted.is_empty() {
            let w_ok =
                r.serve_http(HttpRequest::new("GET", url).with_header("X-Partner-Key", accepted));
            assert_eq!(w_ok.code, 200, "case {}: configured key", name);

            let w_wrong_hdr =
                r.serve_http(HttpRequest::new("GET", url).with_header("X-API-Key", accepted));
            assert_eq!(w_wrong_hdr.code, 401, "case {}: wrong header", name);
        }

        let w_health = r.serve_http(HttpRequest::new("GET", "/health"));
        assert_eq!(w_health.code, 200, "case {}: /health", name);
    }
}
