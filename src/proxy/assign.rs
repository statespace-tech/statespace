//! What serves a request: the subject, its experiment, and its arm,
//! decided from the app's configuration without a network call.

use std::collections::BTreeMap;

use anyhow::Context as _;
use cel_interpreter::{Context, Program, Value as Cel};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// The cookie the proxy sets so anonymous visitors keep their arm.
pub const SUBJECT_COOKIE: &str = "ssp_sid";
/// The query parameter and cookie that override the arm, with `--overrides`.
pub const OVERRIDE: &str = "ssp_arm";
/// The header that overrides the arm of one request, with `--overrides`.
pub const OVERRIDE_HEADER: &str = "ssp-arm";

/// An app's running configuration, as the API returns it.
#[derive(Clone, Debug, Deserialize)]
pub struct HostConfig {
    pub salt: String,
    pub experiments: Vec<ExperimentConfig>,
    pub poll_interval_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExperimentConfig {
    pub name: String,
    pub version: u32,
    pub subject: String,
    pub eligibility: Option<String>,
    pub arms: Vec<ArmConfig>,
}

/// An arm of a running version: a variant, or the control of a base commit.
#[derive(Clone, Debug, Deserialize)]
pub struct ArmConfig {
    /// A variant, or `control` for the control of a base commit.
    pub name: String,
    /// The `exp/` branch of a variant, such as `exp/price/lower`. Controls
    /// have none.
    #[serde(default)]
    pub branch: Option<String>,
    pub commit: String,
    /// The image by digest.
    pub image: String,
    /// Half-open `[start, end)` intervals of the app's hash space.
    pub ranges: Vec<[f64; 2]>,
    /// Where a controller runs the arm.
    pub address: Option<String>,
    pub ready: bool,
}

/// One arm of one experiment version: a variant at a commit, or the control
/// of a base commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub experiment: String,
    pub experiment_version: u32,
    /// `control` or a variant.
    pub arm: String,
    pub commit: String,
}

impl std::fmt::Display for Key {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}@{} of {}@{}",
            self.arm,
            &self.commit[..self.commit.len().min(7)],
            self.experiment,
            self.experiment_version
        )
    }
}

/// What serves a request: the app's default, or an arm.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    Default,
    Arm(Key),
}

impl std::fmt::Display for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(formatter, "default"),
            Self::Arm(key) => key.fmt(formatter),
        }
    }
}

/// An experiment with its rules compiled.
pub struct Compiled {
    pub config: ExperimentConfig,
    subject: Program,
    eligibility: Option<Program>,
}

/// The compiled configuration of an app.
#[derive(Default)]
pub struct Routing {
    pub salt: String,
    pub experiments: Vec<Compiled>,
}

impl Routing {
    /// Compile every experiment. One with a rule that does not compile is
    /// skipped, so its subjects get the default, and reported in the result.
    pub fn compile(config: &HostConfig) -> (Self, Vec<String>) {
        let mut errors = Vec::new();
        let mut experiments = Vec::new();
        for experiment in &config.experiments {
            match compile_experiment(experiment) {
                Ok(compiled) => experiments.push(compiled),
                Err(error) => errors.push(format!("{}: {error:#}", experiment.name)),
            }
        }
        (
            Self {
                salt: config.salt.clone(),
                experiments,
            },
            errors,
        )
    }

    /// Every arm of the running experiments.
    pub fn arms(&self) -> Vec<(Key, &ArmConfig)> {
        self.experiments
            .iter()
            .flat_map(|experiment| {
                experiment
                    .config
                    .arms
                    .iter()
                    .map(|arm| (key(&experiment.config, arm), arm))
            })
            .collect()
    }
}

fn key(experiment: &ExperimentConfig, arm: &ArmConfig) -> Key {
    Key {
        experiment: experiment.name.clone(),
        experiment_version: experiment.version,
        arm: arm.name.clone(),
        commit: arm.commit.clone(),
    }
}

/// The share of all the app's subjects an arm gets.
pub fn share(arm: &ArmConfig) -> f64 {
    arm.ranges.iter().map(|[start, end]| end - start).sum()
}

fn compile_experiment(config: &ExperimentConfig) -> anyhow::Result<Compiled> {
    Ok(Compiled {
        subject: compile(&config.subject).context("invalid subject rule")?,
        eligibility: config
            .eligibility
            .as_deref()
            .map(compile)
            .transpose()
            .context("invalid eligibility rule")?,
        config: config.clone(),
    })
}

fn compile(expression: &str) -> anyhow::Result<Program> {
    // The CEL parser panics on some malformed input instead of returning an error.
    match std::panic::catch_unwind(|| Program::compile(expression)) {
        Ok(Ok(program)) => Ok(program),
        Ok(Err(error)) => anyhow::bail!("{error}"),
        Err(_) => anyhow::bail!("could not parse {expression:?}"),
    }
}

/// Why a subject got what it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Assigned,
    Ineligible,
    EligibilityError,
    ArmUnavailable,
    /// The request named its arm with `?ssp_arm=` or `ssp-arm`.
    Override,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assigned => "assigned",
            Self::Ineligible => "ineligible",
            Self::EligibilityError => "eligibility-error",
            Self::ArmUnavailable => "arm-unavailable",
            Self::Override => "override",
        }
    }
}

/// The experiment a request belongs to and what it decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Assignment {
    pub experiment: String,
    pub version: u32,
    pub subject: String,
    /// The arm the subject is assigned to, or `None` when ineligible.
    pub assigned: Option<Key>,
    pub reason: Reason,
}

/// What serves a request, and the assignment behind it, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub serve: Target,
    pub assignment: Option<Assignment>,
}

/// Decide who serves `request`. `ready` says whether an arm can take
/// traffic; a subject whose arm cannot gets the default.
pub fn decide(routing: &Routing, request: &Value, ready: impl Fn(&Key) -> bool) -> Decision {
    let default = Decision {
        serve: Target::Default,
        assignment: None,
    };
    // Arms of running experiments hold disjoint ranges of the app's hash
    // space, so a subject is in at most one experiment.
    let Some((experiment, subject, arm)) = routing.experiments.iter().find_map(|experiment| {
        let subject = subject_of(&experiment.subject, request)?;
        let point = bucket(&routing.salt, &subject);
        let arm = experiment
            .config
            .arms
            .iter()
            .find(|arm| contains(&arm.ranges, point))?;
        Some((experiment, subject, arm))
    }) else {
        return default;
    };
    let config = &experiment.config;
    let assignment = |assigned, reason| Assignment {
        experiment: config.name.clone(),
        version: config.version,
        subject: subject.clone(),
        assigned,
        reason,
    };
    let eligible = match &experiment.eligibility {
        None => Ok(true),
        Some(program) => evaluate(program, request).map(|value| value == Cel::Bool(true)),
    };
    match eligible {
        Ok(true) => {}
        Ok(false) => {
            return Decision {
                serve: Target::Default,
                assignment: Some(assignment(None, Reason::Ineligible)),
            };
        }
        Err(_) => {
            return Decision {
                serve: Target::Default,
                assignment: Some(assignment(None, Reason::EligibilityError)),
            };
        }
    }
    let key = key(config, arm);
    if !ready(&key) {
        return Decision {
            serve: Target::Default,
            assignment: Some(assignment(Some(key), Reason::ArmUnavailable)),
        };
    }
    Decision {
        serve: Target::Arm(key.clone()),
        assignment: Some(assignment(Some(key), Reason::Assigned)),
    }
}

/// Why the arm a request names cannot serve it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The branch runs, but its arm cannot take traffic yet.
    NotReady,
    /// No running arm has the branch.
    NotRunning,
}

/// Serve `request` on the running arm of `branch`, without eligibility or
/// the split. The subject is read as usual, so the run is recorded, with the
/// reason `override`.
pub fn override_arm(
    routing: &Routing,
    request: &Value,
    branch: &str,
    ready: impl Fn(&Key) -> bool,
) -> Result<Decision, Refused> {
    let Some((experiment, arm)) = routing.experiments.iter().find_map(|experiment| {
        let arm = experiment
            .config
            .arms
            .iter()
            .find(|arm| arm.branch.as_deref() == Some(branch))?;
        Some((experiment, arm))
    }) else {
        return Err(Refused::NotRunning);
    };
    let key = key(&experiment.config, arm);
    if !ready(&key) {
        return Err(Refused::NotReady);
    }
    let assignment = subject_of(&experiment.subject, request).map(|subject| Assignment {
        experiment: experiment.config.name.clone(),
        version: experiment.config.version,
        subject,
        assigned: Some(key.clone()),
        reason: Reason::Override,
    });
    Ok(Decision {
        serve: Target::Arm(key),
        assignment,
    })
}

/// The branches of the running arms, sorted.
pub fn running_branches(routing: &Routing) -> Vec<String> {
    let mut branches: Vec<String> = routing
        .arms()
        .into_iter()
        .filter_map(|(_, arm)| arm.branch.clone())
        .collect();
    branches.sort();
    branches.dedup();
    branches
}

/// Whether a CEL rule compiles.
pub fn check_rule(expression: &str) -> anyhow::Result<()> {
    compile(expression).map(drop)
}

/// The subject ID a rule yields, or `None` when it yields nothing usable.
fn subject_of(program: &Program, request: &Value) -> Option<String> {
    let subject = match evaluate(program, request).ok()? {
        Cel::String(value) => value.to_string(),
        Cel::Int(value) => value.to_string(),
        Cel::UInt(value) => value.to_string(),
        _ => return None,
    };
    (!subject.is_empty() && subject.len() <= 128).then_some(subject)
}

fn evaluate(program: &Program, request: &Value) -> anyhow::Result<Cel> {
    let mut context = Context::default();
    context
        .add_variable("request", request)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    program
        .execute(&context)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

/// Map a subject to a uniform point in `[0, 1)`: the first 53 bits of
/// `SHA-256(salt:subject_id)`, as every Statespace client computes it.
pub fn bucket(salt: &str, subject_id: &str) -> f64 {
    let digest = Sha256::digest(format!("{salt}:{subject_id}").as_bytes());
    let mut prefix = [0; 8];
    prefix.copy_from_slice(&digest[..8]);
    (u64::from_be_bytes(prefix) >> 11) as f64 / (1_u64 << 53) as f64
}

fn contains(ranges: &[[f64; 2]], point: f64) -> bool {
    ranges
        .iter()
        .any(|[start, end]| *start <= point && point < *end)
}

/// The request as CEL rules see it.
pub fn request_value(
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &http::HeaderMap,
    cookies: &Map<String, Value>,
) -> Value {
    let mut header_values: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let Ok(value) = value.to_str() else {
            continue;
        };
        header_values
            .entry(name.as_str().to_owned())
            .and_modify(|joined| {
                joined.push_str(", ");
                joined.push_str(value);
            })
            .or_insert_with(|| value.to_owned());
    }
    let query: Map<String, Value> = query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key), json!(decode(value)))
        })
        .collect();
    json!({
        "method": method,
        "path": path,
        "query": query,
        "headers": header_values,
        "cookies": cookies,
    })
}

/// The cookies of a request.
pub fn cookies(headers: &http::HeaderMap) -> Map<String, Value> {
    headers
        .get_all(http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .map(|(name, value)| (name.to_owned(), json!(value)))
        .collect()
}

/// Percent-decode a query component, treating `+` as a space.
pub fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let hex = |byte: u8| char::from(byte).to_digit(16).map(|digit| digit as u8);
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%' && index + 2 < bytes.len())
            .then(|| Some(hex(bytes[index + 1])? * 16 + hex(bytes[index + 2])?))
            .flatten();
        match (bytes[index], escaped) {
            (_, Some(byte)) => {
                decoded.push(byte);
                index += 2;
            }
            (b'+', None) => decoded.push(b' '),
            (byte, None) => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// An experiment whose variant `fast` and its control hold `fast` and
    /// `control` of the app's hash space.
    fn config(fast: [f64; 2], control: [f64; 2], eligibility: Option<&str>) -> HostConfig {
        let arm = |name: &str, commit: &str, range: [f64; 2]| ArmConfig {
            name: name.into(),
            branch: (name != "control").then(|| format!("exp/checkout/{name}")),
            commit: commit.into(),
            image: String::new(),
            ranges: vec![range],
            address: None,
            ready: false,
        };
        HostConfig {
            salt: "app-salt".into(),
            poll_interval_seconds: 10,
            experiments: vec![ExperimentConfig {
                name: "checkout".into(),
                version: 3,
                subject: "request.headers[\"x-user-id\"]".into(),
                eligibility: eligibility.map(Into::into),
                arms: vec![arm("fast", HEAD, fast), arm("control", BASE, control)],
            }],
        }
    }

    fn request(user: Option<&str>, country: &str) -> Value {
        let mut headers = http::HeaderMap::new();
        if let Some(user) = user {
            headers.insert("x-user-id", user.parse().unwrap());
        }
        headers.insert("cf-ipcountry", country.parse().unwrap());
        request_value("GET", "/", Some("a=1&b=x%20y"), &headers, &Map::new())
    }

    #[test]
    fn buckets_are_stable() {
        // Changing this value would move subjects of running experiments.
        assert_eq!(bucket("salt", "u_42"), 0.2531303365722227);
    }

    #[test]
    fn assigns_subjects_stably_by_their_buckets() {
        let (routing, errors) = Routing::compile(&config([0.0, 0.5], [0.5, 1.0], None));
        assert!(errors.is_empty());
        let mut served = std::collections::HashMap::new();
        for index in 0..400 {
            let user = format!("u_{index}");
            let decision = decide(&routing, &request(Some(&user), "US"), |_| true);
            assert_eq!(
                decision,
                decide(&routing, &request(Some(&user), "US"), |_| true)
            );
            let assignment = decision.assignment.unwrap();
            assert_eq!(assignment.reason, Reason::Assigned);
            let Target::Arm(arm) = decision.serve else {
                panic!("an assigned subject gets an arm");
            };
            assert_eq!(assignment.assigned.as_ref(), Some(&arm));
            *served.entry(arm.arm).or_insert(0) += 1;
        }
        // Half of the subjects each, give or take sampling noise; control
        // is an arm of its own.
        assert!((150..250).contains(&served["fast"]), "{served:?}");
        assert!((150..250).contains(&served["control"]), "{served:?}");
    }

    #[test]
    fn subjects_outside_the_slice_get_the_default_without_a_run() {
        let (routing, _) = Routing::compile(&config([0.0, 0.0], [0.0, 0.0], None));
        let decision = decide(&routing, &request(Some("u_1"), "US"), |_| true);
        assert_eq!(
            (decision.serve, decision.assignment),
            (Target::Default, None)
        );
        let no_subject = decide(&routing, &request(None, "US"), |_| true);
        assert_eq!(no_subject.assignment, None);
    }

    #[test]
    fn eligibility_and_readiness_send_subjects_to_the_default() {
        let rule = Some("request.headers[\"cf-ipcountry\"] == \"US\"");
        let (routing, _) = Routing::compile(&config([0.0, 1.0], [1.0, 1.0], rule));
        let elsewhere = decide(&routing, &request(Some("u_1"), "FR"), |_| true);
        assert_eq!(elsewhere.serve, Target::Default);
        assert_eq!(elsewhere.assignment.unwrap().reason, Reason::Ineligible);

        let down = decide(&routing, &request(Some("u_1"), "US"), |_| false);
        assert_eq!(down.serve, Target::Default);
        let assignment = down.assignment.unwrap();
        assert_eq!(assignment.reason, Reason::ArmUnavailable);
        assert_eq!(assignment.assigned.unwrap().arm, "fast");

        // A missing header is null, but adding a number to text is an error.
        let broken = Some("request.headers[\"x-user-id\"] + 1 == 2");
        let (routing, _) = Routing::compile(&config([0.0, 1.0], [1.0, 1.0], broken));
        let error = decide(&routing, &request(Some("u_1"), "US"), |_| true);
        assert_eq!(error.assignment.unwrap().reason, Reason::EligibilityError);
    }

    #[test]
    fn lists_each_experiments_arms_and_their_shares() {
        let (routing, _) = Routing::compile(&config([0.0, 0.05], [0.05, 0.1], None));
        let arms = routing.arms();
        let names: Vec<String> = arms.iter().map(|(key, _)| key.to_string()).collect();
        assert_eq!(
            names,
            [
                "fast@aaaaaaa of checkout@3",
                "control@bbbbbbb of checkout@3"
            ]
        );
        assert!((share(arms[0].1) - 0.05).abs() < 1e-12);
        assert_eq!(arms[1].0.arm, "control");
    }

    #[test]
    fn overrides_serve_a_running_arm_without_the_split() {
        // The subject is outside every slice and not eligible, yet the
        // override serves the arm and records the run as an override.
        let rule = Some("request.headers[\"cf-ipcountry\"] == \"US\"");
        let (routing, _) = Routing::compile(&config([0.0, 0.0], [0.0, 0.0], rule));
        let request = request(Some("u_1"), "FR");
        let decision = override_arm(&routing, &request, "exp/checkout/fast", |_| true).unwrap();
        let Target::Arm(key) = &decision.serve else {
            panic!("an override is served by its arm");
        };
        assert_eq!(key.arm, "fast");
        let assignment = decision.assignment.unwrap();
        assert_eq!(assignment.reason, Reason::Override);
        assert_eq!(assignment.reason.as_str(), "override");
        assert_eq!(assignment.subject, "u_1");

        assert_eq!(
            override_arm(&routing, &request, "exp/checkout/slow", |_| true),
            Err(Refused::NotRunning)
        );
        assert_eq!(
            override_arm(&routing, &request, "control", |_| true),
            Err(Refused::NotRunning)
        );
        assert_eq!(
            override_arm(&routing, &request, "exp/checkout/fast", |_| false),
            Err(Refused::NotReady)
        );
        assert_eq!(running_branches(&routing), ["exp/checkout/fast"]);

        // Without a subject, the arm still serves, and no run is recorded.
        let anonymous = request_value("GET", "/", None, &http::HeaderMap::new(), &Map::new());
        let decision = override_arm(&routing, &anonymous, "exp/checkout/fast", |_| true).unwrap();
        assert_eq!(decision.assignment, None);
    }

    #[test]
    fn rules_that_do_not_compile_are_skipped() {
        let mut broken = config([0.0, 1.0], [1.0, 1.0], None);
        broken.experiments[0].subject = "request.headers[".into();
        let (routing, errors) = Routing::compile(&broken);
        assert!(routing.experiments.is_empty());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn requests_expose_their_parts() {
        let value = request(Some("u_1"), "US");
        assert_eq!(value["query"]["b"], "x y");
        assert_eq!(decode("%E2%9C%93+%zz%"), "\u{2713} %zz%");
        assert_eq!(value["headers"]["x-user-id"], "u_1");
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::COOKIE, "a=1; ssp_sid=abc".parse().unwrap());
        assert_eq!(cookies(&headers)[SUBJECT_COOKIE], "abc");
    }
}
