//! Site initialization - writes template files if missing.

use crate::templates::{AGENTS_MD, FAVICON_SVG, OPENGRAPH_PNG};
use std::io;
use std::path::Path;
use tokio::fs;
use tracing::info;

#[derive(Debug, Clone, Copy)]
pub enum TemplateFile {
    AgentsMd,
    FaviconSvg,
    OpengraphPng,
}

impl TemplateFile {
    const fn filename(self) -> &'static str {
        match self {
            Self::AgentsMd => "AGENTS.md",
            Self::FaviconSvg => "favicon.svg",
            Self::OpengraphPng => "opengraph.png",
        }
    }
}

#[derive(Debug)]
pub enum InitResult {
    Created,
    AlreadyExists,
}

/// # Errors
///
/// Returns I/O errors when template files cannot be created or read.
pub async fn initialize_templates(
    content_root: &Path,
) -> io::Result<Vec<(TemplateFile, InitResult)>> {
    let mut results = Vec::with_capacity(3);

    results.push((
        TemplateFile::AgentsMd,
        write_if_missing(content_root, TemplateFile::AgentsMd.filename(), AGENTS_MD).await?,
    ));

    results.push((
        TemplateFile::FaviconSvg,
        write_if_missing(
            content_root,
            TemplateFile::FaviconSvg.filename(),
            FAVICON_SVG,
        )
        .await?,
    ));

    results.push((
        TemplateFile::OpengraphPng,
        write_if_missing(
            content_root,
            TemplateFile::OpengraphPng.filename(),
            OPENGRAPH_PNG,
        )
        .await?,
    ));

    for (file, result) in &results {
        match result {
            InitResult::Created => info!("Created {}", file.filename()),
            InitResult::AlreadyExists => {}
        }
    }

    Ok(results)
}

async fn write_if_missing(
    root: &Path,
    filename: &str,
    content: impl AsRef<[u8]>,
) -> io::Result<InitResult> {
    let path = root.join(filename);

    if path.exists() {
        return Ok(InitResult::AlreadyExists);
    }

    fs::write(&path, content).await?;
    Ok(InitResult::Created)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_initialize_templates_creates_files() {
        let dir = TempDir::new().unwrap();

        let results = initialize_templates(dir.path()).await.unwrap();

        assert_eq!(results.len(), 3);

        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("favicon.svg").exists());
        assert!(dir.path().join("opengraph.png").exists());
    }

    #[tokio::test]
    async fn test_initialize_templates_is_idempotent() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# Custom agents").unwrap();

        let results = initialize_templates(dir.path()).await.unwrap();

        let agents_result = results
            .iter()
            .find(|(f, _)| matches!(f, TemplateFile::AgentsMd));
        assert!(matches!(
            agents_result,
            Some((_, InitResult::AlreadyExists))
        ));

        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert_eq!(agents, "# Custom agents");
    }
}
