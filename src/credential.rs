//! The credential a command sends, in this order:
//!
//! 1. `STATESPACE_TOKEN`.
//! 2. The session that `ssp login` saved.
//! 3. A GitHub user token in `GH_TOKEN` or `GITHUB_TOKEN`.
//! 4. In GitHub Actions, the job's OIDC token.

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// The audience the CLI asks GitHub to put in an OIDC token.
pub const AUDIENCE: &str = "statespace";

/// Prefixes of GitHub tokens that act for a person.
const USER_TOKEN_PREFIXES: [&str; 4] = ["gho_", "ghp_", "ghu_", "github_pat_"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// `STATESPACE_TOKEN`.
    StatespaceToken,
    /// The session of `ssp login`.
    Session,
    /// `GH_TOKEN`.
    GhToken,
    /// `GITHUB_TOKEN`.
    GithubToken,
    /// The OIDC token of a GitHub Actions job.
    GithubOidc,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Self::StatespaceToken => "STATESPACE_TOKEN",
            Self::Session => "the ssp login session",
            Self::GhToken => "GH_TOKEN (GitHub user token)",
            Self::GithubToken => "GITHUB_TOKEN (GitHub user token)",
            Self::GithubOidc => "the GitHub Actions OIDC token",
        }
    }
}

/// How a credential is found, before it is read. An OIDC token needs a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Token(Source, String),
    Oidc { url: String, token: String },
}

impl Found {
    pub fn source(&self) -> Source {
        match self {
            Self::Token(source, _) => *source,
            Self::Oidc { .. } => Source::GithubOidc,
        }
    }
}

/// Pick the credential from the environment, read through `variable`, and
/// the saved session.
pub fn find(variable: impl Fn(&str) -> Option<String>, session: Option<String>) -> Option<Found> {
    let set = |name: &str| variable(name).filter(|value| !value.trim().is_empty());
    if let Some(token) = set("STATESPACE_TOKEN") {
        return Some(Found::Token(Source::StatespaceToken, token));
    }
    if let Some(token) = session.filter(|token| !token.is_empty()) {
        return Some(Found::Token(Source::Session, token));
    }
    for (name, source) in [
        ("GH_TOKEN", Source::GhToken),
        ("GITHUB_TOKEN", Source::GithubToken),
    ] {
        // In Actions, GITHUB_TOKEN is an installation token, which does not
        // name a person; only user tokens count.
        if let Some(token) = set(name)
            && USER_TOKEN_PREFIXES
                .iter()
                .any(|prefix| token.starts_with(prefix))
        {
            return Some(Found::Token(source, token));
        }
    }
    if set("GITHUB_ACTIONS").as_deref() == Some("true")
        && let (Some(url), Some(token)) = (
            set("ACTIONS_ID_TOKEN_REQUEST_URL"),
            set("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
        )
    {
        return Some(Found::Oidc { url, token });
    }
    None
}

/// The secret to send for `found`. An OIDC token is requested from GitHub.
pub fn secret(client: &reqwest::blocking::Client, found: &Found) -> anyhow::Result<String> {
    match found {
        Found::Token(_, token) => Ok(token.clone()),
        Found::Oidc { url, token } => {
            #[derive(Deserialize)]
            struct Token {
                value: String,
            }
            let separator = if url.contains('?') { '&' } else { '?' };
            let response = client
                .get(format!("{url}{separator}audience={AUDIENCE}"))
                .bearer_auth(token)
                .send()
                .context("could not request the job's OIDC token")?;
            if !response.status().is_success() {
                bail!(
                    "GitHub refused the job's OIDC token ({}); add `permissions: id-token: write` to the job",
                    response.status()
                );
            }
            Ok(response.json::<Token>()?.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn pick(variables: &[(&str, &str)], session: Option<&str>) -> Option<Found> {
        let variables: HashMap<String, String> = variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        find(
            |name| variables.get(name).cloned(),
            session.map(str::to_owned),
        )
    }

    #[test]
    fn picks_credentials_in_order() {
        let actions = [
            ("GITHUB_ACTIONS", "true"),
            ("ACTIONS_ID_TOKEN_REQUEST_URL", "https://token.actions/x"),
            ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request"),
        ];
        let everything: Vec<(&str, &str)> = [
            ("STATESPACE_TOKEN", "ssp_1"),
            ("GH_TOKEN", "gho_1"),
            ("GITHUB_TOKEN", "ghp_1"),
        ]
        .into_iter()
        .chain(actions)
        .collect();
        assert_eq!(
            pick(&everything, Some("session")),
            Some(Found::Token(Source::StatespaceToken, "ssp_1".into()))
        );
        assert_eq!(
            pick(&everything[1..], Some("session")),
            Some(Found::Token(Source::Session, "session".into()))
        );
        assert_eq!(
            pick(&everything[1..], None),
            Some(Found::Token(Source::GhToken, "gho_1".into()))
        );
        assert_eq!(
            pick(&everything[2..], None),
            Some(Found::Token(Source::GithubToken, "ghp_1".into()))
        );
        assert_eq!(
            pick(&everything[3..], None).map(|found| found.source()),
            Some(Source::GithubOidc)
        );
        assert_eq!(pick(&[], None), None);
    }

    #[test]
    fn installation_tokens_are_not_user_tokens() {
        assert_eq!(pick(&[("GITHUB_TOKEN", "ghs_installation")], None), None);
        assert_eq!(
            pick(&[("GH_TOKEN", "github_pat_11A")], None),
            Some(Found::Token(Source::GhToken, "github_pat_11A".into()))
        );
        assert_eq!(pick(&[("STATESPACE_TOKEN", " ")], None), None);
        // Outside Actions, the OIDC variables mean nothing.
        assert_eq!(
            pick(
                &[
                    ("ACTIONS_ID_TOKEN_REQUEST_URL", "u"),
                    ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "t")
                ],
                None
            ),
            None
        );
    }
}
