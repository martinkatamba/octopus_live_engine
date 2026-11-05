use octopus_live_engine::domain::ports::IVisualizerCompiler;
use octopus_live_engine::{new_live_engine, Value};

#[test]
fn test_visualizer_diagram_compilation_all_nodes() {
    let mut engine = new_live_engine("");
    engine
        .load_blueprint_file("enterprise_settlement_workflow.yaml")
        .expect("Failed to load blueprint");

    let diag = engine
        .visualizer_service
        .compile_x6_diagram("enterprise_settlement_gateway", None, None, None)
        .expect("Failed to compile X6 diagram");

    let nodes = match diag.get("nodes") {
        Some(Value::Array(arr)) => arr,
        _ => panic!("Expected nodes array in diagram"),
    };

    let mut has_loop = false;
    let mut has_branch = false;
    let mut has_signal = false;
    let mut has_async = false;
    let mut has_activity = false;

    for node in nodes {
        if let Value::Object(n) = node {
            if let Some(Value::Object(data)) = n.get("data") {
                let node_type = data.get("type").and_then(|v: &Value| v.as_str()).unwrap_or("");
                match node_type {
                    "loop" => has_loop = true,
                    "branch" => has_branch = true,
                    "signal" => has_signal = true,
                    "async" => has_async = true,
                    "activity" => has_activity = true,
                    _ => {}
                }
            }
        }
    }

    assert!(has_loop, "Expected loop node in diagram");
    assert!(has_branch, "Expected branch node in diagram");
    assert!(has_signal, "Expected signal node in diagram");
    assert!(has_async, "Expected async node in diagram");
    assert!(has_activity, "Expected activity node in diagram");
}
