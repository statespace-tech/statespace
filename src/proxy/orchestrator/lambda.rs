//! `ssp proxy lambda`: live is an AWS Lambda function that deploys from a
//! container image, at `--qualifier` or `$LATEST`. Each unit runs as its own
//! function, `<function>-ssp-<experiment>-<commit 12>` (cut with a short
//! hash when the name is too long), with live's configuration and another
//! image. The unit's function gets no trigger and no function URL, so it gets
//! no live traffic. It is tagged `statespace.com/app`,
//! `statespace.com/experiment`, and `statespace.com/commit`.
//!
//! The proxy invokes the functions itself, at their invocation URLs, which
//! are the units' addresses. It turns each request into the event of a
//! function URL (payload format 2.0), and the answer into a response, as a
//! function URL does. Live is invoked the same way, unless
//! `--live-url` names an HTTP address of live. Readiness and the reason a
//! unit is not ready come from the state of its function. Credentials come
//! from the AWS default chain.

use std::{
    collections::BTreeSet,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use http::{HeaderName, HeaderValue, Request, Response, StatusCode, header};
use serde_json::{Map, Value, json};

use super::{
    Address, App, Build, Live, Orchestrator, RESOURCE_ATTRIBUTES, Running, Unit, UnitKey,
    aws::{Aws, Endpoints, Service, encode},
    fargate::{APP_TAG, COMMIT_TAG, EXPERIMENT_TAG},
};
use crate::{cli::LambdaArgs, image};

/// Function names have at most 64 characters.
const NAME_LIMIT: usize = 64;
/// The end of an invocation URL, such as
/// `https://lambda.us-east-1.amazonaws.com/2015-03-31/functions/shop/invocations`.
const INVOCATIONS: &str = "/invocations";
/// How often the proxy lists the functions of the region again to find the
/// units. In between, it reads only the units it knows.
const LIST_EVERY: Duration = Duration::from_secs(15);
/// Headers of a function's answer that no longer describe the body.
const FRAMING: [HeaderName; 2] = [header::CONTENT_LENGTH, header::TRANSFER_ENCODING];

pub struct Lambda {
    aws: Aws,
    function: String,
    qualifier: Option<String>,
    live_address: Address,
    /// The URL of the functions of the region, which each invocation URL
    /// starts with.
    functions_url: String,
    /// The names of the units' functions, and when the proxy last listed
    /// them.
    known: Mutex<(Option<Instant>, BTreeSet<String>)>,
}

impl Lambda {
    pub async fn new(arguments: LambdaArgs) -> anyhow::Result<Self> {
        let aws = Aws::connect(
            arguments.region.clone(),
            None,
            Endpoints {
                lambda: arguments.lambda_api.clone(),
                ..Endpoints::default()
            },
        )
        .await?;
        let functions_url = format!("{}/2015-03-31/functions/", aws.endpoint(Service::Lambda));
        let mut lambda = Self {
            aws,
            function: arguments.function,
            qualifier: arguments.qualifier,
            live_address: String::new(),
            functions_url,
            known: Mutex::default(),
        };
        lambda.live_address = match arguments.live_url {
            Some(url) => url.trim_end_matches('/').to_owned(),
            None => lambda.address(&lambda.function),
        };
        lambda.live_function().await?;
        Ok(lambda)
    }

    /// A function with its code and tags, as `GetFunction` gives them,
    /// unless it does not exist.
    async fn function(&self, name: &str, qualifier: Option<&str>) -> anyhow::Result<Option<Value>> {
        let mut path = format!("/2015-03-31/functions/{}", encode(name));
        if let Some(qualifier) = qualifier {
            path.push_str(&format!("?Qualifier={}", encode(qualifier)));
        }
        self.aws.rest(Service::Lambda, "GET", &path, None).await
    }

    async fn live_function(&self) -> anyhow::Result<Value> {
        let live = self
            .function(&self.function, self.qualifier.as_deref())
            .await?
            .with_context(|| {
                let qualifier = self
                    .qualifier
                    .as_ref()
                    .map(|qualifier| format!(":{qualifier}"))
                    .unwrap_or_default();
                format!(
                    "function {}{qualifier} does not exist in {}",
                    self.function, self.aws.region
                )
            })?;
        if live["Configuration"]["PackageType"] != "Image" {
            bail!(
                "function {} deploys from a .zip file; ssp proxy lambda runs functions that deploy from a container image",
                self.function
            );
        }
        Ok(live)
    }

    fn unit_name(&self, key: &UnitKey) -> String {
        unit_name(&self.function, key)
    }

    /// The invocation URL of a function.
    fn address(&self, function: &str) -> Address {
        format!("{}{}{INVOCATIONS}", self.functions_url, encode(function))
    }

    /// The names of the units' functions: from a new listing every
    /// [`LIST_EVERY`], else the ones the proxy knows.
    async fn names(&self) -> anyhow::Result<BTreeSet<String>> {
        {
            let known = self.known.lock().expect("names are never poisoned");
            if known.0.is_some_and(|listed| listed.elapsed() < LIST_EVERY) {
                return Ok(known.1.clone());
            }
        }
        // A name cut with a hash keeps at least this much of the prefix.
        let prefix: String = format!("{}-ssp-", self.function)
            .chars()
            .take(NAME_LIMIT - 7)
            .collect();
        let mut names = BTreeSet::new();
        let mut marker: Option<String> = None;
        loop {
            let mut path = "/2015-03-31/functions/?MaxItems=50".to_owned();
            if let Some(marker) = &marker {
                path.push_str(&format!("&Marker={}", encode(marker)));
            }
            let answer = self
                .aws
                .rest(Service::Lambda, "GET", &path, None)
                .await?
                .unwrap_or(Value::Null);
            for function in answer["Functions"].as_array().into_iter().flatten() {
                if let Some(name) = function["FunctionName"].as_str()
                    && name.starts_with(&prefix)
                {
                    names.insert(name.to_owned());
                }
            }
            marker = answer["NextMarker"].as_str().map(str::to_owned);
            if marker.is_none() {
                break;
            }
        }
        *self.known.lock().expect("names are never poisoned") =
            (Some(Instant::now()), names.clone());
        Ok(names)
    }

    fn remember(&self, name: &str, known: bool) {
        let mut names = self.known.lock().expect("names are never poisoned");
        if known {
            names.1.insert(name.to_owned());
        } else {
            names.1.remove(name);
        }
    }
}

impl Orchestrator for Lambda {
    fn name(&self) -> &'static str {
        "lambda"
    }

    /// The digest of the image that live's version runs.
    async fn live(&self, _app: &App) -> anyhow::Result<Live> {
        let live = self.live_function().await?;
        let code = &live["Code"];
        let image = code["ResolvedImageUri"]
            .as_str()
            .or_else(|| code["ImageUri"].as_str())
            .unwrap_or_default();
        if !image::is_digest(image) {
            bail!(
                "function {} reports no image digest, only {image:?}",
                self.function
            );
        }
        Ok(Live::Image(image.to_owned()))
    }

    fn live_address(&self) -> Address {
        self.live_address.clone()
    }

    async fn start(&self, app: &App, unit: &Unit) -> anyhow::Result<Address> {
        let name = self.unit_name(&unit.key());
        if let Some(current) = self.function(&name, None).await? {
            if tag(&current["Tags"], APP_TAG) != Some(app.name.as_str()) {
                bail!("function {name} exists and is not an arm of {}", app.name);
            }
            self.remember(&name, true);
            if current["Code"]["ImageUri"] == unit.image() {
                return Ok(self.address(&name));
            }
            self.aws
                .rest(
                    Service::Lambda,
                    "PUT",
                    &format!("/2015-03-31/functions/{}/code", encode(&name)),
                    Some(&json!({"ImageUri": unit.image()})),
                )
                .await?;
            eprintln!("[ssp] updated function {name} for {}", unit.key());
            return Ok(self.address(&name));
        }
        let mut live = self.live_function().await?;
        // Lambda gives the tags only for the function without a qualifier.
        if self.qualifier.is_some()
            && let Some(function) = self.function(&self.function, None).await?
        {
            live["Tags"] = function["Tags"].clone();
        }
        let creation = unit_function(&live, &app.name, &name, unit)?;
        self.aws
            .rest(
                Service::Lambda,
                "POST",
                "/2015-03-31/functions",
                Some(&creation),
            )
            .await?;
        self.remember(&name, true);
        eprintln!("[ssp] created function {name} for {}", unit.key());
        Ok(self.address(&name))
    }

    async fn stop(&self, _app: &App, unit: &Unit) -> anyhow::Result<()> {
        let name = self.unit_name(&unit.key());
        let deleted = self
            .aws
            .rest(
                Service::Lambda,
                "DELETE",
                &format!("/2015-03-31/functions/{}", encode(&name)),
                None,
            )
            .await?;
        self.remember(&name, false);
        if deleted.is_some() {
            eprintln!("[ssp] deleted function {name}");
        }
        Ok(())
    }

    async fn running(&self, app: &App) -> anyhow::Result<Vec<Running>> {
        let mut running = Vec::new();
        for name in self.names().await? {
            let Some(function) = self.function(&name, None).await? else {
                self.remember(&name, false);
                continue;
            };
            let tags = &function["Tags"];
            if tag(tags, APP_TAG) != Some(app.name.as_str()) {
                continue;
            }
            let (Some(experiment), Some(commit)) =
                (tag(tags, EXPERIMENT_TAG), tag(tags, COMMIT_TAG))
            else {
                continue;
            };
            let unit = Unit {
                experiment: experiment.to_owned(),
                commit: commit.to_owned(),
                build: Build::Image(
                    function["Code"]["ImageUri"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                ),
                share: 0.0,
            };
            let reason = not_ready(&function["Configuration"]);
            running.push(Running {
                unit,
                address: self.address(&name),
                ready: reason.is_none(),
                reason,
            });
        }
        Ok(running)
    }

    fn invokes(&self, address: &str) -> bool {
        address.starts_with(&self.functions_url) && address.ends_with(INVOCATIONS)
    }

    async fn invoke(
        &self,
        address: &str,
        request: Request<Bytes>,
    ) -> anyhow::Result<Response<Bytes>> {
        let name = address
            .strip_prefix(&self.functions_url)
            .and_then(|rest| rest.strip_suffix(INVOCATIONS))
            .map(decode)
            .context("not the invocation URL of a function")?;
        let qualifier = if name == self.function {
            self.qualifier.as_deref()
        } else {
            None
        };
        // The proxy put the request's path after the address.
        let prefix = address.parse::<http::Uri>()?.path().to_owned();
        let path = request
            .uri()
            .path()
            .strip_prefix(&prefix)
            .filter(|path| path.starts_with('/'))
            .unwrap_or("/")
            .to_owned();
        let id = format!("{:032x}", rand::random::<u128>());
        let event = http_event(&request, &path, &name, Utc::now(), &id);
        let reply = self
            .aws
            .invoke(address, qualifier, serde_json::to_vec(&event)?)
            .await?;
        if !reply.status.is_success() {
            let text = reply.text();
            let error: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let message = error["Message"]
                .as_str()
                .or_else(|| error["message"].as_str())
                .map_or(text.clone(), str::to_owned);
            bail!("Lambda answered {} for {name}: {message}", reply.status);
        }
        if reply.header("x-amz-function-error").is_some() {
            let error: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
            bail!(
                "function {name} failed: {} {}",
                error["errorType"].as_str().unwrap_or_default(),
                error["errorMessage"].as_str().unwrap_or_default()
            );
        }
        http_response(&reply.body)
    }
}

/// `<function>-ssp-<experiment>-<commit 12>`, cut with a short hash past 64
/// characters.
pub fn unit_name(function: &str, key: &UnitKey) -> String {
    key.name(&format!("{function}-ssp-"), NAME_LIMIT)
}

/// The value of a tag in a Lambda tag map.
fn tag<'a>(tags: &'a Value, key: &str) -> Option<&'a str> {
    tags.get(key)?.as_str()
}

/// Why a unit's function takes no requests yet, or `None` when it does.
/// An inactive function takes them: the first invocation wakes it.
fn not_ready(configuration: &Value) -> Option<String> {
    let text = |field: &str| configuration[field].as_str().unwrap_or_default();
    match (text("State"), text("LastUpdateStatus")) {
        ("Failed", _) => Some(format!("the function failed: {}", text("StateReason"))),
        ("Pending", _) => Some("the function is starting".into()),
        (_, "InProgress") => Some("the function is updating".into()),
        (_, "Failed") => Some(format!(
            "the update of the function failed: {}",
            text("LastUpdateStatusReason")
        )),
        _ => None,
    }
}

/// The creation of a unit's function: live's configuration, with the
/// unit's name, image, and tags, and the variables that label the arm.
/// Logs go to the unit's own log group, unless live names another one. No
/// `STATESPACE_*` variable reaches a unit.
pub fn unit_function(live: &Value, app: &str, name: &str, unit: &Unit) -> anyhow::Result<Value> {
    let configuration = &live["Configuration"];
    let role = configuration["Role"]
        .as_str()
        .context("the function has no role")?;
    let live_name = configuration["FunctionName"].as_str().unwrap_or_default();
    let mut function = json!({
        "FunctionName": name,
        "PackageType": "Image",
        "Code": {"ImageUri": unit.image()},
        "Role": role,
        "Description": format!("Statespace arm {} of {live_name}", unit.key()),
        "Publish": false,
        "Environment": {"Variables": unit_environment(
            &configuration["Environment"]["Variables"], &unit.key())},
        "Tags": unit_tags(&live["Tags"], app, unit),
    });
    for field in [
        "MemorySize",
        "Timeout",
        "Architectures",
        "EphemeralStorage",
        "KMSKeyArn",
        "FileSystemConfigs",
    ] {
        let value = &configuration[field];
        let empty = value.is_null() || value.as_array().is_some_and(Vec::is_empty);
        if !empty {
            function[field] = value.clone();
        }
    }
    if let Some(mode) = configuration["TracingConfig"]["Mode"].as_str() {
        function["TracingConfig"] = json!({"Mode": mode});
    }
    let vpc = &configuration["VpcConfig"];
    if vpc["SubnetIds"]
        .as_array()
        .is_some_and(|ids| !ids.is_empty())
    {
        let mut copy = json!({
            "SubnetIds": vpc["SubnetIds"],
            "SecurityGroupIds": vpc["SecurityGroupIds"],
        });
        if let Some(dual) = vpc["Ipv6AllowedForDualStack"].as_bool() {
            copy["Ipv6AllowedForDualStack"] = json!(dual);
        }
        function["VpcConfig"] = copy;
    }
    let image = &configuration["ImageConfigResponse"]["ImageConfig"];
    if image.as_object().is_some_and(|fields| !fields.is_empty()) {
        function["ImageConfig"] = image.clone();
    }
    if let Some(logging) = configuration["LoggingConfig"].as_object() {
        let mut logging = logging.clone();
        if logging.get("LogGroup").and_then(Value::as_str)
            == Some(&format!("/aws/lambda/{live_name}"))
        {
            logging.remove("LogGroup");
        }
        function["LoggingConfig"] = json!(logging);
    }
    Ok(function)
}

/// Live's variables without `STATESPACE_*`, with the variables that label
/// the arm. Live's `OTEL_RESOURCE_ATTRIBUTES` keeps its other attributes.
fn unit_environment(live: &Value, key: &UnitKey) -> Value {
    let mut variables: Map<String, Value> = live
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, _)| !name.starts_with("STATESPACE_"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let live_attributes = live[RESOURCE_ATTRIBUTES].as_str();
    for (name, value) in key.environment(live_attributes) {
        variables.insert(name.to_owned(), json!(value));
    }
    json!(variables)
}

/// The tags of a unit: those of live, without the ones AWS reserves, and the
/// unit's own.
fn unit_tags(live: &Value, app: &str, unit: &Unit) -> Value {
    let ours = [APP_TAG, EXPERIMENT_TAG, COMMIT_TAG];
    let mut tags: Map<String, Value> = live
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !key.starts_with("aws:") && !ours.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in ours.iter().zip([app, &unit.experiment, &unit.commit]) {
        tags.insert((*key).to_owned(), json!(value));
    }
    json!(tags)
}

/// The event that a function URL gives a function for `request` (payload
/// format 2.0). Cookies go in `cookies`, not in the headers. A body that is
/// not UTF-8 is sent in base64.
pub fn http_event(
    request: &Request<Bytes>,
    path: &str,
    function: &str,
    now: DateTime<Utc>,
    id: &str,
) -> Value {
    let mut headers = Map::new();
    let mut cookies = Vec::new();
    for name in request.headers().keys() {
        let values: Vec<String> = request
            .headers()
            .get_all(name)
            .iter()
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
            .collect();
        if name == header::COOKIE {
            cookies.extend(
                values
                    .iter()
                    .flat_map(|value| value.split(';'))
                    .map(str::trim)
                    .filter(|cookie| !cookie.is_empty())
                    .map(str::to_owned),
            );
        } else {
            headers.insert(name.as_str().to_owned(), json!(values.join(",")));
        }
    }
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let host = text("host");
    let source = text("x-forwarded-for");
    let source = source.split(',').next().unwrap_or_default().trim();
    let query = request.uri().query().unwrap_or_default();
    let mut event = json!({
        "version": "2.0",
        "routeKey": "$default",
        "rawPath": path,
        "rawQueryString": query,
        "headers": headers,
        "requestContext": {
            "accountId": "anonymous",
            "apiId": function,
            "domainName": host,
            "domainPrefix": host.split('.').next().unwrap_or_default(),
            "http": {
                "method": request.method().as_str(),
                "path": path,
                "protocol": "HTTP/1.1",
                "sourceIp": source,
                "userAgent": text("user-agent"),
            },
            "requestId": id,
            "routeKey": "$default",
            "stage": "$default",
            "time": now.format("%d/%b/%Y:%H:%M:%S +0000").to_string(),
            "timeEpoch": now.timestamp_millis(),
        },
        "isBase64Encoded": false,
    });
    if !cookies.is_empty() {
        event["cookies"] = json!(cookies);
    }
    let parameters = query_parameters(query);
    if !parameters.is_empty() {
        event["queryStringParameters"] = json!(parameters);
    }
    let body = request.body();
    if !body.is_empty() {
        match std::str::from_utf8(body) {
            Ok(text) => event["body"] = json!(text),
            Err(_) => {
                event["body"] = json!(base64::engine::general_purpose::STANDARD.encode(body));
                event["isBase64Encoded"] = json!(true);
            }
        }
    }
    event
}

/// The parameters of a query string, decoded. A repeated parameter has its
/// values joined with commas, as a function URL gives them.
fn query_parameters(query: &str) -> Map<String, Value> {
    let mut parameters = Map::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let (name, value) = (decode(name), decode(value));
        match parameters.get_mut(&name) {
            Some(Value::String(joined)) => {
                joined.push(',');
                joined.push_str(&value);
            }
            _ => {
                parameters.insert(name, json!(value));
            }
        }
    }
    parameters
}

/// A percent-encoded query part, with `+` as a space.
fn decode(part: &str) -> String {
    let bytes = part.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => match bytes
                .get(index + 1..index + 3)
                .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
                .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok())
            {
                Some(byte) => {
                    decoded.push(byte);
                    index += 2;
                }
                None => decoded.push(b'%'),
            },
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The response for a function's answer, as a function URL makes it: an
/// object with `statusCode` gives the status, headers, cookies, and body;
/// any other JSON is the body of a 200 with `content-type:
/// application/json`.
pub fn http_response(payload: &[u8]) -> anyhow::Result<Response<Bytes>> {
    let answer: Value =
        serde_json::from_slice(payload).context("the function answered with no JSON")?;
    let Some(status) = answer.get("statusCode") else {
        let mut response = Response::new(Bytes::copy_from_slice(payload));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        return Ok(response);
    };
    let status = status
        .as_u64()
        .and_then(|status| u16::try_from(status).ok())
        .and_then(|status| StatusCode::from_u16(status).ok())
        .with_context(|| format!("the function answered the status {status}"))?;
    let body = match answer["body"].as_str() {
        None => Bytes::new(),
        Some(body) if answer["isBase64Encoded"] == true => Bytes::from(
            base64::engine::general_purpose::STANDARD
                .decode(body)
                .context("the function answered a body that is not base64")?,
        ),
        Some(body) => Bytes::copy_from_slice(body.as_bytes()),
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    let value = |value: &Value| match value {
        Value::String(text) => HeaderValue::from_str(text).ok(),
        other => HeaderValue::from_str(&other.to_string()).ok(),
    };
    for (name, values) in answer["multiValueHeaders"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        for item in values.as_array().into_iter().flatten() {
            if let Some(item) = value(item) {
                headers.append(name.clone(), item);
            }
        }
    }
    for (name, item) in answer["headers"].as_object().into_iter().flatten() {
        if let (Ok(name), Some(item)) = (HeaderName::from_bytes(name.as_bytes()), value(item)) {
            headers.insert(name, item);
        }
    }
    for cookie in answer["cookies"].as_array().into_iter().flatten() {
        if let Some(cookie) = value(cookie) {
            headers.append(header::SET_COOKIE, cookie);
        }
    }
    for name in FRAMING {
        headers.remove(name);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Value {
        json!({
            "Configuration": {
                "FunctionName": "shop",
                "PackageType": "Image",
                "Role": "arn:aws:iam::123456789012:role/shop",
                "MemorySize": 1024,
                "Timeout": 30,
                "Architectures": ["arm64"],
                "EphemeralStorage": {"Size": 512},
                "Environment": {"Variables": {
                    "MODE": "prod",
                    "STATESPACE_TOKEN": "never",
                    "OTEL_RESOURCE_ATTRIBUTES": "service.name=shop,service.version=old",
                }},
                "TracingConfig": {"Mode": "Active"},
                "VpcConfig": {"SubnetIds": ["subnet-1"], "SecurityGroupIds": ["sg-1"],
                              "VpcId": "vpc-1", "Ipv6AllowedForDualStack": false},
                "ImageConfigResponse": {"ImageConfig": {"Command": ["app.handler"]}},
                "LoggingConfig": {"LogFormat": "JSON", "LogGroup": "/aws/lambda/shop"},
                "FileSystemConfigs": [],
                "Layers": [],
                "State": "Active",
                "LastUpdateStatus": "Successful",
                "CodeSha256": "abc",
                "FunctionArn": "arn:aws:lambda:us-east-1:123456789012:function:shop",
            },
            "Code": {
                "ImageUri": "123456789012.dkr.ecr.us-east-1.amazonaws.com/shop:main",
                "ResolvedImageUri": format!(
                    "123456789012.dkr.ecr.us-east-1.amazonaws.com/shop@sha256:{}", "1".repeat(64)),
            },
            "Tags": {"team": "web", "aws:cloudformation:stack-name": "x"},
        })
    }

    fn arm() -> Unit {
        Unit {
            experiment: "checkout".into(),
            commit: "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0".into(),
            build: Build::Image(format!(
                "123456789012.dkr.ecr.us-east-1.amazonaws.com/shop@sha256:{}",
                "a".repeat(64)
            )),
            share: 0.05,
        }
    }

    #[test]
    fn names_units_after_the_function() {
        assert_eq!(
            unit_name("shop", &arm().key()),
            "shop-ssp-checkout-a1b2c3d4e5f6"
        );
        let long = unit_name(&"s".repeat(60), &arm().key());
        assert_eq!(long.len(), 64);
        assert!(long.starts_with(&"s".repeat(57)), "{long}");
    }

    #[test]
    fn functions_copy_live_and_change_only_the_image() {
        let name = "shop-ssp-checkout-a1b2c3d4e5f6";
        let function = unit_function(&live(), "shop-prod", name, &arm()).unwrap();
        assert_eq!(function["FunctionName"], name);
        assert_eq!(function["PackageType"], "Image");
        assert_eq!(function["Code"], json!({"ImageUri": arm().image()}));
        assert_eq!(function["Role"], "arn:aws:iam::123456789012:role/shop");
        assert_eq!(function["Publish"], false);
        for field in ["MemorySize", "Timeout", "Architectures", "EphemeralStorage"] {
            assert_eq!(function[field], live()["Configuration"][field], "{field}");
        }
        assert_eq!(function["TracingConfig"], json!({"Mode": "Active"}));
        assert_eq!(
            function["VpcConfig"],
            json!({"SubnetIds": ["subnet-1"], "SecurityGroupIds": ["sg-1"],
                   "Ipv6AllowedForDualStack": false})
        );
        assert_eq!(function["ImageConfig"], json!({"Command": ["app.handler"]}));
        // The unit logs to its own group.
        assert_eq!(function["LoggingConfig"], json!({"LogFormat": "JSON"}));
        for field in [
            "FileSystemConfigs",
            "Layers",
            "State",
            "CodeSha256",
            "FunctionArn",
        ] {
            assert!(function.get(field).is_none(), "{field}");
        }
        assert_eq!(
            function["Environment"]["Variables"],
            json!({
                "MODE": "prod",
                "OTEL_RESOURCE_ATTRIBUTES": format!(
                    "service.name=shop,statespace.experiment=checkout,statespace.arm={0},\
                     service.version={0}", arm().commit),
                "SSP_EXPERIMENT": "checkout",
                "SSP_ARM": arm().commit,
            })
        );
        assert_eq!(
            function["Tags"],
            json!({
                "team": "web",
                APP_TAG: "shop-prod",
                EXPERIMENT_TAG: "checkout",
                COMMIT_TAG: arm().commit,
            })
        );
        assert!(!function.to_string().contains("STATESPACE_"));
        // A log group that live names on purpose stays.
        let mut shared = live();
        shared["Configuration"]["LoggingConfig"]["LogGroup"] = json!("/acme/shop");
        let function = unit_function(&shared, "shop-prod", name, &arm()).unwrap();
        assert_eq!(function["LoggingConfig"]["LogGroup"], "/acme/shop");
        // Without a VPC, the unit gets none.
        let mut public = live();
        public["Configuration"]["VpcConfig"] = json!({"SubnetIds": [], "SecurityGroupIds": []});
        let function = unit_function(&public, "shop-prod", name, &arm()).unwrap();
        assert!(function.get("VpcConfig").is_none());
    }

    #[test]
    fn says_why_a_function_is_not_ready() {
        let state = |state: &str, update: &str| {
            not_ready(&json!({"State": state, "LastUpdateStatus": update,
                "StateReason": "Source image is not valid", "LastUpdateStatusReason": "pull failed"}))
        };
        assert_eq!(state("Active", "Successful"), None);
        assert_eq!(state("Inactive", "Successful"), None);
        assert_eq!(
            state("Pending", "InProgress").as_deref(),
            Some("the function is starting")
        );
        assert_eq!(
            state("Active", "InProgress").as_deref(),
            Some("the function is updating")
        );
        assert_eq!(
            state("Failed", "Failed").as_deref(),
            Some("the function failed: Source image is not valid")
        );
        assert_eq!(
            state("Active", "Failed").as_deref(),
            Some("the update of the function failed: pull failed")
        );
    }

    #[test]
    fn requests_become_function_url_events() {
        let request = Request::builder()
            .method("POST")
            .uri("https://lambda.us-east-1.amazonaws.com/2015-03-31/functions/shop/invocations/checkout/pay?step=1&tag=a+b&tag=c%2Cd")
            .header("host", "shop.example.com")
            .header("cookie", "ssp_sid=s1; theme=dark")
            .header("cookie", "lang=en")
            .header("x-forwarded-for", "203.0.113.7, 10.0.0.2")
            .header("user-agent", "curl/8")
            .header("accept", "text/html")
            .header("accept", "application/json")
            .header("ssp-arm", "a1b2c3d")
            .body(Bytes::from_static(b"{\"total\": 3}"))
            .unwrap();
        let now = DateTime::from_timestamp(1_762_400_000, 0).unwrap();
        let event = http_event(&request, "/checkout/pay", "shop", now, "id-1");
        assert_eq!(event["version"], "2.0");
        assert_eq!(event["rawPath"], "/checkout/pay");
        assert_eq!(event["rawQueryString"], "step=1&tag=a+b&tag=c%2Cd");
        assert_eq!(
            event["queryStringParameters"],
            json!({"step": "1", "tag": "a b,c,d"})
        );
        assert_eq!(
            event["cookies"],
            json!(["ssp_sid=s1", "theme=dark", "lang=en"])
        );
        assert!(event["headers"].get("cookie").is_none());
        assert_eq!(event["headers"]["accept"], "text/html,application/json");
        assert_eq!(event["headers"]["ssp-arm"], "a1b2c3d");
        assert_eq!(event["body"], "{\"total\": 3}");
        assert_eq!(event["isBase64Encoded"], false);
        let context = &event["requestContext"];
        assert_eq!(context["http"]["method"], "POST");
        assert_eq!(context["http"]["sourceIp"], "203.0.113.7");
        assert_eq!(context["http"]["userAgent"], "curl/8");
        assert_eq!(context["domainName"], "shop.example.com");
        assert_eq!(context["domainPrefix"], "shop");
        assert_eq!(context["requestId"], "id-1");
        assert_eq!(context["time"], "06/Nov/2025:03:33:20 +0000");
        assert_eq!(context["timeEpoch"], 1_762_400_000_000_i64);

        let binary = Request::builder()
            .uri("https://lambda.us-east-1.amazonaws.com/")
            .body(Bytes::from_static(&[0xff, 0x00]))
            .unwrap();
        let event = http_event(&binary, "/", "shop", now, "id-2");
        assert_eq!(event["body"], "/wA=");
        assert_eq!(event["isBase64Encoded"], true);
        assert!(event.get("cookies").is_none());
        assert!(event.get("queryStringParameters").is_none());

        let empty = Request::builder()
            .uri("https://lambda.us-east-1.amazonaws.com/")
            .body(Bytes::new())
            .unwrap();
        assert!(
            http_event(&empty, "/", "shop", now, "id-3")
                .get("body")
                .is_none()
        );
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz%41"), "%zzA");
    }

    #[test]
    fn answers_become_responses() {
        let answer = json!({
            "statusCode": 201,
            "headers": {"content-type": "text/plain", "x-app-event": "visit",
                        "content-length": "999"},
            "multiValueHeaders": {"vary": ["accept", "cookie"]},
            "cookies": ["a=1; Path=/", "b=2"],
            "body": "aGk=",
            "isBase64Encoded": true,
        });
        let response = http_response(answer.to_string().as_bytes()).unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(response.body(), "hi");
        assert_eq!(response.headers()["x-app-event"], "visit");
        assert!(response.headers().get("content-length").is_none());
        let vary: Vec<_> = response.headers().get_all("vary").iter().collect();
        assert_eq!(vary, ["accept", "cookie"]);
        let cookies: Vec<_> = response.headers().get_all("set-cookie").iter().collect();
        assert_eq!(cookies, ["a=1; Path=/", "b=2"]);

        // Any other JSON is the body of a 200.
        let response = http_response(b"{\"total\": 3}").unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.body(), "{\"total\": 3}");
        let response = http_response(b"\"hello\"").unwrap();
        assert_eq!(response.body(), "\"hello\"");

        let empty = http_response(br#"{"statusCode": 204}"#).unwrap();
        assert_eq!(empty.status(), 204);
        assert!(empty.body().is_empty());
        assert!(http_response(br#"{"statusCode": 99}"#).is_err());
        assert!(http_response(b"not json").is_err());
    }
}
