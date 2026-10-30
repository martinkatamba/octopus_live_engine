//! Runs the conformance cases on the octopus-core `Engine` (no HTTP host) and
//! prints the results as JSON.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use octopus_core::json::parse;
use octopus_core::{Engine, Value, ValueMap};

/// Call counts per activity, and every call's activity name in order.
type Calls = Arc<Mutex<(BTreeMap<String, i64>, Vec<String>)>>;

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn register(engine: &Engine, name: String, script: Vec<Value>, calls: Calls) {
    let key = name.clone();
    engine.register_activity(&name, move |input: &ValueMap| -> Result<ValueMap, String> {
        let n = {
            let mut c = calls.lock().unwrap();
            c.1.push(key.clone());
            let n = c.0.entry(key.clone()).or_insert(0);
            *n += 1;
            (*n - 1) as usize
        };
        let entry = &script[n.min(script.len() - 1)];
        if let Some(ms) = entry.get("delay_ms").and_then(|v| v.as_i64()) {
            std::thread::sleep(std::time::Duration::from_millis(ms.max(0) as u64));
        }
        if let Some(msg) = entry.get("fail") {
            return Err(msg.as_str().unwrap_or_default().to_string());
        }
        let mut out = ValueMap::new();
        if let Some(Value::Object(echo)) = entry.get("echo") {
            for (k, src) in echo {
                let v = src
                    .as_str()
                    .and_then(|s| input.get(s))
                    .cloned()
                    .unwrap_or(Value::Null);
                out.insert(k.clone(), v);
            }
        }
        if let Some(Value::Object(ret)) = entry.get("return") {
            out.extend(ret.clone());
        }
        Ok(out)
    });
}

const ASYNC_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The job as the state store holds it; with `wait_async`, once no step is
/// still ASYNC_DISPATCHED.
fn stored_state(engine: &Engine, job_id: &str, wait_async: bool) -> Value {
    let deadline = std::time::Instant::now() + ASYNC_WAIT;
    loop {
        let st = engine.get_execution_state(job_id).ok();
        let pending = st
            .as_ref()
            .is_some_and(|s| s.traces.iter().any(|t| t.status == "ASYNC_DISPATCHED"));
        if !wait_async || !pending || std::time::Instant::now() > deadline {
            return st.map(|s| s.to_value()).unwrap_or(Value::Null);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn new_engine(dir: &Path, activities: &ValueMap, calls: &Calls) -> Result<Engine, String> {
    let mut engine = Engine::new("");
    engine.load_blueprint_file(&dir.join("blueprint.yaml").to_string_lossy())?;
    for (name, script) in activities {
        let script = script.as_array().cloned().unwrap_or_default();
        register(&engine, name.clone(), script, Arc::clone(calls));
    }
    Ok(engine)
}

fn run_case(dir: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(dir.join("case.json")).map_err(|e| e.to_string())?;
    let case = parse(&text)?;
    let workflow = case
        .get("workflow")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let activities = case
        .get("activities")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let shared = case
        .get("shared_engine")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut runs = Vec::new();
    let mut calls: Calls = Arc::default();
    let mut engine: Option<Engine> = None;
    for run in case
        .get("runs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
    {
        if engine.is_none() || !shared {
            calls = Arc::default();
            match new_engine(dir, &activities, &calls) {
                Ok(e) => engine = Some(e),
                Err(e) => {
                    runs.push(obj(vec![("load_error", Value::String(e))]));
                    engine = None;
                    continue;
                }
            }
        }
        let engine = engine.as_ref().unwrap();
        let input = run
            .get("input")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let (state, error) = match engine.execute_sync(&workflow, input) {
            Ok(st) => (Some(st), Value::Null),
            Err((st, e)) => (st, Value::String(e)),
        };
        let wait_async = run
            .get("wait_async")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let stored = match &state {
            Some(st) if !st.job_id.is_empty() => stored_state(engine, &st.job_id, wait_async),
            _ => Value::Null,
        };
        let state = state.map(|s| s.to_value()).unwrap_or(Value::Null);
        let (counts, sequence) = {
            let c = calls.lock().unwrap();
            let counts: ValueMap =
                c.0.iter()
                    .map(|(k, v)| (k.clone(), Value::Int(*v)))
                    .collect();
            (
                counts,
                c.1.iter().cloned().map(Value::String).collect::<Vec<_>>(),
            )
        };
        runs.push(obj(vec![
            ("state", state),
            ("stored", stored),
            ("error", error),
            ("calls", Value::Object(counts)),
            ("sequence", Value::Array(sequence)),
        ]));
    }
    Ok(runs)
}

fn main() {
    let cases_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases"));
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&cases_dir)
        .expect("cases directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("case.json").exists())
        .collect();
    dirs.sort();
    let mut cases = ValueMap::new();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let runs =
            run_case(&dir).unwrap_or_else(|e| vec![obj(vec![("load_error", Value::String(e))])]);
        cases.insert(name, Value::Array(runs));
    }
    println!(
        "{}",
        obj(vec![
            ("engine", Value::String("core".into())),
            ("cases", Value::Object(cases))
        ])
        .to_json_string()
    );
}
