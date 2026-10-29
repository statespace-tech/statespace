//! `[[event]]` rules: CEL over one source. A request rule sees `request`
//! with `method`, `path`, `query`, `headers`, `cookies`, `status`, and
//! `ms`. An output rule sees `stdout` or `stderr` with `text` and `json`
//! (`{}` when the line is not a JSON object). A missing field, or any
//! other error, is no match.

use cel_interpreter::{Program, Value as Cel};
use serde_json::{Value, json};

use super::{
    assign::{compile, evaluate},
    config::{EventConfig, Source},
    orchestrator::Stream,
};

struct Rule {
    name: String,
    source: Source,
    program: Program,
}

#[derive(Default)]
pub struct Events {
    rules: Vec<Rule>,
}

impl Events {
    /// Compile every rule. A rule that does not compile is left out and
    /// returned as an error.
    pub fn compile(configs: &[EventConfig]) -> (Self, Vec<String>) {
        let mut rules = Vec::new();
        let mut errors = Vec::new();
        for config in configs {
            match compile(&config.rule) {
                Ok(program) => rules.push(Rule {
                    name: config.name.clone(),
                    source: config.source,
                    program,
                }),
                Err(error) => {
                    errors.push(format!("event {} does not compile: {error:#}", config.name))
                }
            }
        }
        (Self { rules }, errors)
    }

    /// The names of the request events that match a finished request.
    /// `request` is the request as assignment saw it; `status` and `ms`
    /// are added here.
    pub fn request(&self, request: &Value, status: u16, ms: f64) -> Vec<&str> {
        if !self.rules.iter().any(|rule| rule.source == Source::Request) {
            return Vec::new();
        }
        let mut value = request.clone();
        value["status"] = json!(status);
        value["ms"] = json!(ms);
        self.matching(Source::Request, "request", &value)
    }

    /// The names of the output events that match one line.
    pub fn output(&self, stream: Stream, text: &str) -> Vec<&str> {
        let source = match stream {
            Stream::Stdout => Source::Stdout,
            Stream::Stderr => Source::Stderr,
        };
        if !self.rules.iter().any(|rule| rule.source == source) {
            return Vec::new();
        }
        let parsed = match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(object)) => Value::Object(object),
            _ => json!({}),
        };
        let value = json!({ "text": text, "json": parsed });
        let name = match stream {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        };
        self.matching(source, name, &value)
    }

    fn matching(&self, source: Source, name: &str, value: &Value) -> Vec<&str> {
        self.rules
            .iter()
            .filter(|rule| rule.source == source)
            .filter(|rule| matches!(evaluate(&rule.program, name, value), Ok(Cel::Bool(true))))
            .map(|rule| rule.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Map;

    use super::*;
    use crate::proxy::assign::request_value;

    fn events(rules: &[(&str, &str, &str)]) -> Events {
        let configs: Vec<EventConfig> = rules
            .iter()
            .map(|(name, source, rule)| {
                serde_json::from_value(json!({"name": name, "source": source, "match": rule}))
                    .unwrap()
            })
            .collect();
        let (events, errors) = Events::compile(&configs);
        assert!(errors.is_empty(), "{errors:?}");
        events
    }

    fn post(path: &str) -> Value {
        let mut headers = http::HeaderMap::new();
        headers.insert("cf-ipcountry", "US".parse().unwrap());
        let mut cookies = Map::new();
        cookies.insert("plan".into(), json!("pro"));
        request_value("POST", path, Some("ref=ad"), &headers, &cookies)
    }

    #[test]
    fn matches_request_events_after_the_response() {
        let events = events(&[
            (
                "lead",
                "request",
                r#"request.method == "POST" && request.path == "/api/leads" && request.status < 300"#,
            ),
            ("slow", "request", "request.ms > 1000.0"),
            (
                "us",
                "request",
                r#"request.headers["cf-ipcountry"] == "US" && request.cookies.plan == "pro" && request.query.ref == "ad""#,
            ),
            (
                "missing",
                "request",
                r#"request.headers["x-missing"] == "1""#,
            ),
        ]);
        assert_eq!(
            events.request(&post("/api/leads"), 201, 12.0),
            ["lead", "us"]
        );
        assert_eq!(
            events.request(&post("/api/leads"), 500, 1500.0),
            ["slow", "us"]
        );
        assert_eq!(events.request(&post("/other"), 200, 1.0), ["us"]);
    }

    #[test]
    fn matches_output_lines_of_one_source() {
        let events = events(&[
            ("fatal", "stderr", r#"stderr.text.contains("FATAL")"#),
            ("signup", "stdout", r#"stdout.json.event == "signup""#),
            ("big", "stdout", "stdout.json.amount > 100"),
        ]);
        assert_eq!(
            events.output(Stream::Stderr, "FATAL: out of memory"),
            ["fatal"]
        );
        // The rule names stderr, so a stdout line never matches it.
        assert!(
            events
                .output(Stream::Stdout, "FATAL: out of memory")
                .is_empty()
        );
        assert_eq!(
            events.output(Stream::Stdout, r#"{"event": "signup", "amount": 120}"#),
            ["signup", "big"]
        );
        // A missing field is no match, not an error.
        assert!(events.output(Stream::Stdout, r#"{"amount": 5}"#).is_empty());
        // A line that is not a JSON object has `json` = {}.
        assert!(events.output(Stream::Stdout, "signup").is_empty());
        assert!(events.output(Stream::Stdout, "[1, 2]").is_empty());
        assert!(events.output(Stream::Stdout, "").is_empty());
    }

    #[test]
    fn rules_that_do_not_compile_are_reported() {
        let configs: Vec<EventConfig> = vec![
            serde_json::from_value(
                json!({"name": "bad", "source": "stdout", "match": "stdout.text.contains("}),
            )
            .unwrap(),
        ];
        let (events, errors) = Events::compile(&configs);
        assert_eq!(errors.len(), 1);
        assert!(events.output(Stream::Stdout, "x").is_empty());
    }
}
