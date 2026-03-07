use crate::templates::render_page_html;
use ammonia::Builder;
use axum::http::{HeaderMap, header};
use pulldown_cmark::{Options, Parser, html};
use std::path::Path;

const BROWSER_UA_KEYWORDS: &[&str] = &["Chrome/", "Safari/", "Firefox/", "Edg/", "Opera/", "OPR/"];

#[must_use]
pub fn is_browser_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| BROWSER_UA_KEYWORDS.iter().any(|kw| ua.contains(kw)))
}

#[must_use]
pub fn wants_html(headers: &HeaderMap) -> bool {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase);

    match accept.as_deref() {
        Some(a) if a.contains("text/html") => true,
        Some(a) if a.contains("text/markdown") || a.contains("text/plain") => false,
        Some(a) if a.contains("*/*") => is_browser_request(headers),
        Some(_) | None => is_browser_request(headers),
    }
}

#[must_use]
pub fn render_markdown_to_html(markdown: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    let sanitized = Builder::default().clean(&html_output).to_string();
    render_page_html(&sanitized)
}

#[must_use]
pub fn is_agents_request(path: &str) -> bool {
    path == "AGENTS" || path == "AGENTS.md"
}

#[must_use]
pub fn markdown_lookup_candidates(path: &str) -> Vec<String> {
    let normalized = path.trim_start_matches('/');

    if normalized.is_empty() {
        return vec!["README.md".to_string()];
    }

    if normalized.ends_with('/') {
        return vec![format!("{normalized}README.md")];
    }

    if Path::new(normalized)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        return vec![normalized.to_string()];
    }

    vec![
        format!("{normalized}/README.md"),
        format!("{normalized}.md"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_candidates_for_root() {
        assert_eq!(markdown_lookup_candidates(""), vec!["README.md"]);
    }

    #[test]
    fn markdown_candidates_for_extensionless_path() {
        assert_eq!(
            markdown_lookup_candidates("docs/intro"),
            vec!["docs/intro/README.md", "docs/intro.md"]
        );
    }

    #[test]
    fn markdown_candidates_for_explicit_markdown_file() {
        assert_eq!(
            markdown_lookup_candidates("docs/intro.md"),
            vec!["docs/intro.md"]
        );
    }

    #[test]
    fn markdown_candidates_for_directory_path() {
        assert_eq!(
            markdown_lookup_candidates("docs/intro/"),
            vec!["docs/intro/README.md"]
        );
    }

    #[test]
    fn agents_requests_are_explicit_only() {
        assert!(is_agents_request("AGENTS"));
        assert!(is_agents_request("AGENTS.md"));
        assert!(!is_agents_request("agents"));
    }
}
