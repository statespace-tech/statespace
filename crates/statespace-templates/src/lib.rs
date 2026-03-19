//! Shared embedded templates for Statespace servers.

pub const AGENTS_MD: &str = include_str!("AGENTS.md");

pub const FAVICON_SVG: &str = include_str!("favicon.svg");

pub const OPENAPI_JSON: &str = include_str!("openapi.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_md_contains_instructions() {
        assert!(AGENTS_MD.contains("App instructions"));
        assert!(AGENTS_MD.contains("Quick start"));
    }

    #[test]
    fn favicon_is_valid_svg() {
        assert!(FAVICON_SVG.starts_with("<?xml"));
        assert!(FAVICON_SVG.contains("<svg"));
    }
}
