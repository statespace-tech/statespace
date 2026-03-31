use crate::args::InitArgs;
use crate::error::{Error, Result};
use inquire::Confirm;
use statespace_templates::{AGENTS_MD, FAVICON_SVG, GITIGNORE};
use std::fs;
use std::path::Path;

fn confirm(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    Confirm::new(prompt)
        .with_default(false)
        .prompt()
        .map_err(|e| Error::cli(format!("Prompt failed: {e}")))
}

fn write_if_confirmed(path: &Path, content: &str, yes: bool) -> Result<bool> {
    if path.exists() && !confirm(&format!("Overwrite existing {}?", path.file_name().unwrap_or_default().to_string_lossy()), yes)? {
        return Ok(false);
    }
    fs::write(path, content)?;
    Ok(true)
}


pub(crate) async fn run_init(args: InitArgs) -> Result<()> {
    let output = &args.path;

    fs::create_dir_all(output)?;

    let (readme_content, dockerfile_content) = match args.template {
        Some(ref template) => {
            let t = statespace_templates::templates::get(template)
                .expect("clap validated the template name but it wasn't found");
            (t.readme.to_string(), t.dockerfile.map(str::to_string))
        }
        None => (String::new(), None),
    };

    let mut created: Vec<&str> = Vec::new();

    if write_if_confirmed(&output.join("README.md"), &readme_content, args.yes)? {
        created.push("README.md");
    }

    if write_if_confirmed(&output.join("AGENTS.md"), AGENTS_MD, args.yes)? {
        created.push("AGENTS.md");
    }

    if write_if_confirmed(&output.join("favicon.svg"), FAVICON_SVG, args.yes)? {
        created.push("favicon.svg");
    }

    if let Some(dockerfile) = dockerfile_content {
        if write_if_confirmed(&output.join("Dockerfile"), &dockerfile, args.yes)? {
            created.push("Dockerfile");
        }
    }

    if write_if_confirmed(&output.join(".gitignore"), GITIGNORE, args.yes)? {
        created.push(".gitignore");
    }

    eprintln!("Initialized: {}", created.join(", "));
    eprintln!("Read AGENTS.md, then run `statespace run {}`", output.display());

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn creates_all_default_files() {
        let dir = TempDir::new().unwrap();
        let args = InitArgs {
            path: dir.path().to_path_buf(),
            template: None,
            yes: true,
        };

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_init(args))
            .unwrap();

        assert!(dir.path().join("README.md").exists());
        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("favicon.svg").exists());
        assert!(dir.path().join(".gitignore").exists());
        assert!(!dir.path().join("Dockerfile").exists());
    }

    #[test]
    fn readme_is_blank_by_default() {
        let dir = TempDir::new().unwrap();
        let args = InitArgs {
            path: dir.path().to_path_buf(),
            template: None,
            yes: true,
        };

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_init(args))
            .unwrap();

        let content = fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn gitignore_created_from_template() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");

        write_if_confirmed(&path, GITIGNORE, true).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(".env"));
        assert!(content.contains(".statespace"));
        assert!(content.contains(".claude/"));
        assert!(content.contains(".cursor/"));
    }

    #[test]
    fn gitignore_prompts_on_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");
        fs::write(&path, "target/\n").unwrap();

        // yes=false would prompt interactively; yes=true overwrites
        let changed = write_if_confirmed(&path, GITIGNORE, true).unwrap();
        assert!(changed);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(".statespace"));
    }
}
