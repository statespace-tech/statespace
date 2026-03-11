use std::path::Path;

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
