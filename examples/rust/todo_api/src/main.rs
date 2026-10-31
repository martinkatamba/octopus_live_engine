use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use octopus_live_engine::crypto::now_utc;
use octopus_live_engine::{new_live_engine, vmap, Value, ValueMap};

fn main() {
    let mut engine = new_live_engine("");
    let todos_db: Arc<RwLock<HashMap<String, ValueMap>>> = Arc::new(RwLock::new(HashMap::new()));

    // 1. Register OpenAPI schemas
    engine.register_model(
        "CreateTodoRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "title" => Value::Object(vmap! { "type" => "string" }),
                "description" => Value::Object(vmap! { "type" => "string" }),
            }),
            "required" => Value::Array(vec![Value::String("title".to_string())]),
        }),
    );
    engine.register_model(
        "UpdateTodoRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "title" => Value::Object(vmap! { "type" => "string" }),
                "completed" => Value::Object(vmap! { "type" => "boolean" }),
            }),
        }),
    );
    engine.register_model(
        "TodoResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "id" => Value::Object(vmap! { "type" => "string" }),
                "title" => Value::Object(vmap! { "type" => "string" }),
                "description" => Value::Object(vmap! { "type" => "string" }),
                "completed" => Value::Object(vmap! { "type" => "boolean" }),
                "created_at" => Value::Object(vmap! { "type" => "string" }),
            }),
        }),
    );
    engine.register_model(
        "TodoListResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "todos" => Value::Object(vmap! { "type" => "array" }),
                "total_count" => Value::Object(vmap! { "type" => "integer" }),
            }),
        }),
    );
    engine.register_model(
        "DeleteTodoResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "id" => Value::Object(vmap! { "type" => "string" }),
                "deleted" => Value::Object(vmap! { "type" => "boolean" }),
                "message" => Value::Object(vmap! { "type" => "string" }),
            }),
        }),
    );

    // 2. Register Activities
    engine.register_activity(
        "ValidateTodoInput",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if title.is_empty() {
                return Err("todo 'title' cannot be empty".to_string());
            }
            let desc = input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(vmap! {
                "validated_title" => title,
                "validated_desc" => desc,
            })
        },
    );

    {
        let db = Arc::clone(&todos_db);
        engine.register_activity(
            "PersistTodo",
            move |input: &ValueMap| -> Result<ValueMap, String> {
                let mut guard = db.write().unwrap();
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let todo_id = format!("todo_{}", nanos % 100_000_000);
                let created_at = now_utc();
                let item = vmap! {
                    "id" => todo_id.clone(),
                    "title" => input.get("validated_title").cloned().unwrap_or(Value::String(String::new())),
                    "description" => input.get("validated_desc").cloned().unwrap_or(Value::String(String::new())),
                    "completed" => false,
                    "created_at" => created_at,
                };
                guard.insert(todo_id, item.clone());
                Ok(vmap! {
                    "todo_item" => Value::Object(item),
                })
            },
        );
    }

    {
        let db = Arc::clone(&todos_db);
        engine.register_activity(
            "FetchTodos",
            move |_input: &ValueMap| -> Result<ValueMap, String> {
                let guard = db.read().unwrap();
                let items: Vec<Value> = guard
                    .values()
                    .cloned()
                    .map(Value::Object)
                    .collect();
                let count = items.len() as i64;
                Ok(vmap! {
                    "items" => Value::Array(items),
                    "count" => count,
                })
            },
        );
    }

    {
        let db = Arc::clone(&todos_db);
        engine.register_activity(
            "FindTodoById",
            move |input: &ValueMap| -> Result<ValueMap, String> {
                let guard = db.read().unwrap();
                let id = input
                    .get("todo_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let Some(item) = guard.get(id) else {
                    return Err(format!("todo with id '{}' not found", id));
                };
                Ok(vmap! {
                    "todo_item" => Value::Object(item.clone()),
                })
            },
        );
    }

    {
        let db = Arc::clone(&todos_db);
        engine.register_activity(
            "UpdateTodo",
            move |input: &ValueMap| -> Result<ValueMap, String> {
                let mut guard = db.write().unwrap();
                let id = input
                    .get("todo_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let Some(item) = guard.get_mut(id) else {
                    return Err(format!("todo with id '{}' not found", id));
                };
                if let Some(t) = input.get("title").and_then(|v| v.as_str()) {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        item.insert("title".to_string(), Value::String(trimmed.to_string()));
                    }
                }
                if let Some(c) = input.get("completed").and_then(|v| v.as_bool()) {
                    item.insert("completed".to_string(), Value::Bool(c));
                }
                let updated = item.clone();
                Ok(vmap! {
                    "todo_item" => Value::Object(updated),
                })
            },
        );
    }

    {
        let db = Arc::clone(&todos_db);
        engine.register_activity(
            "DeleteTodo",
            move |input: &ValueMap| -> Result<ValueMap, String> {
                let mut guard = db.write().unwrap();
                let id = input
                    .get("todo_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if guard.remove(id).is_none() {
                    return Err(format!("todo with id '{}' not found", id));
                }
                Ok(vmap! {
                    "deleted_id" => id,
                })
            },
        );
    }

    engine.register_activity(
        "PackageTodoResponse",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let item = input
                .get("todo_item")
                .and_then(|v| v.as_object())
                .unwrap_or(input);
            Ok(vmap! {
                "id" => item.get("id").cloned().unwrap_or(Value::Null),
                "title" => item.get("title").cloned().unwrap_or(Value::Null),
                "description" => item.get("description").cloned().unwrap_or(Value::Null),
                "completed" => item.get("completed").cloned().unwrap_or(Value::Null),
                "created_at" => item.get("created_at").cloned().unwrap_or(Value::Null),
            })
        },
    );

    engine.register_activity(
        "PackageTodoListResponse",
        |input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "todos" => input.get("items").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
                "total_count" => input.get("count").cloned().unwrap_or(Value::Int(0)),
            })
        },
    );

    engine.register_activity(
        "PackageDeleteResponse",
        |input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "id" => input.get("deleted_id").cloned().unwrap_or(Value::Null),
                "deleted" => true,
                "message" => "Todo deleted successfully",
            })
        },
    );

    engine
        .load_blueprint_file("todo_workflow.yaml")
        .expect("Failed to load todo_workflow.yaml");

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.trim_start_matches(':').parse::<u16>().ok())
        .unwrap_or(8090);

    let router = engine.get_router();

    // Seed sample executions so the Visualizer immediately shows live job traces
    let create_resp = router.serve_http(
        octopus_live_engine::HttpRequest::new("POST", "/api/v1/todos")
            .with_header("Content-Type", "application/json")
            .with_body(
                r#"{"title":"Explore Rust Live Engine Visualizer","description":"Click nodes in AntV X6 to inspect step input/output payloads"}"#,
            ),
    );
    let created_job_id = create_resp.header("X-Job-ID");

    let list_resp =
        router.serve_http(octopus_live_engine::HttpRequest::new("GET", "/api/v1/todos"));
    let list_job_id = list_resp.header("X-Job-ID");

    let fail_resp = router.serve_http(
        octopus_live_engine::HttpRequest::new("POST", "/api/v1/todos")
            .with_header("Content-Type", "application/json")
            .with_body(r#"{"title":"","description":"Sample failed validation trace"}"#),
    );
    let fail_job_id = fail_resp.header("X-Job-ID");

    println!("\n==================================================================");
    println!(
        "🚀 OCTOPUS Live Engine - Rust Todo REST API on http://localhost:{}",
        port
    );
    println!("📖 Swagger Docs:    http://localhost:{}/docs", port);
    println!(
        "🎨 Live Visualizer: http://localhost:{}/visualizer?domain=todo_api_service",
        port
    );
    println!(
        "🔍 POST Job Trace:  http://localhost:{}/visualizer?domain=todo_api_service&job_id={}",
        port, created_job_id
    );
    println!(
        "🔍 GET Job Trace:   http://localhost:{}/visualizer?domain=todo_api_service&job_id={}",
        port, list_job_id
    );
    println!(
        "🔍 FAIL Job Trace:  http://localhost:{}/visualizer?domain=todo_api_service&job_id={}",
        port, fail_job_id
    );
    println!("==================================================================");

    router
        .run(&format!("127.0.0.1:{}", port))
        .expect("Server failed");
}
