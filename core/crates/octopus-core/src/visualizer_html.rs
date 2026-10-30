//! The visualizer frontend: one embedded asset, so every SDK serves the same
//! bytes. The host only serves it.

const VISUALIZER_TEMPLATE: &str = include_str!("visualizer_template.html");

/// Renders the complete interactive dark-mode AntV X6 application, for a
/// host whose activities are written in Rust.
pub fn render_visualizer_html(default_domain: &str, route_prefix: &str) -> String {
    render_visualizer_html_for(default_domain, route_prefix, "Rust")
}

/// As `render_visualizer_html`, naming the language the host's activities
/// are written in ("Python", "Go", ...) in the step inspector. Characters
/// other than letters, digits, spaces, `+`, `#` and `.` are dropped.
pub fn render_visualizer_html_for(
    default_domain: &str,
    route_prefix: &str,
    activity_language: &str,
) -> String {
    let language: String = activity_language
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '+' | '#' | '.'))
        .collect();
    VISUALIZER_TEMPLATE
        .replace("{{DEFAULT_DOMAIN}}", default_domain)
        .replace("{{DEFAULT_PREFIX}}", route_prefix)
        .replace("{{ACTIVITY_LANGUAGE}}", &language)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_inspector_names_the_hosts_activity_language() {
        let rust = render_visualizer_html("d", "/api");
        assert!(rust.contains("Rust Activity Implementation"));
        let python = render_visualizer_html_for("d", "/api", "Python");
        assert!(python.contains("Python Activity Implementation"));
        assert!(python.contains("activities (Python)"));
        assert!(!python.contains("{{ACTIVITY_LANGUAGE}}") && !python.contains("Rust Activity"));
        let odd = render_visualizer_html_for("d", "/api", "C#'</div><script>");
        assert!(odd.contains("C#divscript Activity Implementation"));
    }
}
