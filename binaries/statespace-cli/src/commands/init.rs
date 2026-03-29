use crate::args::InitArgs;
use crate::error::{Error, Result};
use inquire::Confirm;
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

pub(crate) async fn run_init(args: InitArgs) -> Result<()> {
    let output = &args.path;

    std::fs::create_dir_all(output)?;

    let (readme_content, dockerfile_content) = match args.template {
        Some(template) => {
            let t = statespace_templates::templates::get(&template)
                .expect("clap validated the template name but it wasn't found");
            (t.readme.to_string(), t.dockerfile.map(str::to_string))
        }
        None => (DEFAULT_README.to_string(), None),
    };

    let mut created: Vec<&str> = Vec::new();

    let readme_path = output.join("README.md");
    if confirm_overwrite(&readme_path, args.yes)? {
        std::fs::write(&readme_path, &readme_content)?;
        created.push("README.md");
    }

    if let Some(dockerfile) = dockerfile_content {
        let dockerfile_path = output.join("Dockerfile");
        if confirm_overwrite(&dockerfile_path, args.yes)? {
            std::fs::write(&dockerfile_path, &dockerfile)?;
            created.push("Dockerfile");
        }
    }

    let agents_path = output.join("AGENTS.md");
    if confirm_overwrite(&agents_path, args.yes)? {
        std::fs::write(&agents_path, DEFAULT_AGENTS)?;
        created.push("AGENTS.md");
    }

    eprintln!("Created {} in {}", created.join(", "), output.display());
    eprintln!("Read AGENTS.md, then run `statespace serve {}`", output.display());

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
