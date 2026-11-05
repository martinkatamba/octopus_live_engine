use std::collections::HashMap;

use octopus_live_engine::domain::models::{StepCondition, WorkflowBlueprint, WorkflowStep};
use octopus_live_engine::domain::ports::IVisualizerCompiler;
use octopus_live_engine::{vmap, OctopusLiveEngine, Value, ValueMap};

#[test]
fn test_diagram_shows_skipped_branch() {
    let mut e = OctopusLiveEngine::new("");
    let noop = |_in_map: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "handled" => true }) };
    e.register_activity("Find", noop);
    e.register_activity("Check", noop);
    e.register_activity("Respond", noop);

    e.register_blueprint(WorkflowBlueprint {
        workflow_name: "cb".to_string(),
        version: "1.0.0".to_string(),
        steps: vec![
            WorkflowStep {
                id: "find".to_string(),
                activity: "Find".to_string(),
                ..Default::default()
            },
            WorkflowStep {
                id: "check_here".to_string(),
                condition: Some(StepCondition {
                    variable: "handled".to_string(),
                    operator: "!=".to_string(),
                    value: Value::Bool(true),
                    ..Default::default()
                }),
                then_steps: vec![WorkflowStep {
                    id: "check".to_string(),
                    activity: "Check".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            WorkflowStep {
                id: "respond".to_string(),
                activity: "Respond".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    })
    .unwrap();

    let st = e.execute_sync("cb", ValueMap::new()).unwrap();
    let d = e
        .visualizer_service
        .compile_x6_diagram("cb", Some(&st.job_id), None, None)
        .unwrap();

    let edges = d.get("edges").and_then(|v| v.as_array()).unwrap();
    let mut skip: Option<&Value> = None;
    let mut to_check: Option<&Value> = None;
    for edge in edges {
        let src = edge.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let tgt = edge.get("target").and_then(|v| v.as_str()).unwrap_or("");
        if src == "node__check_here" && tgt == "node__respond" {
            skip = Some(edge);
        } else if src == "node__check_here" && tgt.ends_with("__check") {
            to_check = Some(edge);
        }
    }
    let skip_edge = skip.expect("no edge from the false condition to the next step");
    let skip_json = skip_edge.to_json_string();
    assert!(skip_json.contains("FALSE (Skip)"));
    assert!(skip_json.contains("\"stroke\":\"#10b981\""));

    if let Some(tc) = to_check {
        assert!(!tc.to_json_string().contains("\"stroke\":\"#10b981\""));
    }

    let nodes = d.get("nodes").and_then(|v| v.as_array()).unwrap();
    for n in nodes {
        if n.get("id").and_then(|v| v.as_str()) == Some("node__check_here") {
            assert!(n.to_json_string().contains(r#""executed":true"#));
        }
    }
}

#[test]
fn test_diagram_tells_signal_waits_from_human_steps() {
    let mut e = OctopusLiveEngine::new("");
    e.register_blueprint(WorkflowBlueprint {
        workflow_name: "waits".to_string(),
        version: "1.0.0".to_string(),
        steps: vec![
            WorkflowStep {
                id: "wait_for_payment".to_string(),
                await_signal: "payment_result".to_string(),
                timeout_seconds: 200,
                ..Default::default()
            },
            WorkflowStep {
                id: "manager_ok".to_string(),
                human_step: "approval".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    })
    .unwrap();

    let d = e
        .visualizer_service
        .compile_x6_diagram("waits", None, None, None)
        .unwrap();
    let mut types = HashMap::new();
    for n in d.get("nodes").and_then(|v| v.as_array()).unwrap() {
        let id = n.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let t = n
            .get("data")
            .and_then(|d| d.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        types.insert(id.to_string(), t.to_string());
    }
    assert_eq!(
        types.get("node__wait_for_payment").map(|s| s.as_str()),
        Some("signal")
    );
    assert_eq!(
        types.get("node__manager_ok").map(|s| s.as_str()),
        Some("human")
    );
}

#[test]
fn test_diagram_does_not_match_steps_by_activity_name() {
    let mut e = OctopusLiveEngine::new("");
    e.register_activity("Check", |_in_map: &ValueMap| -> Result<ValueMap, String> {
        Ok(vmap! { "nothing_due" => false })
    });
    e.register_activity(
        "Redirect",
        |_in_map: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "redirected" => true }) },
    );

    e.register_blueprint(WorkflowBlueprint {
        workflow_name: "pay".to_string(),
        version: "1.0.0".to_string(),
        steps: vec![
            WorkflowStep {
                id: "check".to_string(),
                activity: "Check".to_string(),
                ..Default::default()
            },
            WorkflowStep {
                id: "anything_to_pay".to_string(),
                condition: Some(StepCondition {
                    variable: "nothing_due".to_string(),
                    operator: "==".to_string(),
                    value: Value::Bool(true),
                    ..Default::default()
                }),
                then_steps: vec![WorkflowStep {
                    id: "show_included_order".to_string(),
                    activity: "Redirect".to_string(),
                    ..Default::default()
                }],
                else_steps: vec![WorkflowStep {
                    id: "go_to_order".to_string(),
                    activity: "Redirect".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    })
    .unwrap();

    let st = e.execute_sync("pay", ValueMap::new()).unwrap();
    let d = e
        .visualizer_service
        .compile_x6_diagram("pay", Some(&st.job_id), None, None)
        .unwrap();

    let mut executed = HashMap::new();
    for n in d.get("nodes").and_then(|v| v.as_array()).unwrap() {
        if let Some(data) = n.get("data").and_then(|v| v.as_object()) {
            if let Some(id) = data.get("step_id").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    let exec = data
                        .get("executed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    executed.insert(id.to_string(), exec);
                }
            }
        }
    }
    assert_eq!(executed.get("go_to_order").copied(), Some(true));
    assert_eq!(executed.get("show_included_order").copied(), Some(false));
}

#[test]
fn test_diagram_disambiguates_same_route_different_http_methods() {
    use octopus_live_engine::domain::models::APIEndpointConfig;
    use octopus_live_engine::domain::ports::IWorkflowEngine;

    let mut e = OctopusLiveEngine::new("");
    e.register_activity(
        "CreateTodo",
        |_in_map: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "created" => true }) },
    );
    e.register_activity(
        "ListTodos",
        |_in_map: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! { "items" => Value::Array(vec![]) })
        },
    );

    let post_ep = APIEndpointConfig {
        route: "/api/v1/todos".to_string(),
        method: "POST".to_string(),
        status_code: 201,
        steps: vec![WorkflowStep {
            id: "step_persist_todo".to_string(),
            activity: "CreateTodo".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let get_ep = APIEndpointConfig {
        route: "/api/v1/todos".to_string(),
        method: "GET".to_string(),
        status_code: 200,
        steps: vec![WorkflowStep {
            id: "step_fetch_all_todos".to_string(),
            activity: "ListTodos".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    e.register_blueprint(WorkflowBlueprint {
        workflow_name: "todo_api_service".to_string(),
        version: "1.0.0".to_string(),
        api: vec![post_ep.clone(), get_ep.clone()],
        ..Default::default()
    })
    .unwrap();

    let get_state = e
        .pipeline_manager
        .execute_endpoint_sync(
            "todo_api_service",
            Some(&get_ep),
            "/api/v1/todos",
            "GET",
            ValueMap::new(),
        )
        .unwrap();

    let d = e
        .visualizer_service
        .compile_x6_diagram("todo_api_service", Some(&get_state.job_id), None, None)
        .unwrap();

    assert_eq!(
        d.get("selected_method").and_then(|v| v.as_str()),
        Some("GET")
    );
    assert_eq!(
        d.get("selected_route").and_then(|v| v.as_str()),
        Some("/api/v1/todos")
    );
    assert_eq!(d.get("executed_count").and_then(|v| v.as_i64()), Some(1));

    let nodes = d.get("nodes").and_then(|v| v.as_array()).unwrap();
    let fetch_node = nodes
        .iter()
        .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("node__step_fetch_all_todos"))
        .expect("GET endpoint step_fetch_all_todos node must be rendered");
    assert_eq!(
        fetch_node
            .get("data")
            .and_then(|d| d.get("executed"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}
