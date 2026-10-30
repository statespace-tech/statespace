//! A small client for the AWS APIs that `ssp proxy fargate` calls. It gets
//! credentials from the AWS default chain (environment, profile, web
//! identity, ECS task role, EC2 instance role), signs each request with
//! SigV4, and sends it with the proxy's own HTTP client.

use std::{
    sync::Mutex,
    time::{Duration, SystemTime},
};

use anyhow::{Context, bail};
use aws_config::{BehaviorVersion, Region, meta::region::RegionProviderChain};
use aws_credential_types::{
    Credentials,
    provider::{ProvideCredentials, SharedCredentialsProvider},
};
use aws_sigv4::{
    http_request::{SignableBody, SignableRequest, SigningSettings, sign},
    sign::v4,
};
use aws_smithy_http_client::tls;
use serde_json::Value;

/// The AWS services the proxy calls, with the endpoint of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Ecs,
    Ec2,
    Logs,
}

impl Service {
    /// The name in SigV4 signatures and default endpoints.
    fn signing_name(self) -> &'static str {
        match self {
            Self::Ecs => "ecs",
            Self::Ec2 => "ec2",
            Self::Logs => "logs",
        }
    }

    /// The prefix of the `X-Amz-Target` header of JSON APIs.
    fn target(self) -> &'static str {
        match self {
            Self::Ecs => "AmazonEC2ContainerServiceV20141113",
            Self::Logs => "Logs_20140328",
            Self::Ec2 => "",
        }
    }
}

/// Endpoints that replace the default ones. For tests.
#[derive(Clone, Debug, Default)]
pub struct Endpoints {
    pub ecs: Option<String>,
    pub ec2: Option<String>,
    pub logs: Option<String>,
}

pub struct Aws {
    client: reqwest::Client,
    pub region: String,
    provider: SharedCredentialsProvider,
    cached: Mutex<Option<Credentials>>,
    endpoints: Endpoints,
}

impl Aws {
    /// Credentials and region from the default chain. `region` replaces the
    /// region of the chain.
    pub async fn connect(region: Option<String>, endpoints: Endpoints) -> anyhow::Result<Self> {
        let http_client = aws_smithy_http_client::Builder::new()
            .tls_provider(tls::Provider::Rustls(
                tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();
        let regions = RegionProviderChain::first_try(region.map(Region::new));
        let config = aws_config::defaults(BehaviorVersion::latest())
            .http_client(http_client)
            .region(regions)
            .load()
            .await;
        let region = config
            .region()
            .map(ToString::to_string)
            .context("no AWS region is set; pass --region or set AWS_REGION")?;
        let provider = config
            .credentials_provider()
            .context("no AWS credentials provider is configured")?;
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()?,
            region,
            provider,
            cached: Mutex::default(),
            endpoints,
        })
    }

    /// Cached credentials, or new ones when they expire in 5 minutes.
    async fn credentials(&self) -> anyhow::Result<Credentials> {
        let fresh = |credentials: &Credentials| {
            credentials.expiry().is_none_or(|expiry| {
                expiry
                    .duration_since(SystemTime::now())
                    .is_ok_and(|left| left > Duration::from_secs(300))
            })
        };
        if let Some(credentials) = self
            .cached
            .lock()
            .expect("credentials are never poisoned")
            .as_ref()
            .filter(|credentials| fresh(credentials))
        {
            return Ok(credentials.clone());
        }
        let credentials = self
            .provider
            .provide_credentials()
            .await
            .context("could not get AWS credentials from the default chain")?;
        *self.cached.lock().expect("credentials are never poisoned") = Some(credentials.clone());
        Ok(credentials)
    }

    fn endpoint(&self, service: Service) -> String {
        let configured = match service {
            Service::Ecs => &self.endpoints.ecs,
            Service::Ec2 => &self.endpoints.ec2,
            Service::Logs => &self.endpoints.logs,
        };
        match configured {
            Some(endpoint) => endpoint.trim_end_matches('/').to_owned(),
            None => format!(
                "https://{}.{}.amazonaws.com",
                service.signing_name(),
                self.region
            ),
        }
    }

    /// Sign and send a POST, and return the status and the body.
    async fn post(
        &self,
        service: Service,
        headers: &[(&str, String)],
        body: Vec<u8>,
    ) -> anyhow::Result<(reqwest::StatusCode, String)> {
        let url = format!("{}/", self.endpoint(service));
        let credentials = self.credentials().await?;
        let identity = credentials.into();
        let settings = SigningSettings::default();
        let parameters = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name(service.signing_name())
            .time(SystemTime::now())
            .settings(settings)
            .build()?
            .into();
        let mut request = http::Request::builder().method("POST").uri(&url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let mut request = request.body(body)?;
        let signable = SignableRequest::new(
            "POST",
            &url,
            request
                .headers()
                .iter()
                .filter_map(|(name, value)| Some((name.as_str(), value.to_str().ok()?))),
            SignableBody::Bytes(request.body()),
        )?;
        let (instructions, _) = sign(signable, &parameters)?.into_parts();
        instructions.apply_to_request_http1x(&mut request);
        let (parts, body) = request.into_parts();
        let response = self
            .client
            .post(&url)
            .headers(parts.headers)
            .body(body)
            .send()
            .await?;
        let status = response.status();
        Ok((status, response.text().await.unwrap_or_default()))
    }

    /// Call an action of a JSON API (ECS, CloudWatch Logs).
    pub async fn json(
        &self,
        service: Service,
        action: &str,
        body: &Value,
    ) -> anyhow::Result<Value> {
        let headers = [
            ("content-type", "application/x-amz-json-1.1".to_owned()),
            ("x-amz-target", format!("{}.{action}", service.target())),
        ];
        let (status, text) = self
            .post(service, &headers, serde_json::to_vec(body)?)
            .await?;
        if !status.is_success() {
            let error: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let kind = error["__type"]
                .as_str()
                .map(|kind| kind.rsplit('#').next().unwrap_or(kind).to_owned())
                .unwrap_or_default();
            let message = error["message"]
                .as_str()
                .or_else(|| error["Message"].as_str())
                .map_or(text.clone(), str::to_owned);
            bail!(
                "AWS answered {status} for {} {action}: {kind} {message}",
                service.signing_name()
            );
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Call an action of a query API (EC2), and return the XML.
    pub async fn query(
        &self,
        service: Service,
        action: &str,
        version: &str,
        parameters: &[(&str, &str)],
    ) -> anyhow::Result<String> {
        let mut form = format!("Action={action}&Version={version}");
        for (name, value) in parameters {
            form.push_str(&format!("&{}={}", encode(name), encode(value)));
        }
        let headers = [(
            "content-type",
            "application/x-www-form-urlencoded; charset=utf-8".to_owned(),
        )];
        let (status, text) = self.post(service, &headers, form.into_bytes()).await?;
        if !status.is_success() {
            let message = xml_value(&text, "Message").unwrap_or(text.clone());
            bail!(
                "AWS answered {status} for {} {action}: {message}",
                service.signing_name()
            );
        }
        Ok(text)
    }
}

/// The text of the first `<tag>` in `xml`.
pub fn xml_value(xml: &str, tag: &str) -> Option<String> {
    let start = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(xml[start..end].trim().to_owned())
}

fn encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_xml_values() {
        let xml = "<DescribeNetworkInterfacesResponse><networkInterfaceSet><item>\
                   <networkInterfaceId>eni-1</networkInterfaceId><association>\
                   <publicIp>3.4.5.6</publicIp></association></item></networkInterfaceSet>\
                   </DescribeNetworkInterfacesResponse>";
        assert_eq!(xml_value(xml, "publicIp").as_deref(), Some("3.4.5.6"));
        assert_eq!(xml_value(xml, "privateIp"), None);
        assert_eq!(encode("eni-1 a/b"), "eni-1%20a%2Fb");
    }
}
