//! Shared embedded templates for Statespace servers.

pub const AGENTS_MD: &str = include_str!("AGENTS.md");

pub const FAVICON_SVG: &str = include_str!("favicon.svg");

pub const OPENGRAPH_PNG: &[u8] = include_bytes!("opengraph.png");

const PAGE_HTML_TEMPLATE: &str = include_str!("page.html");

/// Renders a markdown page wrapped in a minimal HTML document.
#[must_use]
pub fn render_page_html(content: &str) -> String {
    PAGE_HTML_TEMPLATE.replace("{content}", content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_md_contains_instructions() {
        assert!(AGENTS_MD.contains("Quick Start"));
        assert!(AGENTS_MD.contains("Execute tools"));
    }

    #[test]
    fn favicon_is_valid_svg() {
        assert!(FAVICON_SVG.starts_with("<?xml"));
        assert!(FAVICON_SVG.contains("<svg"));
    }

    #[test]
    fn opengraph_is_valid_png() {
        assert!(OPENGRAPH_PNG.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn render_page_html_replaces_content() {
        let html = render_page_html("<h1>Hello</h1>");
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(!html.contains("{content}"));
        assert!(html.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn page_html_includes_meta_tags() {
        let html = render_page_html("");
        assert!(html.contains(r#"<link rel="icon" href="/favicon.svg">"#));
        assert!(html.contains(r#"<meta property="og:image" content="/opengraph.png">"#));
        assert!(html.contains(r#"<meta property="og:type" content="website">"#));
    }
}
