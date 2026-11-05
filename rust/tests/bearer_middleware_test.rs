use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use octopus_live_engine::crypto::{base64url_encode, hmac_sha256};
use octopus_live_engine::{vmap, HttpRequest, OctopusLiveEngine, Router, Value, ValueMap};

static NEXT_JWT_DIR_ID: AtomicU64 = AtomicU64::new(1);

fn hs256(claims: ValueMap, secret: &str, alg: &str) -> String {
    let hdr = Value::Object(vmap! {
        "alg" => alg,
        "typ" => "JWT",
    })
    .to_json_string();
    let payload = Value::Object(claims).to_json_string();
    let signing_input = format!(
        "{}.{}",
        base64url_encode(hdr.as_bytes()),
        base64url_encode(payload.as_bytes())
    );
    let sig = hmac_sha256(secret.as_bytes(), signing_input.as_bytes());
    format!("{}.{}", signing_input, base64url_encode(&sig))
}

fn new_jwt_engine(secret_block: &str) -> Router {
    let id = NEXT_JWT_DIR_ID.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("octopus_test_jwt_{}_{}", std::process::id(), id));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let bp = format!(
        "workflow_name: jwtwf\nversion: \"1.0.0\"\nmiddlewares:\n  - name: BearerAuthAndClaimsMiddleware\n{}    excludes: [/health]\napi:\n  - route: /api/v1.0.0/jwtwf/me\n    method: GET\n    steps:\n      - id: me\n        activity: Me\n        is_output: true\n",
        secret_block
    );
    fs::write(dir.join("jwtwf.yaml"), bp).unwrap();

    let mut engine = OctopusLiveEngine::new("");
    engine.register_activity("Me", |_in_map: &ValueMap| -> Result<ValueMap, String> {
        Ok(vmap! { "ok" => true })
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
fn test_bearer_middleware_verifies_hs256_or_refuses() {
    let cases = [
        (
            "config secret",
            "    config:\n      jwt_secret: blueprint-secret-1\n",
            "",
        ),
        (
            "top-level secret",
            "    jwt_secret: blueprint-secret-1\n",
            "",
        ),
        ("env secret", "", "blueprint-secret-1"),
        ("no secret", "", ""),
    ];

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as f64;
    let later = now_secs + 600.0;
    let earlier = now_secs - 60.0;

    for (name, block, env_val) in cases {
        std::env::set_var("OCTOPUS_JWT_SECRET", env_val);
        let r = new_jwt_engine(block);
        let url = "/api/v1.0.0/jwtwf/me";

        let refused = [
            "anything-at-all".to_string(),
            hs256(
                vmap! { "sub" => "mallory", "exp" => later },
                "wrong-secret",
                "HS256",
            ),
            hs256(
                vmap! { "sub" => "alice", "exp" => earlier },
                "blueprint-secret-1",
                "HS256",
            ),
            hs256(
                vmap! { "sub" => "alice", "exp" => later },
                "blueprint-secret-1",
                "none",
            ),
            hs256(
                vmap! { "sub" => "alice" },
                "octopus-live-engine-secret-key-2026",
                "HS256",
            ),
        ];

        for tok in refused {
            let w = r.serve_http(
                HttpRequest::new("GET", url)
                    .with_header("Authorization", &format!("Bearer {}", tok)),
            );
            assert_eq!(w.code, 401, "case {}: refused token {}", name, tok);
        }

        let want = if !block.is_empty() || !env_val.is_empty() {
            200
        } else {
            401
        };
        let good = hs256(
            vmap! { "sub" => "alice", "exp" => later },
            "blueprint-secret-1",
            "HS256",
        );
        let w = r.serve_http(
            HttpRequest::new("GET", url).with_header("Authorization", &format!("Bearer {}", good)),
        );
        assert_eq!(
            w.code,
            want,
            "case {}: valid token body={}",
            name,
            w.body_string()
        );

        let w_health = r.serve_http(HttpRequest::new("GET", "/health"));
        assert_eq!(w_health.code, 200);
    }
    std::env::remove_var("OCTOPUS_JWT_SECRET");
}
