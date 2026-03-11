//! Shared embedded templates for Statespace servers.

pub const AGENTS_MD: &str = include_str!("AGENTS.md");

pub const FAVICON_SVG: &str = include_str!("favicon.svg");

pub const OPENGRAPH_PNG: &[u8] = include_bytes!("opengraph.png");

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
        assert!(OPENGRAPH_PNG.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
    }
}
