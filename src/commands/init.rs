//! `ssp init`: write a starter `statespace.toml` in the current folder and
//! print the CI step that records builds. It changes only the working copy.

use std::{io::Write, path::Path};

use serde_json::json;

use crate::{cli::PROJECT_FILE, output::emit, repo};

/// The starter file: every setting at its default, as a comment.
pub const STARTER: &str = r#"# Statespace settings for every app built from this folder.
# A change takes effect when the build of its commit runs as live.
# Reference: https://docs.statespace.com/statespace-toml

# The subject ID. The default is the visitor cookie that the proxy sets.
# subject = 'request.cookies.ssp_sid'

# Who may enter an arm or the control. Others get live.
# eligibility = 'request.headers["cf-ipcountry"] == "US"'

# The largest share of subjects in arms plus the control.
max_traffic = 0.2

# The most arms that run at once. The control does not count.
max_arms = 5

# The forge level needed to add and remove arms: write, maintain, or admin.
arm_role = "write"

# Let testers pick a running arm with ?ssp_arm=<commit>.
overrides = false

# Regular expressions. Matches become [redacted] before output leaves the machine.
redact = []

# A named count, matched on a request or on an output line.
# [[event]]
# name = "signup"
# match = 'request.method == "POST" && request.path == "/signup" && request.status < 300'

# A formula over the totals of one side. The first metric is the headline.
# [[metric]]
# label = "signup rate %"
# value = '100.0 * events["signup"].subjects / subjects'

# Settings for one app replace the shared settings above.
# [app.shop-dev]
# max_traffic = 1.0
# overrides = true
"#;

/// The CI step that records each build, with the image and the folder.
pub fn step(image: &str, folder: &str) -> String {
    format!(
        "- uses: statespace-tech/action@v1\n  \
         with:\n    \
         image: {image}\n    \
         path: {folder}\n    \
         # app: shop-prod   # optional: a build for one app only"
    )
}

pub fn run(image: Option<&str>) -> anyhow::Result<()> {
    let file = Path::new(PROJECT_FILE);
    let mut handle = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => {
                anyhow::anyhow!("{PROJECT_FILE} exists here already; edit it instead")
            }
            _ => anyhow::Error::new(error).context(format!("could not write {PROJECT_FILE}")),
        })?;
    handle.write_all(STARTER.as_bytes())?;
    let folder = repo::folder_of(Path::new("."));
    let image = image.unwrap_or("ghcr.io/acme/shop@${{ steps.build.outputs.digest }}");
    let step = step(image, &folder);
    let document = json!({
        "file": PROJECT_FILE,
        "path": folder,
        "step": step,
    });
    emit(&document, || {
        format!(
            "Wrote {PROJECT_FILE} in {folder}.\n\n\
             Add this step after the image build in the workflow that builds every commit.\n\
             The job needs `permissions: id-token: write`.\n\n{step}\n\n\
             Then commit both files, and run `ssp app add <name>` here."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starter_file_is_valid_toml() {
        let parsed: toml::Value = toml::from_str(STARTER).unwrap();
        assert_eq!(parsed["max_traffic"].as_float(), Some(0.2));
        assert_eq!(parsed["overrides"].as_bool(), Some(false));
        assert!(step("x@sha256:1", "shop").contains("path: shop"));
    }
}
