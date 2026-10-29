//! What serves a request: live, the control, or an arm. Decided from the
//! app's runtime configuration, without a network call.

use std::collections::BTreeMap;

use anyhow::Context as _;
use cel_interpreter::{Context, Program, Value as Cel};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{
    config::{ArmConfig, ControlConfig, Live, RuntimeConfig},
    events::Events,
    orchestrator::{Side, Unit, UnitKey},
};

/// The visitor cookie the proxy sets with the default subject.
pub const SUBJECT_COOKIE: &str = "ssp_sid";
/// The default subject rule.
pub const DEFAULT_SUBJECT: &str = "request.cookies.ssp_sid";
/// The query parameter and cookie that name an arm.
pub const OVERRIDE: &str = "ssp_arm";
/// The header that names the arm of one request.
pub const OVERRIDE_HEADER: &str = "ssp-arm";

/// The compiled runtime configuration of an app.
pub struct Routing {
    pub salt: String,
    pub settings_hash: Option<String>,
    subject: Program,
    /// Whether the subject is the visitor cookie, which the proxy then sets.
    pub default_subject: bool,
    eligibility: Option<Program>,
    pub overrides: bool,
    pub redact: Vec<Regex>,
    pub events: Events,
    pub live: Option<Live>,
    pub control: Option<ControlConfig>,
    /// The arms that run, at most `max_arms`.
    pub arms: Vec<ArmConfig>,
}

impl Routing {
    /// Compile a configuration. A subject or eligibility rule that does not
    /// compile makes the whole configuration unusable: every request goes
    /// to live. Event rules and redact patterns that do not compile are
    /// left out and returned as warnings.
    pub fn compile(config: &RuntimeConfig) -> anyhow::Result<(Self, Vec<String>)> {
        let settings = &config.settings;
        let subject_rule = settings.subject.as_deref().unwrap_or(DEFAULT_SUBJECT);
        let subject = compile(subject_rule).context("the subject rule does not compile")?;
        let eligibility = settings
            .eligibility
            .as_deref()
            .map(compile)
            .transpose()
            .context("the eligibility rule does not compile")?;
        let mut warnings = Vec::new();
        let mut redact = Vec::new();
        for pattern in &settings.redact {
            match Regex::new(pattern) {
                Ok(pattern) => redact.push(pattern),
                Err(error) => {
                    warnings.push(format!("redact {pattern:?} does not compile: {error}"))
                }
            }
        }
        let (events, errors) = Events::compile(&config.events);
        warnings.extend(errors);
        let max_arms = settings.max_arms.unwrap_or(5).max(1) as usize;
        let (live, control, arms) = match &config.live {
            // Without a live commit, nothing runs beside live.
            None => (None, None, Vec::new()),
            Some(live) => (
                Some(live.clone()),
                config.control.clone(),
                config.arms.iter().take(max_arms).cloned().collect(),
            ),
        };
        if config.arms.len() > max_arms {
            warnings.push(format!(
                "the configuration has {} arms; max_arms is {max_arms}",
                config.arms.len()
            ));
        }
        Ok((
            Self {
                salt: config.salt.clone(),
                settings_hash: config.settings_hash.clone(),
                subject,
                default_subject: subject_rule.trim() == DEFAULT_SUBJECT,
                eligibility,
                overrides: settings.overrides,
                redact,
                events,
                live,
                control,
                arms,
            },
            warnings,
        ))
    }

    /// The units that should run: the control and each arm.
    pub fn units(&self) -> Vec<Unit> {
        let Some(live) = &self.live else {
            return Vec::new();
        };
        let mut units = Vec::new();
        if let Some(control) = &self.control {
            units.push(Unit {
                side: Side::Control,
                commit: live.commit.clone(),
                image: control.image.clone(),
                share: size(&control.ranges),
            });
        }
        for arm in &self.arms {
            units.push(Unit {
                side: Side::Arm,
                commit: arm.commit.clone(),
                image: arm.image.clone(),
                share: size(&arm.ranges),
            });
        }
        units
    }

    /// The image a unit runs now.
    pub fn image(&self, key: &UnitKey) -> Option<&str> {
        match key {
            UnitKey::Control => self.control.as_ref().map(|control| control.image.as_str()),
            UnitKey::Arm(commit) => self
                .arms
                .iter()
                .find(|arm| &arm.commit == commit)
                .map(|arm| arm.image.as_str()),
        }
    }

    /// The version of an arm. The control has none.
    pub fn version(&self, key: &UnitKey) -> Option<i64> {
        match key {
            UnitKey::Control => None,
            UnitKey::Arm(commit) => self
                .arms
                .iter()
                .find(|arm| &arm.commit == commit)
                .map(|arm| arm.version),
        }
    }

    /// The live commit.
    pub fn live_commit(&self) -> Option<&str> {
        self.live.as_ref().map(|live| live.commit.as_str())
    }

    /// The subject a rule yields, or `None` when it yields nothing usable.
    pub fn subject(&self, request: &Value) -> Option<String> {
        let subject = match evaluate(&self.subject, "request", request).ok()? {
            Cel::String(value) => value.to_string(),
            Cel::Int(value) => value.to_string(),
            Cel::UInt(value) => value.to_string(),
            _ => return None,
        };
        (!subject.is_empty() && subject.len() <= 128).then_some(subject)
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

/// A run to record: the unit whose range holds the subject, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub unit: UnitKey,
    pub reason: Reason,
}

/// What serves a request.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// The unit that serves the request, or `None` for live.
    pub serve: Option<UnitKey>,
    pub subject: Option<String>,
    pub run: Option<Run>,
}

impl Decision {
    pub fn live(subject: Option<String>) -> Self {
        Self {
            serve: None,
            subject,
            run: None,
        }
    }
}

/// Decide who serves `request`. `ready` says whether a unit can take
/// traffic; a subject whose unit cannot gets live.
pub fn decide(routing: &Routing, request: &Value, ready: impl Fn(&UnitKey) -> bool) -> Decision {
    let Some(subject) = routing.subject(request) else {
        return Decision::live(None);
    };
    let point = bucket(&routing.salt, &subject);
    let unit = routing
        .control
        .as_ref()
        .filter(|control| contains(&control.ranges, point))
        .map(|_| UnitKey::Control)
        .or_else(|| {
            routing
                .arms
                .iter()
                .find(|arm| contains(&arm.ranges, point))
                .map(|arm| UnitKey::Arm(arm.commit.clone()))
        });
    let Some(unit) = unit.filter(|_| routing.live.is_some()) else {
        return Decision::live(Some(subject));
    };
    let eligible = match &routing.eligibility {
        None => Ok(true),
        Some(program) => {
            evaluate(program, "request", request).map(|value| value == Cel::Bool(true))
        }
    };
    let reason = match eligible {
        Ok(true) if ready(&unit) => Reason::Assigned,
        Ok(true) => Reason::ArmUnavailable,
        Ok(false) => Reason::Ineligible,
        Err(_) => Reason::EligibilityError,
    };
    Decision {
        serve: (reason == Reason::Assigned).then(|| unit.clone()),
        subject: Some(subject),
        run: Some(Run { unit, reason }),
    }
}

/// Why the arm a request names cannot serve it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The arm runs, but cannot take traffic yet.
    NotReady,
    /// No running arm matches the value.
    NotRunning,
}

/// Serve `request` on the running arm whose commit starts with `value`, a
/// prefix of at least 7 characters, without eligibility or the split.
pub fn override_arm(
    routing: &Routing,
    request: &Value,
    value: &str,
    ready: impl Fn(&UnitKey) -> bool,
) -> Result<Decision, Refused> {
    let value = value.to_ascii_lowercase();
    if !(7..=40).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Refused::NotRunning);
    }
    let matches: Vec<&ArmConfig> = routing
        .arms
        .iter()
        .filter(|arm| arm.commit.starts_with(&value))
        .collect();
    let [arm] = matches.as_slice() else {
        return Err(Refused::NotRunning);
    };
    let unit = UnitKey::Arm(arm.commit.clone());
    if !ready(&unit) {
        return Err(Refused::NotReady);
    }
    let subject = routing.subject(request);
    Ok(Decision {
        serve: Some(unit.clone()),
        run: subject.as_ref().map(|_| Run {
            unit,
            reason: Reason::Override,
        }),
        subject,
    })
}

/// The running arms, by their first 7 characters, sorted.
pub fn running_arms(routing: &Routing) -> Vec<String> {
    let mut arms: Vec<String> = routing
        .arms
        .iter()
        .map(|arm| arm.commit.chars().take(7).collect())
        .collect();
    arms.sort();
    arms
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
    const LIVE: &str = "9f1ce8a000000000000000000000000000000000";

    fn config(arm: [f64; 2], control: [f64; 2], eligibility: Option<&str>) -> RuntimeConfig {
        serde_json::from_value(json!({
            "app": "shop-prod",
            "salt": "app-salt",
            "settings_hash": "h1",
            "settings": {
                "subject": "request.headers[\"x-user-id\"]",
                "eligibility": eligibility,
                "overrides": true,
                "redact": [],
                "max_arms": 5,
            },
            "events": [],
            "live": {"commit": LIVE, "image": "shop@sha256:1"},
            "control": {"image": "shop@sha256:1", "ranges": [control]},
            "arms": [{"commit": ARM, "image": "shop@sha256:2", "version": 3, "ranges": [arm]}],
            "lease": null,
            "poll_interval_seconds": 5,
        }))
        .unwrap()
    }

    fn routing(config: &RuntimeConfig) -> Routing {
        Routing::compile(config).unwrap().0
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
        // Changing this value would move subjects of running arms.
        assert_eq!(bucket("salt", "u_42"), 0.2531303365722227);
    }

    #[test]
    fn assigns_subjects_by_ranges() {
        let routing = routing(&config([0.0, 0.5], [0.5, 1.0], None));
        let mut served = std::collections::HashMap::new();
        for index in 0..400 {
            let user = format!("u_{index}");
            let decision = decide(&routing, &request(Some(&user), "US"), |_| true);
            assert_eq!(
                decision,
                decide(&routing, &request(Some(&user), "US"), |_| true)
            );
            let run = decision.run.unwrap();
            assert_eq!(run.reason, Reason::Assigned);
            assert_eq!(decision.serve.as_ref(), Some(&run.unit));
            // The ranges decide the side.
            let point = bucket("app-salt", &user);
            assert_eq!(run.unit == UnitKey::Control, point >= 0.5);
            *served.entry(run.unit).or_insert(0) += 1;
        }
        assert!(
            (150..250).contains(&served[&UnitKey::Control]),
            "{served:?}"
        );
        assert!((150..250).contains(&served[&UnitKey::Arm(ARM.into())]));
    }

    #[test]
    fn subjects_outside_every_range_get_live_without_a_run() {
        let routing = routing(&config([0.0, 0.0], [0.0, 0.0], None));
        let decision = decide(&routing, &request(Some("u_1"), "US"), |_| true);
        assert_eq!(decision, Decision::live(Some("u_1".into())));
        assert_eq!(
            decide(&routing, &request(None, "US"), |_| true),
            Decision::live(None)
        );
    }

    #[test]
    fn reasons_send_subjects_to_live() {
        let rule = Some("request.headers[\"cf-ipcountry\"] == \"US\"");
        let routing = routing(&config([0.0, 1.0], [1.0, 1.0], rule));
        let elsewhere = decide(&routing, &request(Some("u_1"), "FR"), |_| true);
        assert_eq!(elsewhere.serve, None);
        assert_eq!(elsewhere.run.unwrap().reason, Reason::Ineligible);

        let down = decide(&routing, &request(Some("u_1"), "US"), |_| false);
        assert_eq!(down.serve, None);
        let run = down.run.unwrap();
        assert_eq!(run.reason, Reason::ArmUnavailable);
        assert_eq!(run.unit, UnitKey::Arm(ARM.into()));
        assert_eq!(run.reason.as_str(), "arm-unavailable");

        let broken = Some("request.headers[\"x-user-id\"] + 1 == 2");
        let routing = super::tests::routing(&config([0.0, 1.0], [1.0, 1.0], broken));
        let error = decide(&routing, &request(Some("u_1"), "US"), |_| true);
        assert_eq!(error.serve, None);
        assert_eq!(error.run.unwrap().reason, Reason::EligibilityError);
    }

    #[test]
    fn without_live_everything_goes_to_live() {
        let mut config = config([0.0, 1.0], [1.0, 1.0], None);
        config.live = None;
        let routing = routing(&config);
        assert!(routing.units().is_empty());
        let decision = decide(&routing, &request(Some("u_1"), "US"), |_| true);
        assert_eq!(decision.serve, None);
        assert_eq!(decision.run, None);
    }

    #[test]
    fn units_follow_the_configuration_and_max_arms() {
        let mut config = config([0.0, 0.05], [0.05, 0.1], None);
        let units = routing(&config).units();
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].side, Side::Control);
        assert_eq!(units[0].commit, LIVE);
        assert!((units[0].share - 0.05).abs() < 1e-12);
        assert_eq!(units[1].key(), UnitKey::Arm(ARM.into()));
        config.settings.max_arms = Some(1);
        let mut second = config.arms[0].clone();
        second.commit = OTHER.into();
        config.arms.push(second);
        let (routing, warnings) = Routing::compile(&config).unwrap();
        assert_eq!(routing.arms.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(routing.version(&UnitKey::Arm(ARM.into())), Some(3));
        assert_eq!(routing.version(&UnitKey::Control), None);
    }

    #[test]
    fn overrides_name_one_running_arm_by_commit() {
        let rule = Some("request.headers[\"cf-ipcountry\"] == \"US\"");
        let mut config = config([0.0, 0.0], [0.0, 0.0], rule);
        let routing_one = routing(&config);
        let request = request(Some("u_1"), "FR");
        let decision = override_arm(&routing_one, &request, "A1B2C3D", |_| true).unwrap();
        assert_eq!(decision.serve, Some(UnitKey::Arm(ARM.into())));
        let run = decision.run.unwrap();
        assert_eq!(run.reason, Reason::Override);
        for value in ["a1b2c3", "b2c3d4e", "control", "a1b2c3d!"] {
            assert_eq!(
                override_arm(&routing_one, &request, value, |_| true),
                Err(Refused::NotRunning),
                "{value}"
            );
        }
        assert_eq!(
            override_arm(&routing_one, &request, ARM, |_| false),
            Err(Refused::NotReady)
        );
        // A prefix that matches two arms names none.
        let mut second = config.arms[0].clone();
        second.commit = OTHER.into();
        config.arms.push(second);
        let routing_two = routing(&config);
        assert_eq!(
            override_arm(&routing_two, &request, "a1b2c3d", |_| true),
            Err(Refused::NotRunning)
        );
        assert!(override_arm(&routing_two, &request, "a1b2c3d4", |_| true).is_ok());
        assert_eq!(running_arms(&routing_two), ["a1b2c3d", "a1b2c3d"]);
        // Without a subject, the arm still serves, and no run is recorded.
        let anonymous = request_value("GET", "/", None, &http::HeaderMap::new(), &Map::new());
        let decision = override_arm(&routing_one, &anonymous, ARM, |_| true).unwrap();
        assert_eq!(decision.run, None);
    }

    #[test]
    fn rules_that_do_not_compile_make_the_configuration_unusable() {
        let mut broken = config([0.0, 1.0], [1.0, 1.0], None);
        broken.settings.subject = Some("request.headers[".into());
        assert!(Routing::compile(&broken).is_err());
        let mut defaults = config([0.0, 1.0], [1.0, 1.0], None);
        defaults.settings.subject = None;
        assert!(routing(&defaults).default_subject);
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
