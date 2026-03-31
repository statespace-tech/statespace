//! Shared embedded templates for Statespace servers.

pub mod templates;

pub const AGENTS_MD: &str = include_str!("../../../AGENTS.md");

pub const FAVICON_SVG: &str = include_str!("favicon.svg");

pub const GITIGNORE: &str = include_str!("../../../.gitignore");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicon_is_valid_svg() {
        assert!(FAVICON_SVG.starts_with("<?xml"));
        assert!(FAVICON_SVG.contains("<svg"));
    }
}
