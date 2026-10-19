//! `statespace.toml`: the app a repository's builds belong to, and the
//! settings of the arm an experiment branch is.

use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::proxy::assign::{SUBJECT_COOKIE, check_rule};

/// The subject rule an arm uses when its file sets none.
pub fn default_subject() -> String {
    format!("request.cookies.{SUBJECT_COOKIE}")
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// The app: lowercase letters, digits, and underscores, starting with a letter.
    pub app: String,
    /// The arm's share of all the app's subjects. Experiment branches only.
    pub share: Option<f64>,
    /// CEL over `request` that yields the subject ID.
    pub subject: Option<String>,
    /// CEL over `request` that decides who may enter.
    pub eligibility: Option<String>,
}

impl Project {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!(
                "could not read {}; run `ssp init github` to write one",
                path.display()
            )
        })?;
        let project: Self =
            toml::from_str(&text).with_context(|| format!("{} is invalid", path.display()))?;
        project
            .validate()
            .with_context(|| format!("{} is invalid", path.display()))?;
        Ok(project)
    }

    /// The app of `statespace.toml` in the current directory, if there is one.
    pub fn current_app() -> Option<String> {
        let text = std::fs::read_to_string(crate::cli::PROJECT_FILE).ok()?;
        toml::from_str::<Self>(&text)
            .ok()
            .map(|project| project.app)
    }

    pub fn subject(&self) -> String {
        self.subject.clone().unwrap_or_else(default_subject)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let app = self.app.as_bytes();
        if app.is_empty()
            || app.len() > 40
            || !app[0].is_ascii_lowercase()
            || !app
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        {
            bail!(
                "app must be 1 to 40 lowercase letters, digits, or underscores, starting with a letter"
            );
        }
        if let Some(share) = self.share
            && (!share.is_finite() || share <= 0.0 || share > 1.0)
        {
            bail!("share must be greater than 0 and at most 1, such as 0.05 for 5%");
        }
        check_rule(&self.subject()).context("invalid subject rule")?;
        if let Some(eligibility) = &self.eligibility {
            check_rule(eligibility).context("invalid eligibility rule")?;
        }
        Ok(())
    }
}

/// The experiment and arm an `exp/` branch names: `exp/pricing` is the arm
/// `pricing` of the experiment `pricing`, and `exp/checkout/one-page` the
/// arm `one-page` of `checkout`.
pub fn experiment_branch(branch: &str) -> Option<anyhow::Result<(String, String)>> {
    let path = branch
        .strip_prefix("refs/heads/")
        .unwrap_or(branch)
        .strip_prefix("exp/")?;
    let names: Vec<&str> = path.split('/').collect();
    Some(match names.as_slice() {
        [arm] => Ok(((*arm).to_owned(), (*arm).to_owned())),
        [experiment, arm] => Ok(((*experiment).to_owned(), (*arm).to_owned())),
        _ => Err(anyhow::anyhow!(
            "an experiment branch is exp/<experiment> or exp/<experiment>/<arm>, not {branch}"
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> anyhow::Result<Project> {
        let project: Project = toml::from_str(text)?;
        project.validate()?;
        Ok(project)
    }

    #[test]
    fn reads_an_arms_settings() {
        let project = parse(
            r#"
            app = "web"
            share = 0.05
            eligibility = 'request.headers["cf-ipcountry"] == "US"'
            "#,
        )
        .unwrap();
        assert_eq!(project.share, Some(0.05));
        assert_eq!(project.subject(), "request.cookies.ssp_sid");
        for invalid in [
            r#"app = "Web""#,
            r#"app = "web"
               share = 5"#,
            r#"app = "web"
               subject = "request.headers[""#,
            r#"app = "web"
               traffic = 0.1"#,
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn branches_name_experiments_and_arms() {
        let names = |branch| experiment_branch(branch).map(|names| names.unwrap());
        assert_eq!(
            names("exp/pricing"),
            Some(("pricing".into(), "pricing".into()))
        );
        assert_eq!(
            names("refs/heads/exp/checkout/one-page"),
            Some(("checkout".into(), "one-page".into()))
        );
        assert_eq!(names("main"), None);
        assert!(experiment_branch("exp/a/b/c").unwrap().is_err());
    }
}
