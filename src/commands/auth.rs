//! Browser sign-in with device approval.

use std::time::{Duration, Instant};

use anyhow::bail;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    api::{Api, decode},
    config::Settings,
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
    let me: Value = api.with_token(token.clone()).get("/v1/me")?;
    Settings {
        endpoint: Some(api.endpoint().into()),
        token: Some(token),
    }
    .save()?;
    emit(
        &json!({ "status": "signed_in", "person": me["person"], "workspaces": me["workspaces"] }),
        || {
            let workspaces: Vec<String> = me["workspaces"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|workspace| text(&workspace["login"]))
                .collect();
            let login = text(&me["person"]["login"]);
            match workspaces.as_slice() {
                [] => format!(
                    "Signed in as {login}. Install the Statespace GitHub App at \
                     https://statespace.com to make a workspace."
                ),
                [one] => format!("Signed in as {login}, in the workspace {one}."),
                many => format!(
                    "Signed in as {login}. Your workspaces: {}. Commands use the owner of \
                     their repository, or --workspace.",
                    many.join(", ")
                ),
            }
        },
    )
}

/// Sign this terminal out, or every session of the account with `all`. The
/// saved session is forgotten even when Statespace cannot be reached.
pub fn logout(api: &Api, all: bool) -> anyhow::Result<()> {
    let ended = match api.token() {
        Some(_) if all => api
            .send::<Value>(
                Method::POST,
                "/v1/tokens/revoke-all",
                &json!({ "kind": "session" }),
            )
            .map(|response| response["revoked"].clone()),
        Some(_) => api.delete("/v1/auth/session").map(|()| json!(1)),
        None => Ok(json!(0)),
    };
    Settings {
        endpoint: Some(api.endpoint().into()),
        token: None,
    }
    .save()?;
    let ended = ended?;
    emit(
        &json!({ "status": "signed_out", "sessions_ended": ended }),
        || {
            if all {
                format!("Signed out of {} sessions.", text(&ended))
            } else {
                "Signed out.".into()
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
