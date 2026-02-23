//! Shared embedded templates for Statespace servers.

pub const AGENTS_MD: &str = include_str!("AGENTS.md");

pub const FAVICON_SVG: &str = include_str!("favicon.svg");

const INDEX_HTML_TEMPLATE: &str = include_str!("index.html");

/// Renders the index HTML page, replacing `{current_url}` and `{agents_md_content}` placeholders.
///
/// Both inputs are trusted server-controlled values (base URL from server config,
/// agents_md from on-disk AGENTS.md) and are inserted without HTML escaping.
#[must_use]
pub fn render_index_html(base_url: &str, agents_md: &str) -> String {
    INDEX_HTML_TEMPLATE
        .replace("{current_url}", base_url)
        .replace("{agents_md_content}", agents_md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_index_html_replaces_placeholders() {
        let html = render_index_html("http://localhost:8000", "# Test agents");

        assert!(html.contains("http://localhost:8000"));
        assert!(html.contains("# Test agents"));
        assert!(!html.contains("{current_url}"));
        assert!(!html.contains("{agents_md_content}"));
    }

    #[test]
    fn agents_md_contains_instructions() {
        assert!(AGENTS_MD.contains("Discover available tools"));
        assert!(AGENTS_MD.contains("Execute tools"));
    }

    #[test]
    fn favicon_is_valid_svg() {
        assert!(FAVICON_SVG.starts_with("<?xml"));
        assert!(FAVICON_SVG.contains("<svg"));
    }
}
