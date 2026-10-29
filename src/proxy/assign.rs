//! What serves a request: live, or one arm of a running experiment.
//! Decided from the app's runtime configuration, without a network call.

use std::collections::BTreeMap;

use cel_interpreter::{Context, Program, Value as Cel};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{
    config::{ArmConfig, ExperimentConfig, RuntimeConfig, State},
    events::Events,
    orchestrator::{Unit, UnitKey},
};

/// The visitor cookie the proxy sets with the default subject.
pub const SUBJECT_COOKIE: &str = "ssp_sid";
/// The default subject rule.
pub const DEFAULT_SUBJECT: &str = "request.cookies.ssp_sid";
/// The query parameter and cookie that name an arm.
pub const OVERRIDE: &str = "ssp_arm";
/// The header that names the arm of one request.
pub const OVERRIDE_HEADER: &str = "ssp-arm";

/// One experiment of the configuration, compiled.
pub struct Experiment {
    pub name: String,
    pub version: i64,
    pub state: State,
    subject: Option<Program>,
    /// Whether the subject is the visitor cookie, which the proxy then sets.
    pub default_subject: bool,
    eligibility: Option<Program>,
    /// The events of its units: the shared events and its own.
    pub events: Events,
    pub arms: Vec<ArmConfig>,
    /// Why its rules do not compile. Such an experiment takes no request
    /// and runs no unit.
    pub broken: Option<String>,
}

impl Experiment {
    fn compile(config: &ExperimentConfig, warnings: &mut Vec<String>) -> Self {
        let subject_rule = config.subject.as_deref().unwrap_or(DEFAULT_SUBJECT);
        let mut broken = None;
        let subject = compile(subject_rule)
            .map_err(|error| broken = Some(format!("the subject rule does not compile: {error:#}")))
            .ok();
        let eligibility = match config.eligibility.as_deref().map(compile) {
            Some(Err(error)) => {
                broken.get_or_insert(format!("the eligibility rule does not compile: {error:#}"));
                None
            }
            Some(Ok(program)) => Some(program),
            None => None,
        };
        if let Some(problem) = &broken {
            warnings.push(format!(
                "{problem}; experiment {} sends every request to live",
                config.name
            ));
        }
        let (events, errors) = Events::compile(&config.events);
        warnings.extend(
            errors
                .into_iter()
                .map(|error| format!("experiment {}: {error}", config.name)),
        );
        Self {
            name: config.name.clone(),
            version: config.version,
            state: config.state,
            subject,
            default_subject: subject_rule.trim() == DEFAULT_SUBJECT,
            eligibility,
            events,
            arms: config.arms.clone(),
            broken,
        }
    }

    /// Whether the experiment takes requests now.
    pub fn routes(&self) -> bool {
        self.state == State::Running && self.broken.is_none()
    }

    /// Whether the lease holder runs its units.
    fn runs_units(&self) -> bool {
        matches!(self.state, State::Starting | State::Running) && self.broken.is_none()
    }

    /// The subject its rule yields, or `None` when it yields nothing usable.
    pub fn subject(&self, request: &Value) -> Option<String> {
        let subject = match evaluate(self.subject.as_ref()?, "request", request).ok()? {
            Cel::String(value) => value.to_string(),
            Cel::Int(value) => value.to_string(),
            Cel::UInt(value) => value.to_string(),
            _ => return None,
        };
        (!subject.is_empty() && subject.len() <= 128).then_some(subject)
    }

    fn eligible(&self, request: &Value) -> anyhow::Result<bool> {
        match &self.eligibility {
            None => Ok(true),
            Some(program) => {
                evaluate(program, "request", request).map(|value| value == Cel::Bool(true))
            }
        }
    }

    fn arm(&self, commit: &str) -> Option<&ArmConfig> {
        self.arms.iter().find(|arm| arm.commit == commit)
    }
}

/// The compiled runtime configuration of an app.
pub struct Routing {
    pub salt: String,
    pub settings_hash: Option<String>,
    pub overrides: bool,
    pub redact: Vec<Regex>,
    /// The `starting` and `running` experiments, in the order of
    /// `statespace.toml`. Without a live commit, there are none.
    pub experiments: Vec<Experiment>,
    /// Whether a running experiment uses the visitor cookie, which the proxy
    /// then sets.
    pub default_subject: bool,
}

impl Routing {
    /// Compile a configuration. An experiment whose subject or eligibility
    /// rule does not compile takes no request. Event rules and redact
    /// patterns that do not compile are left out. Each problem is returned
    /// as a warning.
    pub fn compile(config: &RuntimeConfig) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut redact = Vec::new();
        for pattern in &config.settings.redact {
            match Regex::new(pattern) {
                Ok(pattern) => redact.push(pattern),
                Err(error) => {
                    warnings.push(format!("redact {pattern:?} does not compile: {error}"))
                }
            }
        }
        let experiments: Vec<Experiment> = match &config.live {
            // Without a live commit, nothing runs beside live.
            None => Vec::new(),
            Some(_) => config
                .experiments
                .iter()
                .map(|experiment| Experiment::compile(experiment, &mut warnings))
                .collect(),
        };
        let default_subject = experiments
            .iter()
            .any(|experiment| experiment.routes() && experiment.default_subject);
        (
            Self {
                salt: config.salt.clone(),
                settings_hash: config.settings_hash.clone(),
                overrides: config.settings.overrides,
                redact,
                experiments,
                default_subject,
            },
            warnings,
        )
    }

    /// The units that should run: each arm of each starting or running
    /// experiment.
    pub fn units(&self) -> Vec<Unit> {
        self.experiments
            .iter()
            .filter(|experiment| experiment.runs_units())
            .flat_map(|experiment| {
                experiment.arms.iter().map(|arm| Unit {
                    experiment: experiment.name.clone(),
                    commit: arm.commit.clone(),
                    image: arm.image.clone(),
                    share: size(&arm.ranges),
                })
            })
            .collect()
    }

    /// The units of experiments whose rules do not compile, and why.
    pub fn broken(&self) -> Vec<(UnitKey, String)> {
        self.experiments
            .iter()
            .filter_map(|experiment| Some((experiment, experiment.broken.clone()?)))
            .flat_map(|(experiment, problem)| {
                experiment.arms.iter().map(move |arm| {
                    (
                        UnitKey {
                            experiment: experiment.name.clone(),
                            commit: arm.commit.clone(),
                        },
                        problem.clone(),
                    )
                })
            })
            .collect()
    }

    pub fn experiment(&self, name: &str) -> Option<&Experiment> {
        self.experiments
            .iter()
            .find(|experiment| experiment.name == name)
    }

    /// The image a unit runs now.
    pub fn image(&self, key: &UnitKey) -> Option<&str> {
        let arm = self.experiment(&key.experiment)?.arm(&key.commit)?;
        Some(arm.image.as_str())
    }

    /// The version of a unit's experiment.
    pub fn version(&self, key: &UnitKey) -> Option<i64> {
        self.experiment(&key.experiment)
            .map(|experiment| experiment.version)
    }

    /// The events that apply to a unit: its experiment's.
    pub fn events(&self, key: &UnitKey) -> Option<&Events> {
        self.experiment(&key.experiment)
            .map(|experiment| &experiment.events)
    }

    /// The subject for the `x-statespace-subject` header of a request that
    /// no experiment took: the first subject a running experiment yields.
    pub fn any_subject(&self, request: &Value) -> Option<String> {
        self.experiments
            .iter()
            .filter(|experiment| experiment.routes())
            .find_map(|experiment| experiment.subject(request))
    }
}

/// The share of the hash space that `ranges` hold.
pub fn size(ranges: &[[f64; 2]]) -> f64 {
    ranges
        .iter()
        .map(|[start, end]| (end - start).max(0.0))
        .sum()
}

pub fn compile(expression: &str) -> anyhow::Result<Program> {
    // The CEL parser panics on some malformed input instead of returning an error.
    match std::panic::catch_unwind(|| Program::compile(expression)) {
        Ok(Ok(program)) => Ok(program),
        Ok(Err(error)) => anyhow::bail!("{error}"),
        Err(_) => anyhow::bail!("could not parse {expression:?}"),
    }
}

/// Run a program with one variable.
pub fn evaluate(program: &Program, name: &str, value: &Value) -> anyhow::Result<Cel> {
    let mut context = Context::default();
    context
        .add_variable(name, value)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let program = std::panic::AssertUnwindSafe(program);
    let context = std::panic::AssertUnwindSafe(&context);
    match std::panic::catch_unwind(|| program.execute(&context)) {
        Ok(result) => result.map_err(|error| anyhow::anyhow!("{error}")),
        Err(_) => anyhow::bail!("the rule failed"),
    }
}

/// Why a subject got what it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    Assigned,
    /// The unit is not ready: the request goes to live.
    ArmUnavailable,
    /// The request named its arm with `?ssp_arm=` or `ssp-arm`.
    Override,
    /// The eligibility rule failed, so the experiment skipped the request.
    EligibilityError,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assigned => "assigned",
            Self::ArmUnavailable => "arm-unavailable",
            Self::Override => "override",
            Self::EligibilityError => "eligibility-error",
        }
    }
}

/// A run to record: the experiment, the arm whose range holds the subject
/// (none for an eligibility error), and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub experiment: String,
    pub arm: Option<String>,
    pub subject: String,
    pub reason: Reason,
}

/// What serves a request.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// The unit that serves the request, or `None` for live.
    pub serve: Option<UnitKey>,
    /// The subject for the `x-statespace-subject` header and the rows.
    pub subject: Option<String>,
    /// The runs to record: eligibility errors, then the run of the
    /// experiment that took the request.
    pub runs: Vec<Run>,
}

impl Decision {
    pub fn live(subject: Option<String>) -> Self {
        Self {
            serve: None,
            subject,
            runs: Vec::new(),
        }
    }
}

/// Decide who serves `request`. Each running experiment, in the order of
/// `statespace.toml`, reads its subject and hashes it; when the point is in
/// a range of one of its arms and the subject is eligible, the experiment
/// takes the request. Else the next experiment. When none takes it, live
/// serves it. `ready` says whether a unit can take traffic; a subject whose
/// unit cannot gets live.
pub fn decide(routing: &Routing, request: &Value, ready: impl Fn(&UnitKey) -> bool) -> Decision {
    let mut runs = Vec::new();
    for experiment in routing.experiments.iter().filter(|e| e.routes()) {
        let Some(subject) = experiment.subject(request) else {
            continue;
        };
        let point = bucket(&routing.salt, &subject);
        let Some(arm) = experiment
            .arms
            .iter()
            .find(|arm| contains(&arm.ranges, point))
        else {
            continue;
        };
        match experiment.eligible(request) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => {
                runs.push(Run {
                    experiment: experiment.name.clone(),
                    arm: None,
                    subject,
                    reason: Reason::EligibilityError,
                });
                continue;
            }
        }
        let key = UnitKey {
            experiment: experiment.name.clone(),
            commit: arm.commit.clone(),
        };
        let reason = if ready(&key) {
            Reason::Assigned
        } else {
            Reason::ArmUnavailable
        };
        runs.push(Run {
            experiment: experiment.name.clone(),
            arm: Some(arm.commit.clone()),
            subject: subject.clone(),
            reason,
        });
        return Decision {
            serve: (reason == Reason::Assigned).then_some(key),
            subject: Some(subject),
            runs,
        };
    }
    Decision {
        serve: None,
        subject: routing.any_subject(request),
        runs,
    }
}

/// Why the arm a request names cannot serve it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The arm runs, but cannot take traffic yet.
    NotReady,
    /// No arm of a running experiment matches the value.
    NotRunning,
}

/// Serve `request` on the arm that `value` names: `<experiment>:<commit>`,
/// the commit full or a prefix of at least 7 characters, that matches one
/// arm of a running experiment. Eligibility and the split do not apply.
pub fn override_arm(
    routing: &Routing,
    request: &Value,
    value: &str,
    ready: impl Fn(&UnitKey) -> bool,
) -> Result<Decision, Refused> {
    let (name, commit) = value.split_once(':').ok_or(Refused::NotRunning)?;
    let commit = commit.to_ascii_lowercase();
    if !(7..=40).contains(&commit.len()) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Refused::NotRunning);
    }
    let experiment = routing
        .experiment(name)
        .filter(|experiment| experiment.routes())
        .ok_or(Refused::NotRunning)?;
    let matches: Vec<&ArmConfig> = experiment
        .arms
        .iter()
        .filter(|arm| arm.commit.starts_with(&commit))
        .collect();
    let [arm] = matches.as_slice() else {
        return Err(Refused::NotRunning);
    };
    let key = UnitKey {
        experiment: experiment.name.clone(),
        commit: arm.commit.clone(),
    };
    if !ready(&key) {
        return Err(Refused::NotReady);
    }
    let subject = experiment.subject(request);
    Ok(Decision {
        runs: subject
            .iter()
            .map(|subject| Run {
                experiment: experiment.name.clone(),
                arm: Some(arm.commit.clone()),
                subject: subject.clone(),
                reason: Reason::Override,
            })
            .collect(),
        serve: Some(key),
        subject,
    })
}

/// The arms of running experiments, as `<experiment>:<commit 7>`, in the
/// order of `statespace.toml`.
pub fn running_arms(routing: &Routing) -> Vec<String> {
    routing
        .experiments
        .iter()
        .filter(|experiment| experiment.routes())
        .flat_map(|experiment| {
            experiment.arms.iter().map(|arm| {
                format!(
                    "{}:{}",
                    experiment.name,
                    arm.commit.chars().take(7).collect::<String>()
                )
            })
        })
        .collect()
}

/// Map a subject to a uniform point in `[0, 1)`: the first 53 bits of
/// `SHA-256(salt:subject_id)`. Never change this function: it would move
/// the subjects of running arms.
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

/// The request as CEL rules see it: `method`, `path`, `query`, `headers`,
/// and `cookies`.
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

    const ARM: &str = "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0";
    const OTHER: &str = "a1b2c3d9e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0";
    const LIVE: &str = "c0ffee1000000000000000000000000000000000";
    const LANDING: &str = "b2c3d4e000000000000000000000000000000000";

    /// `checkout` splits users by `x-user-id`: LIVE in `reference`, ARM in
    /// `arm`. `us-landing` uses the visitor cookie, eligible in the US only.
    fn config(reference: [f64; 2], arm: [f64; 2], landing: [f64; 2]) -> RuntimeConfig {
        serde_json::from_value(json!({
            "app": "shop-prod",
            "salt": "app-salt",
            "settings_hash": "h1",
            "settings": {"overrides": true, "redact": []},
            "live": {"commit": LIVE, "image": "shop@sha256:1"},
            "experiments": [
                {"name": "checkout", "version": 3, "state": "running",
                 "subject": "request.headers[\"x-user-id\"]", "eligibility": null,
                 "events": [],
                 "arms": [
                     {"commit": LIVE, "image": "shop@sha256:1", "ranges": [reference]},
                     {"commit": ARM, "image": "shop@sha256:2", "ranges": [arm]},
                 ]},
                {"name": "us-landing", "version": 1, "state": "running",
                 "subject": null,
                 "eligibility": "request.headers[\"cf-ipcountry\"] == \"US\"",
                 "events": [],
                 "arms": [{"commit": LANDING, "image": "shop@sha256:3", "ranges": [landing]}]},
            ],
            "lease": null,
            "poll_interval_seconds": 5,
        }))
        .unwrap()
    }

    fn routing(config: &RuntimeConfig) -> Routing {
        let (routing, warnings) = Routing::compile(config);
        assert!(warnings.is_empty(), "{warnings:?}");
        routing
    }

    fn request(user: Option<&str>, visitor: Option<&str>, country: &str) -> Value {
        let mut headers = http::HeaderMap::new();
        if let Some(user) = user {
            headers.insert("x-user-id", user.parse().unwrap());
        }
        headers.insert("cf-ipcountry", country.parse().unwrap());
        let mut cookies = Map::new();
        if let Some(visitor) = visitor {
            cookies.insert(SUBJECT_COOKIE.into(), json!(visitor));
        }
        request_value("GET", "/", Some("a=1&b=x%20y"), &headers, &cookies)
    }

    fn key(experiment: &str, commit: &str) -> UnitKey {
        UnitKey {
            experiment: experiment.into(),
            commit: commit.into(),
        }
    }

    #[test]
    fn buckets_are_stable() {
        // Changing this value would move subjects of running experiments.
        assert_eq!(bucket("salt", "u_42"), 0.2531303365722227);
    }

    #[test]
    fn assigns_subjects_by_ranges() {
        let routing = routing(&config([0.0, 0.5], [0.5, 1.0], [0.0, 0.0]));
        let mut served = std::collections::HashMap::new();
        for index in 0..400 {
            let user = format!("u_{index}");
            let decision = decide(&routing, &request(Some(&user), None, "US"), |_| true);
            assert_eq!(
                decision,
                decide(&routing, &request(Some(&user), None, "US"), |_| true)
            );
            let [run] = decision.runs.as_slice() else {
                panic!("{decision:?}")
            };
            assert_eq!(run.reason, Reason::Assigned);
            assert_eq!(
                decision.serve,
                Some(key("checkout", run.arm.as_ref().unwrap()))
            );
            // The ranges decide the arm.
            let point = bucket("app-salt", &user);
            assert_eq!(run.arm.as_deref() == Some(ARM), point >= 0.5);
            *served.entry(run.arm.clone().unwrap()).or_insert(0) += 1;
        }
        assert!((150..250).contains(&served[LIVE]), "{served:?}");
        assert!((150..250).contains(&served[ARM]));
    }

    #[test]
    fn the_first_experiment_in_the_file_wins_a_request_both_claim() {
        // Both experiments claim every subject: checkout is first.
        let routing = routing(&config([0.0, 0.5], [0.5, 1.0], [0.0, 1.0]));
        let both = decide(&routing, &request(Some("u_1"), Some("sid_1"), "US"), |_| {
            true
        });
        assert_eq!(both.serve.unwrap().experiment, "checkout");
        assert_eq!(both.subject.as_deref(), Some("u_1"));
        // Without the user ID, checkout skips the request; us-landing takes it.
        let visitor = decide(&routing, &request(None, Some("sid_1"), "US"), |_| true);
        assert_eq!(visitor.serve, Some(key("us-landing", LANDING)));
        assert_eq!(visitor.subject.as_deref(), Some("sid_1"));
        // Outside the US, us-landing skips it too: live, without a run.
        let elsewhere = decide(&routing, &request(None, Some("sid_1"), "FR"), |_| true);
        assert_eq!(elsewhere, Decision::live(Some("sid_1".into())));
        assert!(routing.default_subject);
    }

    #[test]
    fn subjects_outside_every_range_get_live_without_a_run() {
        let routing = routing(&config([0.0, 0.0], [0.0, 0.0], [0.0, 0.0]));
        let decision = decide(&routing, &request(Some("u_1"), None, "US"), |_| true);
        assert_eq!(decision, Decision::live(Some("u_1".into())));
        assert_eq!(
            decide(&routing, &request(None, None, "US"), |_| true),
            Decision::live(None)
        );
    }

    #[test]
    fn units_that_are_not_ready_and_eligibility_errors_send_subjects_on() {
        let routing = routing(&config([0.0, 0.0], [0.0, 1.0], [0.0, 0.0]));
        let down = decide(&routing, &request(Some("u_1"), None, "US"), |_| false);
        assert_eq!(down.serve, None);
        assert_eq!(down.runs[0].reason, Reason::ArmUnavailable);
        assert_eq!(down.runs[0].arm.as_deref(), Some(ARM));
        assert_eq!(Reason::ArmUnavailable.as_str(), "arm-unavailable");

        let mut broken = config([0.0, 0.0], [0.0, 1.0], [0.0, 1.0]);
        broken.experiments[0].eligibility = Some("request.headers[\"x-user-id\"] + 1 == 2".into());
        let routing = super::tests::routing(&broken);
        let error = decide(&routing, &request(Some("u_1"), Some("sid_1"), "US"), |_| {
            true
        });
        // checkout counts an eligibility error, and us-landing takes the request.
        assert_eq!(error.runs.len(), 2);
        assert_eq!(error.runs[0].reason, Reason::EligibilityError);
        assert_eq!(error.runs[0].arm, None);
        assert_eq!(error.serve, Some(key("us-landing", LANDING)));
    }

    #[test]
    fn starting_experiments_run_units_but_take_no_requests() {
        let mut config = config([0.0, 0.5], [0.5, 1.0], [0.0, 0.0]);
        config.experiments[0].state = State::Starting;
        let routing = routing(&config);
        let units = routing.units();
        assert_eq!(units.len(), 3);
        assert_eq!(units[1].key(), key("checkout", ARM));
        assert!((units[1].share - 0.5).abs() < 1e-12);
        assert_eq!(routing.version(&key("checkout", ARM)), Some(3));
        assert_eq!(
            routing.image(&key("us-landing", LANDING)),
            Some("shop@sha256:3")
        );
        let decision = decide(&routing, &request(Some("u_1"), None, "US"), |_| true);
        assert_eq!(decision.serve, None);
        // Only us-landing runs, and it uses the visitor cookie.
        assert!(routing.default_subject);
        config.experiments.remove(1);
        assert!(!super::tests::routing(&config).default_subject);
    }

    #[test]
    fn one_commit_in_two_experiments_is_two_units() {
        let mut config = config([0.0, 0.1], [0.1, 0.2], [0.2, 0.3]);
        config.experiments[1].arms[0].commit = ARM.into();
        let keys: Vec<UnitKey> = routing(&config).units().iter().map(Unit::key).collect();
        assert_eq!(
            keys,
            [
                key("checkout", LIVE),
                key("checkout", ARM),
                key("us-landing", ARM)
            ]
        );
    }

    #[test]
    fn without_live_everything_goes_to_live() {
        let mut config = config([0.0, 1.0], [1.0, 1.0], [0.0, 0.0]);
        config.live = None;
        let routing = routing(&config);
        assert!(routing.units().is_empty());
        let decision = decide(&routing, &request(Some("u_1"), None, "US"), |_| true);
        assert_eq!(decision, Decision::live(None));
    }

    #[test]
    fn overrides_name_an_experiment_and_a_commit() {
        let mut config = config([0.0, 0.0], [0.0, 0.0], [0.0, 0.0]);
        let routing_one = routing(&config);
        let request = request(Some("u_1"), None, "FR");
        let decision = override_arm(&routing_one, &request, "checkout:A1B2C3D", |_| true).unwrap();
        assert_eq!(decision.serve, Some(key("checkout", ARM)));
        assert_eq!(decision.runs[0].reason, Reason::Override);
        assert_eq!(decision.runs[0].subject, "u_1");
        for value in [
            "a1b2c3d",
            "checkout:a1b2c3",
            "checkout:b2c3d4e",
            "us-landing:a1b2c3d",
            "pricing:a1b2c3d",
            "checkout:a1b2c3d!",
        ] {
            assert_eq!(
                override_arm(&routing_one, &request, value, |_| true),
                Err(Refused::NotRunning),
                "{value}"
            );
        }
        assert_eq!(
            override_arm(&routing_one, &request, &format!("checkout:{ARM}"), |_| {
                false
            }),
            Err(Refused::NotReady)
        );
        // A prefix that matches two arms names none.
        config.experiments[0].arms[0].commit = OTHER.into();
        let routing_two = routing(&config);
        assert_eq!(
            override_arm(&routing_two, &request, "checkout:a1b2c3d", |_| true),
            Err(Refused::NotRunning)
        );
        assert!(override_arm(&routing_two, &request, "checkout:a1b2c3d4", |_| true).is_ok());
        assert_eq!(
            running_arms(&routing_two),
            ["checkout:a1b2c3d", "checkout:a1b2c3d", "us-landing:b2c3d4e"]
        );
        // A starting experiment takes no override.
        config.experiments[0].state = State::Starting;
        assert_eq!(
            override_arm(&routing(&config), &request, "checkout:a1b2c3d4", |_| true),
            Err(Refused::NotRunning)
        );
        // Without a subject, the arm still serves, and no run is recorded.
        let anonymous = request_value("GET", "/", None, &http::HeaderMap::new(), &Map::new());
        let decision = override_arm(&routing_one, &anonymous, &format!("checkout:{ARM}"), |_| {
            true
        })
        .unwrap();
        assert!(decision.runs.is_empty());
    }

    #[test]
    fn experiments_whose_rules_do_not_compile_take_nothing() {
        let mut config = config([0.0, 1.0], [1.0, 1.0], [0.0, 1.0]);
        config.experiments[0].subject = Some("request.headers[".into());
        let (routing, warnings) = Routing::compile(&config);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("experiment checkout"), "{warnings:?}");
        assert_eq!(routing.units().len(), 1);
        assert_eq!(routing.broken().len(), 2);
        let decision = decide(&routing, &request(Some("u_1"), Some("sid_1"), "US"), |_| {
            true
        });
        assert_eq!(decision.serve, Some(key("us-landing", LANDING)));
    }

    #[test]
    fn requests_expose_their_parts() {
        let value = request(Some("u_1"), None, "US");
        assert_eq!(value["query"]["b"], "x y");
        assert_eq!(decode("%E2%9C%93+%zz%"), "\u{2713} %zz%");
        assert_eq!(value["headers"]["x-user-id"], "u_1");
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::COOKIE, "a=1; ssp_sid=abc".parse().unwrap());
        assert_eq!(cookies(&headers)[SUBJECT_COOKIE], "abc");
    }
}
