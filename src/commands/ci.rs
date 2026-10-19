//! `ssp check` and `ssp apply`: statespace.toml on a laptop, and the build
//! of a CI job.

use std::{io::Write, path::Path, process::Command};

use anyhow::{Context, bail};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    api::{Api, decode},
    cli::CheckArgs,
    image,
    output::{emit, fields, share, text},
    project::{Project, experiment_branch},
    proxy::{
        self,
        assign::{request_value, test_rules},
    },
};

/// The audience `ssp apply` asks GitHub to put in its OIDC token.
const AUDIENCE: &str = "statespace";

pub fn check(arguments: &CheckArgs) -> anyhow::Result<()> {
    let project = Project::load(&arguments.file)?;
    let mut result = json!({
        "app": project.app,
        "share": project.share,
        "subject": project.subject(),
        "eligibility": project.eligibility,
    });
    let mut rows = vec![
        ("App", project.app.clone()),
        ("Share", project.share.map_or_else(|| "-".into(), share)),
        ("Subject", project.subject()),
        (
            "Eligibility",
            project
                .eligibility
                .clone()
                .unwrap_or_else(|| "everyone".into()),
        ),
    ];
    if let Some(path) = &arguments.path {
        let mut headers = http::HeaderMap::new();
        for (name, value) in proxy::pairs(&arguments.headers, "--header")? {
            headers.insert(
                http::HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes())
                    .with_context(|| format!("{name} is not a header name"))?,
                value
                    .parse()
                    .with_context(|| format!("{value} is not a header value"))?,
            );
        }
        let cookies: Map<String, Value> = proxy::pairs(&arguments.cookies, "--cookie")?
            .into_iter()
            .map(|(name, value)| (name, json!(value)))
            .collect();
        let request = request_value(
            &arguments.method,
            path,
            arguments.query.as_deref(),
            &headers,
            &cookies,
        );
        let (subject, eligible) =
            test_rules(&project.subject(), project.eligibility.as_deref(), &request)?;
        result["request"] = json!({ "subject": subject, "eligible": eligible });
        rows.push((
            "Request",
            match (&subject, eligible) {
                (None, _) => "no subject: the default serves it".into(),
                (Some(subject), true) => format!("subject {subject}, eligible"),
                (Some(subject), false) => format!("subject {subject}, not eligible"),
            },
        ));
    }
    emit(&result, || {
        format!(
            "{} is valid.\n\n{}",
            arguments.file.display(),
            fields(&rows)
        )
    })
}

/// What a GitHub Actions job says about itself.
struct Job {
    branch: String,
    request_url: String,
    request_token: String,
}

impl Job {
    fn from_env() -> anyhow::Result<Option<Self>> {
        if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
            return Ok(None);
        }
        let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let (Some(request_url), Some(request_token)) = (
            variable("ACTIONS_ID_TOKEN_REQUEST_URL"),
            variable("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
        ) else {
            bail!("ssp apply needs an OIDC token; add `permissions: id-token: write` to the job");
        };
        Ok(Some(Self {
            branch: variable("GITHUB_REF_NAME").context("GITHUB_REF_NAME is not set")?,
            request_url,
            request_token,
        }))
    }

    /// An OIDC token that proves the job's repository, workflow, and commit.
    fn token(&self, api: &Api) -> anyhow::Result<String> {
        #[derive(Deserialize)]
        struct Token {
            value: String,
        }
        let separator = if self.request_url.contains('?') {
            '&'
        } else {
            '?'
        };
        let url = format!("{}{separator}audience={AUDIENCE}", self.request_url);
        let token: Token = decode(
            api.client()
                .get(url)
                .bearer_auth(&self.request_token)
                .send()
                .context("could not request the job's OIDC token")?,
        )?;
        Ok(token.value)
    }
}

pub fn apply(api: &Api, reference: &str, file: &Path, dry_run: bool) -> anyhow::Result<()> {
    let project = Project::load(file)?;
    let Some(job) = Job::from_env()? else {
        if !dry_run {
            bail!("ssp apply records builds from GitHub Actions; pass --dry-run to check locally");
        }
        return local_plan(&project);
    };
    let image = image::resolve(reference)?;
    let body = json!({
        "app": project.app,
        "image": image,
        "share": project.share,
        "subject": project.subject,
        "eligibility": project.eligibility,
        "dry_run": dry_run,
    });
    let token = job.token(api)?;
    let result: Value = decode(
        api.anonymous(Method::POST, "/v1/ci/apply")
            .bearer_auth(token)
            .json(&body)
            .send()?,
    )?;
    let plan = text(&result["plan"]);
    if let Ok(summary) = std::env::var("GITHUB_STEP_SUMMARY")
        && let Ok(mut summary) = std::fs::OpenOptions::new().append(true).open(summary)
    {
        let _ = writeln!(summary, "**Statespace** `{}`: {plan}", job.branch);
    }
    emit(&result, || plan.clone())
}

/// What `ssp apply` would do on the current Git branch, without CI.
fn local_plan(project: &Project) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .context("could not run git")?;
    if !output.status.success() {
        bail!("ssp apply --dry-run needs a Git branch checked out");
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let plan = match experiment_branch(&branch) {
        Some(names) => {
            let (experiment, arm) = names?;
            let share = project
                .share
                .context("statespace.toml must set share for an experiment branch")?;
            json!({
                "app": project.app,
                "branch": branch,
                "experiment": experiment,
                "arm": arm,
                "share": share,
                "plan": format!(
                    "{experiment}: arm {arm} at {} (+ a control of the same size); it runs \
                     while its pull request can merge",
                    crate::output::share(share)
                ),
            })
        }
        None => json!({
            "app": project.app,
            "branch": branch,
            "plan": format!(
                "{branch}: on the default branch, CI records the default build; on other \
                 branches, nothing"
            ),
        }),
    };
    emit(&plan, || text(&plan["plan"]))
}
