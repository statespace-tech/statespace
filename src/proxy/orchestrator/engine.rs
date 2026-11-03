//! A small client for the Docker Engine API. It talks to the daemon over
//! its socket, so `ssp proxy docker` needs no `docker` command: the `ssp`
//! image has none. `DOCKER_HOST` names the daemon (`unix://<path>` or
//! `tcp://<host>:<port>`, without TLS); the default is
//! `unix:///var/run/docker.sock`.

use std::path::PathBuf;

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{Line, Stream};
use crate::image;

/// Where the daemon listens.
#[derive(Clone, Debug)]
enum Socket {
    Unix(PathBuf),
    Tcp(String),
}

#[derive(Clone, Debug)]
pub struct Engine {
    socket: Socket,
}

impl Engine {
    /// The daemon of `DOCKER_HOST`, or of `/var/run/docker.sock`.
    pub fn connect() -> anyhow::Result<Self> {
        let host = std::env::var("DOCKER_HOST").unwrap_or_default();
        let socket = if host.is_empty() {
            Socket::Unix("/var/run/docker.sock".into())
        } else if let Some(path) = host.strip_prefix("unix://") {
            Socket::Unix(path.into())
        } else if let Some(address) = host.strip_prefix("tcp://") {
            Socket::Tcp(address.trim_end_matches('/').to_owned())
        } else {
            bail!("DOCKER_HOST is {host}; ssp proxy docker takes unix:// or tcp:// without TLS");
        };
        Ok(Self { socket })
    }

    /// Send one request on a new connection, and return the response with a
    /// body that streams.
    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        headers: &[(&str, String)],
    ) -> anyhow::Result<http::Response<Incoming>> {
        let bytes = body.map_or_else(Bytes::new, |body| Bytes::from(body.to_string()));
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(http::header::HOST, "docker");
        if body.is_some() {
            request = request.header(http::header::CONTENT_TYPE, "application/json");
        }
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let request = request.body(Full::new(bytes))?;
        let reached = || format!("could not reach the Docker daemon at {:?}", self.socket);
        let mut sender = match &self.socket {
            Socket::Unix(path) => {
                let stream = tokio::net::UnixStream::connect(path)
                    .await
                    .with_context(reached)?;
                let (sender, connection) =
                    hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
                tokio::spawn(connection);
                sender
            }
            Socket::Tcp(address) => {
                let stream = tokio::net::TcpStream::connect(address)
                    .await
                    .with_context(reached)?;
                let (sender, connection) =
                    hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
                tokio::spawn(connection);
                sender
            }
        };
        Ok(sender.send_request(request).await?)
    }

    async fn text(response: http::Response<Incoming>) -> anyhow::Result<(StatusCode, String)> {
        let status = response.status();
        let body = response.into_body().collect().await?.to_bytes();
        Ok((status, String::from_utf8_lossy(&body).into_owned()))
    }

    /// Call the API and return its JSON answer; an error for a status that
    /// is not 2xx.
    pub async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let response = self.send(method.clone(), path, body, &[]).await?;
        let (status, text) = Self::text(response).await?;
        if !status.is_success() {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| Some(value["message"].as_str()?.to_owned()))
                .unwrap_or(text);
            bail!("Docker answered {status} for {method} {path}: {message}");
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// A container or image by name or ID, unless it does not exist.
    async fn find(&self, path: &str) -> anyhow::Result<Option<Value>> {
        let response = self.send(Method::GET, path, None, &[]).await?;
        let (status, text) = Self::text(response).await?;
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("Docker answered {status} for {path}: {}", text.trim());
        }
        Ok(Some(serde_json::from_str(&text)?))
    }

    pub async fn container(&self, name: &str) -> anyhow::Result<Option<Value>> {
        self.find(&format!("/containers/{}/json", segment(name)))
            .await
    }

    pub async fn image(&self, name: &str) -> anyhow::Result<Option<Value>> {
        self.find(&format!("/images/{}/json", image_path(name)))
            .await
    }

    /// The containers with every label of `labels` (`key=value`), also the
    /// ones that stopped.
    pub async fn containers(&self, labels: &[String]) -> anyhow::Result<Vec<Value>> {
        let filters = json!({"label": labels}).to_string();
        let listed = self
            .call(
                Method::GET,
                &format!("/containers/json?all=1&filters={}", segment(&filters)),
                None,
            )
            .await?;
        Ok(listed.as_array().cloned().unwrap_or_default())
    }

    /// Pull `reference` unless the daemon has it. Credentials come from the
    /// `auths` of `$DOCKER_CONFIG/config.json` (default
    /// `~/.docker/config.json`); credential helpers are not read.
    pub async fn pull(&self, reference: &str) -> anyhow::Result<()> {
        if self.image(reference).await?.is_some() {
            return Ok(());
        }
        eprintln!("[ssp] pulling {reference}");
        let repository = image::repository(reference);
        let tag = reference
            .strip_prefix(repository)
            .map(|rest| rest.trim_start_matches([':', '@']))
            .filter(|rest| !rest.is_empty())
            .unwrap_or("latest");
        let path = format!(
            "/images/create?fromImage={}&tag={}",
            segment(repository),
            segment(tag)
        );
        let mut headers = Vec::new();
        if let Some(auth) = registry_auth(repository) {
            headers.push(("x-registry-auth", auth));
        }
        let response = self.send(Method::POST, &path, None, &headers).await?;
        let (status, text) = Self::text(response).await?;
        // The daemon answers 200 and then reports a failure in the stream.
        let failure = text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|line| line["error"].as_str().map(str::to_owned));
        match failure {
            Some(error) => bail!("could not pull {reference}: {error}"),
            None if !status.is_success() => {
                bail!(
                    "could not pull {reference}: Docker answered {status}: {}",
                    text.trim()
                )
            }
            None => Ok(()),
        }
    }

    /// Wait for a container to stop; its exit code, if Docker knows it.
    pub async fn wait(&self, id: &str) -> Option<i64> {
        let answer = self
            .call(
                Method::POST,
                &format!("/containers/{}/wait", segment(id)),
                None,
            )
            .await
            .ok()?;
        answer["StatusCode"].as_i64()
    }

    /// Stop a container, give it `seconds` to finish, and remove it with its
    /// anonymous volumes.
    pub async fn remove(&self, id: &str, seconds: u32) {
        let id = segment(id);
        let _ = self
            .call(
                Method::POST,
                &format!("/containers/{id}/stop?t={seconds}"),
                None,
            )
            .await;
        let _ = self
            .call(
                Method::DELETE,
                &format!("/containers/{id}?force=1&v=1"),
                None,
            )
            .await;
    }

    /// Send each line of a container to `sender`, from `since` (Unix
    /// seconds) or from its start, until the container stops.
    pub async fn follow(
        &self,
        id: &str,
        since: Option<i64>,
        tty: bool,
        sender: mpsc::Sender<Line>,
    ) {
        let mut path = format!(
            "/containers/{}/logs?follow=1&stdout=1&stderr=1",
            segment(id)
        );
        if let Some(since) = since {
            path.push_str(&format!("&since={since}"));
        }
        let mut body = match self.send(Method::GET, &path, None, &[]).await {
            Ok(response) if response.status().is_success() => response.into_body(),
            _ => {
                eprintln!("[ssp] could not follow the output of container {id}");
                return;
            }
        };
        let mut lines = Lines::new(tty);
        while let Some(Ok(frame)) = body.frame().await {
            let Ok(data) = frame.into_data() else {
                continue;
            };
            for line in lines.push(&data) {
                if sender.send(line).await.is_err() {
                    return;
                }
            }
        }
        for line in lines.finish() {
            let _ = sender.send(line).await;
        }
    }
}

/// Splits the log stream of a container into lines. Without a TTY, Docker
/// sends frames: one byte for the stream, three zero bytes, the size as four
/// bytes, then the data. With a TTY, it sends the output as it is.
pub struct Lines {
    tty: bool,
    buffer: Vec<u8>,
    partial: [Vec<u8>; 2],
}

impl Lines {
    pub fn new(tty: bool) -> Self {
        Self {
            tty,
            buffer: Vec::new(),
            partial: [Vec::new(), Vec::new()],
        }
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<Line> {
        let mut lines = Vec::new();
        if self.tty {
            self.split(0, data, &mut lines);
            return lines;
        }
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= 8 {
            let size = u32::from_be_bytes([
                self.buffer[4],
                self.buffer[5],
                self.buffer[6],
                self.buffer[7],
            ]) as usize;
            if self.buffer.len() < 8 + size {
                break;
            }
            let index = usize::from(self.buffer[0] == 2);
            let payload: Vec<u8> = self.buffer.drain(..8 + size).skip(8).collect();
            self.split(index, &payload, &mut lines);
        }
        lines
    }

    fn split(&mut self, index: usize, data: &[u8], lines: &mut Vec<Line>) {
        let stream = if index == 1 {
            Stream::Stderr
        } else {
            Stream::Stdout
        };
        for byte in data {
            if *byte == b'\n' {
                let text = String::from_utf8_lossy(&self.partial[index])
                    .trim_end_matches('\r')
                    .to_owned();
                self.partial[index].clear();
                lines.push(Line { stream, text });
            } else {
                self.partial[index].push(*byte);
            }
        }
    }

    /// The last lines, which ended without a newline.
    pub fn finish(&mut self) -> Vec<Line> {
        let mut lines = Vec::new();
        for (index, stream) in [(0, Stream::Stdout), (1, Stream::Stderr)] {
            if !self.partial[index].is_empty() {
                let text = String::from_utf8_lossy(&self.partial[index]).into_owned();
                self.partial[index].clear();
                lines.push(Line { stream, text });
            }
        }
        lines
    }
}

/// An image name in a path: Docker takes its slashes as they are.
fn image_path(name: &str) -> String {
    name.split('/').map(segment).collect::<Vec<_>>().join("/")
}

fn segment(value: &str) -> String {
    crate::api::segment(value)
}

/// The `X-Registry-Auth` header for the registry of `repository`, from the
/// `auths` of the Docker configuration.
fn registry_auth(repository: &str) -> Option<String> {
    let directory = std::env::var_os("DOCKER_CONFIG")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".docker")))?;
    let config: Value =
        serde_json::from_slice(&std::fs::read(directory.join("config.json")).ok()?).ok()?;
    let registry = registry_of(repository);
    let auths = config["auths"].as_object()?;
    let entry = auths.iter().find_map(|(key, value)| {
        let host = key
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or_default();
        let host = if host == "index.docker.io" {
            "docker.io"
        } else {
            host
        };
        (host == registry).then_some(value)
    })?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(entry["auth"].as_str()?)
        .ok()?;
    let (username, password) = std::str::from_utf8(&decoded).ok()?.split_once(':')?;
    let auth = json!({"username": username, "password": password, "serveraddress": registry});
    Some(URL_SAFE.encode(auth.to_string()))
}

/// The registry host of a repository; Docker Hub is `docker.io`.
fn registry_of(repository: &str) -> &str {
    match repository.split_once('/') {
        Some((first, _)) if first.contains(['.', ':']) || first == "localhost" => first,
        _ => "docker.io",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frames_into_lines_of_each_stream() {
        let frame = |stream: u8, text: &str| {
            let mut bytes = vec![stream, 0, 0, 0];
            bytes.extend_from_slice(&(text.len() as u32).to_be_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes
        };
        let mut data = frame(1, "hello\nwor");
        data.extend(frame(2, "oops\r\n"));
        data.extend(frame(1, "ld\n"));
        let mut lines = Lines::new(false);
        // A frame may arrive in pieces.
        let mut got = lines.push(&data[..5]);
        got.extend(lines.push(&data[5..]));
        assert_eq!(
            got,
            [
                Line {
                    stream: Stream::Stdout,
                    text: "hello".into()
                },
                Line {
                    stream: Stream::Stderr,
                    text: "oops".into()
                },
                Line {
                    stream: Stream::Stdout,
                    text: "world".into()
                },
            ]
        );
        let mut raw = Lines::new(true);
        assert_eq!(raw.push(b"one\ntw").len(), 1);
        assert_eq!(raw.finish()[0].text, "tw");
    }

    #[test]
    fn finds_the_registry_of_a_repository() {
        assert_eq!(registry_of("ghcr.io/acme/shop"), "ghcr.io");
        assert_eq!(registry_of("127.0.0.1:5000/shop"), "127.0.0.1:5000");
        assert_eq!(registry_of("acme/shop"), "docker.io");
        assert_eq!(registry_of("python"), "docker.io");
        assert_eq!(
            image_path("ghcr.io/acme/shop@sha256:ab"),
            "ghcr.io/acme/shop%40sha256%3Aab"
        );
    }
}
