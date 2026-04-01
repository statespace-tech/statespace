use std::path::Path;

#[must_use]
pub fn markdown_lookup_candidates(path: &str) -> Vec<String> {
    let normalized = path.trim_start_matches('/');

    if normalized.is_empty() {
        return vec!["API.md".to_string()];
    }

    if normalized.ends_with('/') {
        return vec![format!("{normalized}README.md")];
    }

    if Path::new(normalized).extension().is_some() {
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
        assert_eq!(markdown_lookup_candidates(""), vec!["API.md"]);
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
    fn markdown_candidates_for_non_markdown_file() {
        assert_eq!(
            markdown_lookup_candidates("data/export.csv"),
            vec!["data/export.csv"]
        );
    }

    #[test]
    fn markdown_candidates_for_directory_path() {
        assert_eq!(
            markdown_lookup_candidates("docs/intro/"),
            vec!["docs/intro/README.md"]
        );
    }
}
