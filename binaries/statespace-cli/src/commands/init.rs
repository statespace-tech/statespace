use crate::args::InitArgs;
use crate::commands::env::resolve_env_overrides;
use crate::config::{Config, save_config};
use crate::error::{Error, Result};
use inquire::Confirm;
use std::collections::HashMap;
use std::path::Path;

const DEFAULT_README: &str = "---
tools: []
---

# App

Describe your app here.
";

const DEFAULT_AGENTS: &str = include_str!("../../../../AGENTS.md");

/// Returns true if the file should be written (either missing, --yes, or user confirms).
fn confirm_overwrite(path: &Path, yes: bool) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if yes {
        return Ok(true);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    Confirm::new(&format!("Override existing {name}?"))
        .with_default(false)
        .prompt()
        .map_err(|e| Error::cli(format!("Prompt failed: {e}")))
}

async fn fetch_template_file(template: &str, filename: &str) -> Result<Option<String>> {
    let normalized = template.to_lowercase().replace('-', "_");
    let url = format!(
        "https://raw.githubusercontent.com/statespace-tech/statespace/main/examples/{normalized}/{filename}"
    );
    let response = reqwest::get(&url).await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(Error::cli(format!("HTTP {}: {}", response.status(), url)));
    }
    Ok(Some(response.text().await?))
}

pub(crate) async fn run_init(args: InitArgs) -> Result<()> {
    let output = &args.path;

    std::fs::create_dir_all(output)?;

    let (readme_content, dockerfile_content) = match &args.from {
        Some(from) => {
            let readme = fetch_template_file(from, "README.md")
                .await?
                .ok_or_else(|| Error::cli(format!("Unknown template '{from}'")))?;
            let dockerfile = fetch_template_file(from, "Dockerfile").await?;
            (readme, dockerfile)
        }
        None => (DEFAULT_README.to_string(), None),
    };

    let readme_path = output.join("README.md");
    if confirm_overwrite(&readme_path, args.yes)? {
        std::fs::write(&readme_path, &readme_content)?;
    }

    if let Some(dockerfile) = dockerfile_content {
        let dockerfile_path = output.join("Dockerfile");
        if confirm_overwrite(&dockerfile_path, args.yes)? {
            std::fs::write(&dockerfile_path, &dockerfile)?;
        }
    }

    let agents_path = output.join("AGENTS.md");
    if confirm_overwrite(&agents_path, args.yes)? {
        std::fs::write(&agents_path, DEFAULT_AGENTS)?;
    }

    if !args.env_vars.is_empty() {
        let env = resolve_env_overrides(HashMap::new(), &args.env_vars, None, "init")?;
        let config = Config {
            env,
            ..Config::default()
        };
        save_config(&output.join("config.toml"), &config)?;
    }

    eprintln!("Initialized '{}'", output.display());
    eprintln!("Run: statespace serve {}", output.display());

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn confirm_overwrite_returns_true_when_file_missing() {
        let dir = TempDir::new().unwrap();
        assert!(confirm_overwrite(&dir.path().join("missing.md"), false).unwrap());
    }

    #[test]
    fn confirm_overwrite_returns_true_with_yes_flag() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("file.md");
        std::fs::write(&path, "existing").unwrap();
        assert!(confirm_overwrite(&path, true).unwrap());
    }
}
