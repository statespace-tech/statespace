use crate::args::InitArgs;
use crate::error::{Error, Result};
use inquire::Confirm;
use statespace_templates::{AGENTS_MD, API_MD, GITIGNORE};
use std::collections::HashSet;
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

fn merge_gitignore(path: &Path, template: &str) -> Result<bool> {
    if !path.exists() {
        fs::write(path, template)?;
        return Ok(true);
    }
    let existing = fs::read_to_string(path)?;
    let existing_entries: HashSet<&str> = existing
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let missing: Vec<&str> = template
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#') && !existing_entries.contains(t)
        })
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }
    let mut content = existing;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push('\n');
    for line in missing {
        content.push_str(line);
        content.push('\n');
    }
    fs::write(path, content)?;
    Ok(true)
}

fn write_if_confirmed(path: &Path, content: &str, yes: bool) -> Result<bool> {
    if path.exists()
        && !confirm(
            &format!(
                "Overwrite existing {}?",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            yes,
        )?
    {
        return Ok(false);
    }
    fs::write(path, content)?;
    Ok(true)
}

pub(crate) fn run_init(args: &InitArgs) -> Result<()> {
    let output = &args.path;

    fs::create_dir_all(output)?;

    let (readme_content, dockerfile_content) = match args.template {
        Some(ref template) => {
            let t = statespace_templates::get(template)
                .ok_or_else(|| Error::cli(format!("unknown template: {template}")))?;
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

    if write_if_confirmed(&output.join("CLAUDE.md"), AGENTS_MD, args.yes)? {
        created.push("CLAUDE.md");
    }

    if write_if_confirmed(&output.join("API.md"), API_MD, args.yes)? {
        created.push("API.md");
    }

    if merge_gitignore(&output.join(".gitignore"), GITIGNORE)? {
        created.push(".gitignore");
    }

    if let Some(dockerfile) = dockerfile_content {
        if write_if_confirmed(&output.join("Dockerfile"), &dockerfile, args.yes)? {
            created.push("Dockerfile");
        }
    }

    eprintln!("Initialized: {}", created.join(", "));
    eprintln!("Read AGENTS.md for setup and run instructions.");
    eprintln!(
        "If your app uses a `.env` file, load it explicitly with `statespace run --env-file .env {}`",
        output.display()
    );

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

        run_init(&args).unwrap();

        assert!(dir.path().join("README.md").exists());
        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("CLAUDE.md").exists());
        assert!(dir.path().join("API.md").exists());
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

        run_init(&args).unwrap();

        let content = fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn postgresql_template_creates_expected_files() {
        let dir = TempDir::new().unwrap();
        let args = InitArgs {
            path: dir.path().to_path_buf(),
            template: Some("postgresql".into()),
            yes: true,
        };

        run_init(&args).unwrap();

        assert!(dir.path().join("README.md").exists());
        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("CLAUDE.md").exists());
        assert!(dir.path().join("API.md").exists());
        assert!(dir.path().join(".gitignore").exists());
        assert!(dir.path().join("Dockerfile").exists());

        let readme = fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert!(readme.contains("psql"));
        assert!(readme.contains("DATABASE_URL"));
    }

    #[test]
    fn gitignore_created_from_template() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");

        merge_gitignore(&path, GITIGNORE).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(".env"));
        assert!(content.contains(".statespace"));
        assert!(content.contains(".claude/"));
        assert!(content.contains(".cursor/"));
    }

    #[test]
    fn gitignore_merges_missing_entries_into_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");
        fs::write(&path, "target/\n").unwrap();

        let changed = merge_gitignore(&path, GITIGNORE).unwrap();
        assert!(changed);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("target/"), "existing entry preserved");
        assert!(content.contains(".statespace"), "missing entry appended");
    }

    #[test]
    fn gitignore_no_change_when_all_entries_present() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".gitignore");
        merge_gitignore(&path, GITIGNORE).unwrap();

        let changed = merge_gitignore(&path, GITIGNORE).unwrap();
        assert!(!changed, "nothing to add on second run");
    }
}
