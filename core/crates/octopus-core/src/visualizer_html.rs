//! The visualizer frontend: one embedded asset, so every SDK serves the same
//! bytes. The host only serves it.

const VISUALIZER_TEMPLATE: &str = include_str!("visualizer_template.html");

/// Renders the complete interactive dark-mode AntV X6 application.
pub fn render_visualizer_html(default_domain: &str, route_prefix: &str) -> String {
    VISUALIZER_TEMPLATE
        .replace("{{DEFAULT_DOMAIN}}", default_domain)
        .replace("{{DEFAULT_PREFIX}}", route_prefix)
}
