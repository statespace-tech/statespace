//! `[[event]]` rules: CEL over one request and its response. A rule sees
//! `request` with `method`, `host`, `path`, `query`, `headers`, and
//! `cookies`, and `response` with `status`, `headers`, and `ms` (the time
//! to the response headers). The proxy evaluates the rules once per arm
//! request, when the response headers arrive, and never reads a body. A
//! missing field, or any other error, is no match.

use std::collections::BTreeMap;

use cel_interpreter::{Program, Value as Cel};
use serde_json::{Value, json};

use super::{
    assign::{compile, evaluate},
    config::EventConfig,
};

/// The built-in events of every experiment. An `[[event]]` cannot have
/// these names: Statespace counts them from the response status.
pub const BUILT_IN_EVENTS: [&str; 2] = ["requests_4xx", "requests_5xx"];

/// Why an event name is not valid: it must be snake_case
/// (`^[a-z][a-z0-9_]*$`, at most 64 characters), and not a built-in event.
pub fn check_name(name: &str) -> Result<(), String> {
    let bytes = name.as_bytes();
    let snake = (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_');
    if !snake {
        return Err(format!(
            "event name {name:?} must be lowercase snake_case, such as add_to_cart"
        ));
    }
    if BUILT_IN_EVENTS.contains(&name) {
        return Err(format!(
            "event name {name:?} is reserved: Statespace shows requests_4xx and requests_5xx \
             for every experiment"
        ));
    }
    Ok(())
}

struct Rule {
    name: String,
    program: Program,
}

#[derive(Default)]
pub struct Events {
    rules: Vec<Rule>,
}

impl Events {
    /// Compile every rule. A rule with a name that is not valid, or that
    /// does not compile, is left out and returned as an error.
    pub fn compile(configs: &[EventConfig]) -> (Self, Vec<String>) {
        let mut rules = Vec::new();
        let mut errors = Vec::new();
        for config in configs {
            if let Err(error) = check_name(&config.name) {
                errors.push(error);
                continue;
            }
            match compile(&config.rule) {
                Ok(program) => rules.push(Rule {
                    name: config.name.clone(),
                    program,
                }),
                Err(error) => {
                    errors.push(format!("event {} does not compile: {error:#}", config.name))
                }
            }
        }
        (Self { rules }, errors)
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The names of the events that match a request and its response.
    /// `request` is the request as assignment saw it.
    pub fn matching(&self, request: &Value, response: &Value) -> Vec<&str> {
        self.rules
            .iter()
            .filter(|rule| {
                matches!(
                    evaluate(
                        &rule.program,
                        &[("request", request), ("response", response)]
                    ),
                    Ok(Cel::Bool(true))
                )
            })
            .map(|rule| rule.name.as_str())
            .collect()
    }
}

/// The response as event rules see it: `status`, `headers` (lowercase
/// names; a repeated header is joined with `, `), and `ms`.
pub fn response_value(status: u16, headers: &http::HeaderMap, ms: f64) -> Value {
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let Ok(value) = value.to_str() else {
            continue;
        };
        values
            .entry(name.as_str().to_owned())
            .and_modify(|joined| {
                joined.push_str(", ");
                joined.push_str(value);
            })
            .or_insert_with(|| value.to_owned());
    }
    json!({ "status": status, "headers": values, "ms": ms })
}

#[cfg(test)]
mod tests {
    use serde_json::Map;

    use super::*;
    use crate::proxy::assign::request_value;

    fn events(rules: &[(&str, &str)]) -> Events {
        let configs: Vec<EventConfig> = rules
            .iter()
            .map(|(name, rule)| {
                serde_json::from_value(json!({"name": name, "match": rule})).unwrap()
            })
            .collect();
        let (events, errors) = Events::compile(&configs);
        assert!(errors.is_empty(), "{errors:?}");
        events
    }

    fn post(path: &str) -> Value {
        let mut headers = http::HeaderMap::new();
        headers.insert("cf-ipcountry", "US".parse().unwrap());
        headers.insert("host", "shop.acme.com".parse().unwrap());
        let mut cookies = Map::new();
        cookies.insert("plan".into(), json!("pro"));
        request_value("POST", None, path, Some("ref=ad"), &headers, &cookies)
    }

    fn response(status: u16, ms: f64, event: Option<&str>) -> Value {
        let mut headers = http::HeaderMap::new();
        if let Some(event) = event {
            headers.insert("x-statespace-event", event.parse().unwrap());
        }
        response_value(status, &headers, ms)
    }

    #[test]
    fn matches_a_request_and_its_response() {
        let events = events(&[
            (
                "signup",
                r#"request.method == "POST" && request.path == "/signup" && response.status == 201"#,
            ),
            ("slow", "response.ms > 1000.0"),
            (
                "us",
                r#"request.headers["cf-ipcountry"] == "US" && request.cookies.plan == "pro" && request.query.ref == "ad" && request.host == "shop.acme.com""#,
            ),
            (
                "account",
                r#"response.headers["x-statespace-event"] == "account_created""#,
            ),
            ("missing", r#"request.headers["x-missing"] == "1""#),
            // Removed in 0.5.0: these fields are missing, so no match.
            ("old", "request.status == 201 || request.ms > 0.0"),
        ]);
        assert_eq!(
            events.matching(&post("/signup"), &response(201, 12.0, None)),
            ["signup", "us"]
        );
        assert_eq!(
            events.matching(&post("/signup"), &response(500, 1500.0, None)),
            ["slow", "us"]
        );
        assert_eq!(
            events.matching(
                &post("/other"),
                &response(200, 1.0, Some("account_created"))
            ),
            ["us", "account"]
        );
    }

    #[test]
    fn rules_that_do_not_compile_are_reported() {
        let configs: Vec<EventConfig> = vec![
            serde_json::from_value(json!({"name": "bad", "match": "request.path.contains("}))
                .unwrap(),
        ];
        let (events, errors) = Events::compile(&configs);
        assert_eq!(errors.len(), 1);
        assert!(events.is_empty());
    }
    #[test]
    fn names_are_snake_case_and_not_built_in() {
        let config = |name: &str| -> EventConfig {
            serde_json::from_value(json!({"name": name, "match": "response.status == 201"}))
                .unwrap()
        };
        let configs = vec![
            config("signup"),
            config("requests_5xx"),
            config("Sign-Up"),
            config("add_to_cart2"),
        ];
        let (events, errors) = Events::compile(&configs);
        assert_eq!(events.rules.len(), 2);
        assert_eq!(
            errors,
            [
                "event name \"requests_5xx\" is reserved: Statespace shows requests_4xx and \
                 requests_5xx for every experiment",
                "event name \"Sign-Up\" must be lowercase snake_case, such as add_to_cart",
            ]
        );
        assert!(check_name("_x").is_err() && check_name(&"a".repeat(65)).is_err());
        assert!(check_name("requests_4xx").is_err() && check_name("requests").is_ok());
    }
}
