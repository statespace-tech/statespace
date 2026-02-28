//! Site initialization - writes template files if missing.

use crate::templates::{AGENTS_MD, FAVICON_SVG, render_index_html};
use blake3;
use std::io;
use std::path::Path;
use tokio::fs;
use tracing::info;

#[derive(Debug, Clone, Copy)]
pub enum TemplateFile {
    AgentsMd,
    FaviconSvg,
    IndexHtml,
}

impl TemplateFile {
    const fn filename(self) -> &'static str {
        match self {
            Self::AgentsMd => "AGENTS.md",
            Self::FaviconSvg => "favicon.svg",
            Self::IndexHtml => "index.html",
        }
    }
}

#[derive(Debug)]
pub enum InitResult {
    Created,
    AlreadyExists,
    Updated,
}

/// # Errors
///
/// Returns I/O errors when template files cannot be created or read.
pub async fn initialize_templates(
    content_root: &Path,
    base_url: &str,
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

    let agents_content =
        read_or_default(content_root, TemplateFile::AgentsMd.filename(), AGENTS_MD).await;
    let index_html = render_index_html(base_url, &agents_content);
    results.push((
        TemplateFile::IndexHtml,
        write_if_changed(
            content_root,
            TemplateFile::IndexHtml.filename(),
            &index_html,
        )
        .await?,
    ));

    for (file, result) in &results {
        match result {
            InitResult::Created => info!("Created {}", file.filename()),
            InitResult::Updated => info!("Updated {}", file.filename()),
            InitResult::AlreadyExists => {}
        }
    }

    Ok(results)
}

async fn write_if_missing(root: &Path, filename: &str, content: &str) -> io::Result<InitResult> {
    let path = root.join(filename);

    if path.exists() {
        return Ok(InitResult::AlreadyExists);
    }

    fs::write(&path, content).await?;
    Ok(InitResult::Created)
}

async fn write_if_changed(root: &Path, filename: &str, content: &str) -> io::Result<InitResult> {
    let path = root.join(filename);
    match fs::read(&path).await {
        Ok(existing) => {
            if blake3::hash(&existing) == blake3::hash(content.as_bytes()) {
                return Ok(InitResult::AlreadyExists);
            }

            fs::write(&path, content).await?;
            Ok(InitResult::Updated)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::write(&path, content).await?;
            Ok(InitResult::Created)
        }
        Err(e) => Err(e),
    }
}

async fn read_or_default(root: &Path, filename: &str, default: &str) -> String {
    let path = root.join(filename);
    fs::read_to_string(&path)
        .await
        .unwrap_or_else(|_| default.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_initialize_templates_creates_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();

        let results = initialize_templates(dir.path(), "http://localhost:8000")
            .await
            .unwrap();

        assert_eq!(results.len(), 3);

        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("favicon.svg").exists());
        assert!(dir.path().join("index.html").exists());

        let index = std::fs::read_to_string(dir.path().join("index.html")).unwrap();
        assert!(index.contains("http://localhost:8000"));
    }

    #[tokio::test]
    async fn test_initialize_templates_updates_index_html_on_change() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();

        let results = initialize_templates(dir.path(), "http://localhost:8000")
            .await
            .unwrap();

        assert_eq!(results.len(), 3);

        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("favicon.svg").exists());
        assert!(dir.path().join("index.html").exists());

        let index = std::fs::read_to_string(dir.path().join("index.html")).unwrap();
        assert!(index.contains("http://localhost:8000"));

        let results = initialize_templates(dir.path(), "http://localhost:8001")
            .await
            .unwrap();
        assert_eq!(results.len(), 3);

        let index = std::fs::read_to_string(dir.path().join("index.html")).unwrap();
        assert!(index.contains("http://localhost:8001"));

        let index_result = results
            .iter()
            .find(|(f, _)| matches!(f, TemplateFile::IndexHtml));
        assert!(matches!(index_result, Some((_, InitResult::Updated))));

        let results = initialize_templates(dir.path(), "http://localhost:8001")
            .await
            .unwrap();
        assert_eq!(results.len(), 3);

        let index_result = results
            .iter()
            .find(|(f, _)| matches!(f, TemplateFile::IndexHtml));
        assert!(matches!(index_result, Some((_, InitResult::AlreadyExists))));
    }

    #[tokio::test]
    async fn test_initialize_templates_is_idempotent() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# Custom agents").unwrap();

        let results = initialize_templates(dir.path(), "http://localhost:8000")
            .await
            .unwrap();

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

    #[tokio::test]
    async fn test_index_html_uses_existing_agents_md() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# My custom instructions").unwrap();

        initialize_templates(dir.path(), "http://localhost:8000")
            .await
            .unwrap();

        let index = std::fs::read_to_string(dir.path().join("index.html")).unwrap();
        assert!(index.contains("# My custom instructions"));
    }
}
