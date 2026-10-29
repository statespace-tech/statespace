//! Browser sign-in with device approval.

use std::time::{Duration, Instant};

use anyhow::bail;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    api::{Api, decode},
    config::Settings,
    credential::{Found, Source},
    output::{emit, text},
};

#[derive(Deserialize)]
struct DeviceLogin {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct Session {
    token: String,
}

pub fn login(api: &Api, no_open: bool) -> anyhow::Result<()> {
    let login: DeviceLogin = decode(api.anonymous(Method::POST, "/v1/auth/device").send()?)?;
    eprintln!("Confirm this code in your browser: {}", login.user_code);
    if no_open || webbrowser::open(&login.verification_url).is_err() {
        eprintln!("Open {}", login.verification_url);
    }
    let token = wait_for_approval(api, &login)?;
    let me: Value = api
        .with_credential(Found::Token(Source::Session, token.clone()))
        .get("/v1/whoami")?;
    Settings {
        endpoint: Some(api.endpoint().into()),
        token: Some(token),
    }
    .save()?;
    emit(&json!({ "status": "signed_in", "whoami": me }), || {
        let login = me["person"]["login"]
            .as_str()
            .map_or_else(|| text(&me["login"]), str::to_owned);
        format!("Signed in as {login}.")
    })
}

/// Sign this terminal out. The saved session is forgotten even when
/// Statespace cannot be reached.
pub fn logout(api: &Api) -> anyhow::Result<()> {
    let settings = Settings::load()?;
    let ended = match &settings.token {
        Some(token) => api
            .with_credential(Found::Token(Source::Session, token.clone()))
            .send::<Value>(Method::DELETE, "/v1/auth/session", &json!({}))
            .map(|_| true),
        None => Ok(false),
    };
    Settings {
        endpoint: settings.endpoint,
        token: None,
    }
    .save()?;
    let ended = ended?;
    emit(
        &json!({ "status": "signed_out", "session_ended": ended }),
        || {
            if ended {
                "Signed out.".into()
            } else {
                "No session was saved.".into()
            }
        },
    )
}

fn wait_for_approval(api: &Api, login: &DeviceLogin) -> anyhow::Result<String> {
    let deadline = Instant::now() + Duration::from_secs(login.expires_in);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(login.interval.max(1)));
        let response = api
            .anonymous(Method::POST, "/v1/auth/device/token")
            .json(&json!({ "device_code": login.device_code }))
            .send()?;
        if response.status() != StatusCode::ACCEPTED {
            return Ok(decode::<Session>(response)?.token);
        }
    }
    bail!("sign-in expired; run `ssp login` again")
}
