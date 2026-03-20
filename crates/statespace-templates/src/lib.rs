//! Shared embedded templates for Statespace servers.

pub const AGENTS_MD: &str = include_str!("AGENTS.md");

pub const FAVICON_SVG: &str = include_str!("favicon.svg");

pub const OPENAPI_JSON: &str = include_str!("openapi.json");

include!(concat!(env!("OUT_DIR"), "/generated_template_assets.rs"));

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

    #[test]
    fn template_assets_are_non_empty() {
        assert!(!RAG_TEMPLATE_ASSETS.is_empty());
        assert!(!KNOWLEDGE_BASE_TEMPLATE_ASSETS.is_empty());
        assert!(!AGENT_SKILL_TEMPLATE_ASSETS.is_empty());
        assert!(!TEXT_TO_SQL_TEMPLATE_ASSETS.is_empty());
        assert!(!WORKFLOW_TEMPLATE_ASSETS.is_empty());
    }
}
