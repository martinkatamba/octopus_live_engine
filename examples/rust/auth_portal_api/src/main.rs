use auth_portal_api::build_engine;

fn main() {
    let mut engine = build_engine();

    // Load Blueprint & Mount Routes
    engine
        .load_blueprint_file("auth_portal_workflow.yaml")
        .expect("Failed to load auth_portal_workflow.yaml");

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.trim_start_matches(':').parse::<u16>().ok())
        .unwrap_or(8093);

    let router = engine.get_router();

    println!("\n==================================================================");
    println!(
        "🚀 OCTOPUS Live Engine - Rust Customer Auth Portal API on http://localhost:{}",
        port
    );
    println!("📖 Swagger Docs:    http://localhost:{}/docs", port);
    println!(
        "🎨 Live Visualizer: http://localhost:{}/visualizer?domain=customer_auth_portal",
        port
    );
    println!("==================================================================");

    router
        .run(&format!("127.0.0.1:{}", port))
        .expect("Server failed");
}
