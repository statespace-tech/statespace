//! File runtimes, such as `ssp proxy s3`. Live is the bucket root, and each
//! unit is a build folder, such as `s3://acme-site/builds/a1b2c3d/`. The
//! proxy serves the files itself, like a static website endpoint, and never
//! writes to the bucket.
//!
//! A unit is ready only while the files hash of its folder is the hash that
//! CI recorded. The proxy checks each folder again every 60 seconds by
//! listing it, and reads again only the objects that changed. Live is found
//! by its files hash too: the hash of the bucket root without the folders
//! that hold the app's builds.

pub mod s3;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Context;
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use sha2::{Digest, Sha256};

use super::{Address, Build, Live, Orchestrator, OutputStream, Running, Unit, UnitKey, channel};
use crate::{cli::FileArgs, proxy::remote::Remote};

/// How long the cache keeps a file of live.
const LIVE_FOR: Duration = Duration::from_secs(10);
/// The size of the cache.
const CACHE_BYTES: usize = 256 * 1024 * 1024;
/// How many objects a check reads at the same time.
const READS: usize = 16;

/// One object of a listing, and a value that changes when its content
/// changes, such as its ETag or generation.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub key: String,
    pub version: String,
}

/// An object and the metadata the proxy serves with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    pub body: Bytes,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub cache_control: Option<String>,
}

/// A bucket store: S3 or Cloud Storage. It only reads.
pub trait Storage: Send + Sync + 'static {
    /// Every object whose key starts with `prefix`.
    fn list(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<Listed>>> + Send;
    /// An object, or `None` when there is no such key.
    fn get(
        &self,
        bucket: &str,
        key: &str,
    ) -> impl Future<Output = anyhow::Result<Option<Object>>> + Send;
}

/// What a file runtime needs of a request.
#[derive(Clone, Debug)]
pub struct FileRequest {
    pub method: Method,
    /// The path, percent-encoded, as the request has it.
    pub path: String,
    pub if_none_match: Option<String>,
}

/// The answer of a file runtime.
#[derive(Debug)]
pub struct Served {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

/// The files hash v1 of a folder: one line per file, `<sha256 hex of
/// content>  <relative path>\n`, sorted by relative path in byte order, and
/// `sha256:` with the hex SHA-256 of all lines.
pub fn files_hash<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let sorted: BTreeMap<&str, &str> = files.into_iter().collect();
    let mut digest = Sha256::new();
    for (path, content) in sorted {
        digest.update(format!("{content}  {path}\n").as_bytes());
    }
    format!("sha256:{}", hex(&digest.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `(bucket, prefix)` of a location such as `s3://acme-site/builds/a1b2c3d/`
/// with the scheme `s3`. The prefix is empty for the bucket root.
pub fn parse_location<'a>(scheme: &str, location: &'a str) -> Option<(&'a str, &'a str)> {
    let rest = location.strip_prefix(scheme)?.strip_prefix("://")?;
    let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
    let valid = !bucket.is_empty() && (prefix.is_empty() || prefix.ends_with('/'));
    valid.then_some((bucket, prefix))
}

/// The folder to leave out of live for a build at `prefix`: its parent, or
/// the folder itself when its parent is the bucket root.
pub fn excluded_folder(prefix: &str) -> &str {
    let inner = prefix.trim_end_matches('/');
    match inner.rfind('/') {
        Some(at) => &prefix[..=at],
        None => prefix,
    }
}

/// The keys to try for a request path, in order, like a static website
/// endpoint: `/` and paths that end with `/` get `index.html`; other paths
/// get the key, then `<path>/index.html`, then `<path>.html`. `None` for a
/// path that is not a plain file path, such as one with `..`.
pub fn candidates(path: &str) -> Option<Vec<String>> {
    let decoded = percent_decode(path)?;
    let relative = decoded.strip_prefix('/').unwrap_or(&decoded);
    if relative
        .split('/')
        .any(|segment| matches!(segment, "." | ".."))
        || relative.contains(['\0', '\\'])
        || relative.contains("//")
    {
        return None;
    }
    if relative.is_empty() || relative.ends_with('/') {
        return Some(vec![format!("{relative}index.html")]);
    }
    Some(vec![
        relative.to_owned(),
        format!("{relative}/index.html"),
        format!("{relative}.html"),
    ])
}

/// Whether the last segment of a path names a file extension, such as
/// `app.js`. A single-page app's fallback serves only paths without one.
pub fn has_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|last| last.trim_start_matches('.').contains('.'))
}

fn percent_decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The content type of a key by its extension.
pub fn content_type(key: &str) -> &'static str {
    let extension = key
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

/// Files in memory: one LRU of a fixed size. Arm files stay until their arm
/// stops, because a build that is ready cannot change. Live files stay 10
/// seconds. A missing key is kept too, as `None`.
pub struct Cache {
    limit: usize,
    used: usize,
    tick: u64,
    entries: HashMap<String, Entry>,
    order: BTreeMap<u64, String>,
}

struct Entry {
    object: Option<Arc<Object>>,
    until: Option<Instant>,
    tick: u64,
    size: usize,
}

impl Cache {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            used: 0,
            tick: 0,
            entries: HashMap::new(),
            order: BTreeMap::new(),
        }
    }

    /// A kept entry: `Some(None)` for a key that was missing.
    pub fn get(&mut self, key: &str, now: Instant) -> Option<Option<Arc<Object>>> {
        let expired = self
            .entries
            .get(key)?
            .until
            .is_some_and(|until| now >= until);
        if expired {
            self.remove(key);
            return None;
        }
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        self.order.remove(&entry.tick);
        entry.tick = tick;
        self.order.insert(tick, key.to_owned());
        Some(entry.object.clone())
    }

    /// Keep an entry, until `until` or for as long as space allows. An
    /// object larger than the cache is not kept.
    pub fn put(&mut self, key: &str, object: Option<Arc<Object>>, until: Option<Instant>) {
        let size = key.len() + object.as_ref().map_or(0, |object| object.body.len());
        if size > self.limit {
            return;
        }
        self.remove(key);
        while self.used + size > self.limit {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.used -= entry.size;
            }
        }
        self.tick += 1;
        self.order.insert(self.tick, key.to_owned());
        self.entries.insert(
            key.to_owned(),
            Entry {
                object,
                until,
                tick: self.tick,
                size,
            },
        );
        self.used += size;
    }

    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.order.remove(&entry.tick);
            self.used -= entry.size;
        }
    }

    /// Forget every entry whose key starts with `prefix`.
    pub fn forget(&mut self, prefix: &str) {
        let keys: Vec<String> = self
            .entries
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect();
        for key in keys {
            self.remove(&key);
        }
    }

    #[cfg(test)]
    pub fn used(&self) -> usize {
        self.used
    }
}

/// The SHA-256 of each object of a folder, by key, with the version it was
/// read at. A check reads again only the objects whose version changed.
type Known = HashMap<String, (String, String)>;

/// A unit and what its last check found.
struct Tracked {
    unit: Unit,
    checked: Option<Instant>,
    ready: bool,
    reason: Option<String>,
    known: Known,
}

/// Live's files hash, and when it was found.
#[derive(Default)]
struct LiveState {
    hash: Option<(String, Instant)>,
    known: Known,
}

struct Inner<S> {
    storage: Arc<S>,
    /// `s3` or `gs`, as locations start.
    scheme: &'static str,
    /// The subcommand: `s3` or `gcs`.
    name: &'static str,
    /// What the runtime runs, such as `folders in S3`.
    kind: &'static str,
    bucket: String,
    remote: Remote,
    cache: Option<Mutex<Cache>>,
    fallback: Option<String>,
    check_every: Duration,
    units: Mutex<HashMap<UnitKey, Tracked>>,
    live: tokio::sync::Mutex<LiveState>,
}

/// `ssp proxy s3` and `ssp proxy gcs`, over a [`Storage`].
pub struct FileRuntime<S> {
    inner: Arc<Inner<S>>,
}

impl<S: Storage> FileRuntime<S> {
    pub fn new(
        storage: S,
        names: (&'static str, &'static str, &'static str),
        bucket: String,
        files: &FileArgs,
        remote: Remote,
    ) -> Self {
        let (scheme, name, kind) = names;
        let inner = Arc::new(Inner {
            storage: Arc::new(storage),
            scheme,
            name,
            kind,
            bucket,
            remote,
            cache: files.cache.then(|| Mutex::new(Cache::new(CACHE_BYTES))),
            fallback: files
                .fallback
                .as_deref()
                .map(|file| file.trim_start_matches('/').to_owned()),
            check_every: Duration::from_secs(files.check_every.max(1)),
            units: Mutex::default(),
            live: tokio::sync::Mutex::default(),
        });
        tokio::spawn(check_units(Arc::downgrade(&inner)));
        Self { inner }
    }
}

/// Check each unit when it is new and then every `check_every`, while the
/// runtime exists.
async fn check_units<S: Storage>(inner: std::sync::Weak<Inner<S>>) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let due: Vec<Unit> = {
            let units = inner.units.lock().expect("units are never poisoned");
            units
                .values()
                .filter(|tracked| {
                    tracked
                        .checked
                        .is_none_or(|at| at.elapsed() >= inner.check_every)
                })
                .map(|tracked| tracked.unit.clone())
                .collect()
        };
        for unit in due {
            inner.check(&unit).await;
        }
    }
}

impl<S: Storage> Inner<S> {
    fn live_address(&self) -> String {
        format!("{}://{}/", self.scheme, self.bucket)
    }

    /// Check a unit's folder against its recorded hash, and keep the result
    /// while the unit is still tracked with the same build.
    async fn check(&self, unit: &Unit) {
        let Build::Files(files) = &unit.build else {
            return;
        };
        let key = unit.key();
        let short = &unit.commit[..unit.commit.len().min(7)];
        let known = {
            let units = self.units.lock().expect("units are never poisoned");
            match units.get(&key) {
                Some(tracked) if tracked.unit.build == unit.build => tracked.known.clone(),
                _ => return,
            }
        };
        let mut known = known;
        let (ready, reason) = match parse_location(self.scheme, &files.location) {
            None => (
                false,
                Some(format!(
                    "{} is not a {} location",
                    files.location, self.scheme
                )),
            ),
            Some((bucket, prefix)) => {
                match hash_folder(&self.storage, bucket, prefix, &[], &mut known).await {
                    Ok(None) => (
                        false,
                        Some(format!(
                            "the build of {short} is missing at {}",
                            files.location
                        )),
                    ),
                    Ok(Some(hash)) if hash == files.hash => (true, None),
                    Ok(Some(_)) => (
                        false,
                        Some(format!(
                            "the build of {short} at {} changed after it was recorded",
                            files.location
                        )),
                    ),
                    Err(error) => (
                        false,
                        Some(format!(
                            "could not read the build of {short} at {}: {error:#}",
                            files.location
                        )),
                    ),
                }
            }
        };
        let mut units = self.units.lock().expect("units are never poisoned");
        if let Some(tracked) = units.get_mut(&key)
            && tracked.unit.build == unit.build
        {
            if tracked.ready && !ready {
                eprintln!(
                    "[ssp] {key} is not ready: {}",
                    reason.as_deref().unwrap_or_default()
                );
            }
            tracked.checked = Some(Instant::now());
            tracked.ready = ready;
            tracked.reason = reason;
            tracked.known = known;
        }
    }

    /// Track a unit, unless it is tracked with the same build already.
    fn track(&self, unit: &Unit) -> bool {
        let mut units = self.units.lock().expect("units are never poisoned");
        if units
            .get(&unit.key())
            .is_some_and(|tracked| tracked.unit.build == unit.build)
        {
            return false;
        }
        units.insert(
            unit.key(),
            Tracked {
                unit: unit.clone(),
                checked: None,
                ready: false,
                reason: None,
                known: Known::new(),
            },
        );
        true
    }

    /// Forget the cached files of a folder.
    fn forget(&self, location: &str) {
        if let Some(cache) = &self.cache
            && let Some((bucket, prefix)) = parse_location(self.scheme, location)
        {
            cache
                .lock()
                .expect("the cache is never poisoned")
                .forget(&format!("{bucket}/{prefix}"));
        }
    }

    /// The folders of the app's builds in this bucket: the recent builds
    /// that Statespace recorded, and the tracked units.
    async fn build_folders(&self) -> anyhow::Result<HashSet<String>> {
        let recorded = self
            .remote
            .builds()
            .await
            .context("could not read the app's builds")?;
        let tracked: Vec<Build> = self
            .units
            .lock()
            .expect("units are never poisoned")
            .values()
            .map(|tracked| tracked.unit.build.clone())
            .collect();
        let mut folders = HashSet::new();
        for build in recorded
            .into_iter()
            .map(|recorded| recorded.build)
            .chain(tracked)
        {
            if let Build::Files(files) = build
                && let Some((bucket, prefix)) = parse_location(self.scheme, &files.location)
                && bucket == self.bucket
                && !prefix.is_empty()
            {
                folders.insert(excluded_folder(prefix).to_owned());
            }
        }
        Ok(folders)
    }

    /// Live's files hash, found again when it is older than `check_every`.
    async fn live_hash(&self) -> anyhow::Result<String> {
        let mut live = self.live.lock().await;
        if let Some((hash, at)) = &live.hash
            && at.elapsed() < self.check_every
        {
            return Ok(hash.clone());
        }
        let folders: Vec<String> = self.build_folders().await?.into_iter().collect();
        let hash = hash_folder(&self.storage, &self.bucket, "", &folders, &mut live.known)
            .await?
            .with_context(|| format!("{} holds no files", self.live_address()))?;
        live.hash = Some((hash.clone(), Instant::now()));
        Ok(hash)
    }

    /// An object through the cache.
    async fn object(
        &self,
        bucket: &str,
        key: &str,
        live: bool,
    ) -> anyhow::Result<Option<Arc<Object>>> {
        let cache_key = format!("{bucket}/{key}");
        if let Some(cache) = &self.cache
            && let Some(kept) = cache
                .lock()
                .expect("the cache is never poisoned")
                .get(&cache_key, Instant::now())
        {
            return Ok(kept);
        }
        let object = self.storage.get(bucket, key).await?.map(Arc::new);
        if let Some(cache) = &self.cache {
            let until = live.then(|| Instant::now() + LIVE_FOR);
            cache.lock().expect("the cache is never poisoned").put(
                &cache_key,
                object.clone(),
                until,
            );
        }
        Ok(object)
    }

    /// Answer a request from the folder at `address`.
    async fn serve(&self, address: &str, request: &FileRequest) -> anyhow::Result<Served> {
        if !matches!(request.method, Method::GET | Method::HEAD) {
            let mut served = plain(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed\n");
            served
                .headers
                .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return Ok(served);
        }
        let (bucket, prefix) = parse_location(self.scheme, address)
            .with_context(|| format!("{address} is not a {} location", self.scheme))?;
        let live = address == self.live_address();
        let Some(keys) = candidates(&request.path) else {
            return Ok(plain(StatusCode::NOT_FOUND, "Not Found\n"));
        };
        let mut found = None;
        for key in &keys {
            if let Some(object) = self.object(bucket, &format!("{prefix}{key}"), live).await? {
                found = Some((StatusCode::OK, key.clone(), object));
                break;
            }
        }
        if found.is_none()
            && let Some(fallback) = &self.fallback
            && !has_extension(&request.path)
            && let Some(object) = self
                .object(bucket, &format!("{prefix}{fallback}"), live)
                .await?
        {
            found = Some((StatusCode::OK, fallback.clone(), object));
        }
        if found.is_none()
            && let Some(object) = self
                .object(bucket, &format!("{prefix}404.html"), live)
                .await?
        {
            found = Some((StatusCode::NOT_FOUND, "404.html".into(), object));
        }
        let Some((status, key, object)) = found else {
            return Ok(plain(StatusCode::NOT_FOUND, "Not Found\n"));
        };
        Ok(respond(status, &key, &object, request))
    }
}

/// The response for an object: its metadata as headers, `304` when the
/// client has it already, and no body for `HEAD`.
fn respond(status: StatusCode, key: &str, object: &Object, request: &FileRequest) -> Served {
    let mut headers = HeaderMap::new();
    let content_type = object
        .content_type
        .as_deref()
        .filter(|kind| !matches!(*kind, "binary/octet-stream" | "application/octet-stream"))
        .unwrap_or_else(|| content_type(key));
    let mut set = |name: header::HeaderName, value: Option<&str>| {
        if let Some(value) = value.and_then(|value| HeaderValue::from_str(value).ok()) {
            headers.insert(name, value);
        }
    };
    set(header::CONTENT_TYPE, Some(content_type));
    set(header::ETAG, object.etag.as_deref());
    set(header::LAST_MODIFIED, object.last_modified.as_deref());
    set(header::CACHE_CONTROL, object.cache_control.as_deref());
    let unchanged = status == StatusCode::OK
        && object.etag.as_deref().is_some_and(|etag| {
            request.if_none_match.as_deref().is_some_and(|asked| {
                asked
                    .split(',')
                    .map(|tag| tag.trim().trim_start_matches("W/"))
                    .any(|tag| tag == "*" || tag == etag)
            })
        });
    if unchanged {
        headers.remove(header::CONTENT_TYPE);
        return Served {
            status: StatusCode::NOT_MODIFIED,
            headers,
            body: Bytes::new(),
        };
    }
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(object.body.len()));
    let body = match request.method {
        Method::HEAD => Bytes::new(),
        _ => object.body.clone(),
    };
    Served {
        status,
        headers,
        body,
    }
}

fn plain(status: StatusCode, text: &'static str) -> Served {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    Served {
        status,
        headers,
        body: Bytes::from_static(text.as_bytes()),
    }
}

/// The files hash of the objects under `prefix`, except those under the
/// folders of `excluded`, or `None` when there are none. `known` keeps the
/// hash of each object by its version, so only changed objects are read.
async fn hash_folder<S: Storage>(
    storage: &Arc<S>,
    bucket: &str,
    prefix: &str,
    excluded: &[String],
    known: &mut Known,
) -> anyhow::Result<Option<String>> {
    let listed: Vec<Listed> = storage
        .list(bucket, prefix)
        .await?
        .into_iter()
        .filter(|object| !object.key.ends_with('/'))
        .filter(|object| {
            !excluded
                .iter()
                .any(|folder| object.key.starts_with(folder.as_str()))
        })
        .collect();
    if listed.is_empty() {
        return Ok(None);
    }
    let keys: HashSet<&str> = listed.iter().map(|object| object.key.as_str()).collect();
    known.retain(|key, _| keys.contains(key.as_str()));
    let stale: Vec<Listed> = listed
        .iter()
        .filter(|object| {
            known
                .get(&object.key)
                .is_none_or(|(version, _)| *version != object.version)
        })
        .cloned()
        .collect();
    for batch in stale.chunks(READS) {
        let mut reads = tokio::task::JoinSet::new();
        for object in batch {
            let storage = storage.clone();
            let bucket = bucket.to_owned();
            let object = object.clone();
            reads.spawn(async move {
                let read = storage.get(&bucket, &object.key).await;
                (object, read)
            });
        }
        while let Some(joined) = reads.join_next().await {
            let (object, read) = joined?;
            let body = read?
                .with_context(|| format!("{} went away while it was read", object.key))?
                .body;
            known.insert(
                object.key.clone(),
                (object.version.clone(), hex(&Sha256::digest(&body))),
            );
        }
    }
    let lines: Vec<(&str, &str)> = listed
        .iter()
        .filter_map(|object| {
            let (_, content) = known.get(&object.key)?;
            Some((&object.key[prefix.len()..], content.as_str()))
        })
        .collect();
    Ok(Some(files_hash(lines)))
}

impl<S: Storage> Orchestrator for FileRuntime<S> {
    fn name(&self) -> &'static str {
        self.inner.name
    }

    async fn live(&self) -> anyhow::Result<Live> {
        self.inner.live_hash().await.map(Live::Files)
    }

    fn runs(&self, build: &Build) -> bool {
        matches!(build, Build::Files(files) if parse_location(self.inner.scheme, &files.location).is_some())
    }

    fn kind(&self) -> &'static str {
        self.inner.kind
    }

    fn live_address(&self) -> Address {
        self.inner.live_address()
    }

    /// Track the unit and check its folder now.
    async fn start(&self, unit: &Unit) -> anyhow::Result<Address> {
        let Build::Files(files) = &unit.build else {
            anyhow::bail!("{} is not a files build", unit.build);
        };
        if self.inner.track(unit) {
            self.inner.check(unit).await;
        }
        Ok(files.location.clone())
    }

    /// Forget the unit and its cached files.
    async fn stop(&self, unit: &Unit) -> anyhow::Result<()> {
        self.inner
            .units
            .lock()
            .expect("units are never poisoned")
            .remove(&unit.key());
        if let Build::Files(files) = &unit.build {
            self.inner.forget(&files.location);
        }
        Ok(())
    }

    async fn running(&self) -> anyhow::Result<Vec<Running>> {
        let units = self.inner.units.lock().expect("units are never poisoned");
        Ok(units
            .values()
            .map(|tracked| Running {
                unit: tracked.unit.clone(),
                address: match &tracked.unit.build {
                    Build::Files(files) => files.location.clone(),
                    Build::Image(_) => String::new(),
                },
                ready: tracked.ready,
                reason: match tracked.checked {
                    None => Some("checking the build".into()),
                    Some(_) => tracked.reason.clone(),
                },
            })
            .collect())
    }

    /// A unit of a file runtime prints nothing.
    fn output(&self, _unit: &Unit) -> OutputStream {
        channel().1
    }

    fn follow(&self, units: &[Unit]) {
        let gone: Vec<Unit> = {
            let mut tracked = self.inner.units.lock().expect("units are never poisoned");
            let gone = tracked
                .values()
                .filter(|tracked| !units.contains(&tracked.unit))
                .map(|tracked| tracked.unit.clone())
                .collect();
            tracked.retain(|_, tracked| {
                units.iter().any(|unit| {
                    unit.key() == tracked.unit.key() && unit.build == tracked.unit.build
                })
            });
            gone
        };
        for unit in gone {
            if let Build::Files(files) = &unit.build {
                self.inner.forget(&files.location);
            }
        }
        for unit in units {
            self.inner.track(unit);
        }
    }

    async fn serve(&self, address: &str, request: &FileRequest) -> Option<anyhow::Result<Served>> {
        Some(self.inner.serve(address, request).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_hash_matches_the_shared_vector() {
        let sha = |content: &str| hex(&Sha256::digest(content.as_bytes()));
        let (index, b, missing) = (sha("hi\n"), sha("x"), sha("<h1>404</h1>\n"));
        let hash = files_hash([
            ("index.html", index.as_str()),
            ("a/b.txt", b.as_str()),
            ("404.html", missing.as_str()),
        ]);
        assert_eq!(
            hash,
            "sha256:7726e9b6dbc42157d5aad33260684c5737842185afaf55f654496460ea3b71ea"
        );
    }

    #[test]
    fn paths_map_to_keys_like_a_website_endpoint() {
        let keys = |path: &str| candidates(path).unwrap();
        assert_eq!(keys("/"), ["index.html"]);
        assert_eq!(keys("/docs/"), ["docs/index.html"]);
        assert_eq!(keys("/about"), ["about", "about/index.html", "about.html"]);
        assert_eq!(keys("/a%20b.txt")[0], "a b.txt");
        for refused in ["/../x", "/a/./b", "/a//b", "/%2e%2e/x", "/a%5cb", "/%zz"] {
            assert_eq!(candidates(refused), None, "{refused}");
        }
        assert!(has_extension("/app.js"));
        assert!(has_extension("/a/b.min.css"));
        assert!(!has_extension("/settings/profile"));
        assert!(!has_extension("/v1.2/page"));
        assert!(!has_extension("/.well-known"));
    }

    #[test]
    fn locations_and_build_folders() {
        assert_eq!(
            parse_location("s3", "s3://acme-site/builds/a1b2c3d/"),
            Some(("acme-site", "builds/a1b2c3d/"))
        );
        assert_eq!(
            parse_location("s3", "s3://acme-site/"),
            Some(("acme-site", ""))
        );
        assert_eq!(parse_location("gs", "s3://acme-site/x/"), None);
        assert_eq!(parse_location("s3", "s3://acme-site/x"), None);
        assert_eq!(excluded_folder("builds/a1b2c3d/"), "builds/");
        assert_eq!(excluded_folder("site/builds/a1b2c3d/"), "site/builds/");
        assert_eq!(excluded_folder("a1b2c3d/"), "a1b2c3d/");
    }

    #[test]
    fn content_types_come_from_the_extension() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/app.JS"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("data"), "application/octet-stream");
    }

    fn object(size: usize) -> Option<Arc<Object>> {
        Some(Arc::new(Object {
            body: Bytes::from(vec![b'x'; size]),
            ..Object::default()
        }))
    }

    #[test]
    fn the_cache_drops_the_least_recently_used_files() {
        let now = Instant::now();
        let mut cache = Cache::new(100);
        cache.put("a", object(40), None);
        cache.put("b", object(40), None);
        // Reading a makes b the oldest.
        assert!(cache.get("a", now).is_some());
        cache.put("c", object(40), None);
        assert!(cache.get("b", now).is_none());
        assert!(cache.get("a", now).is_some());
        assert!(cache.get("c", now).is_some());
        assert!(cache.used() <= 100);
        // Too large to keep.
        cache.put("d", object(200), None);
        assert!(cache.get("d", now).is_none());
    }

    #[test]
    fn the_cache_expires_live_files_and_forgets_stopped_arms() {
        let now = Instant::now();
        let mut cache = Cache::new(1000);
        cache.put("site/index.html", object(10), Some(now + LIVE_FOR));
        cache.put("site/builds/a/index.html", object(10), None);
        cache.put("site/missing", None, Some(now + LIVE_FOR));
        assert_eq!(cache.get("site/missing", now), Some(None));
        assert!(cache.get("site/index.html", now).is_some());
        let later = now + LIVE_FOR + Duration::from_secs(1);
        assert!(cache.get("site/index.html", later).is_none());
        assert!(cache.get("site/builds/a/index.html", later).is_some());
        cache.forget("site/builds/a/");
        assert!(cache.get("site/builds/a/index.html", later).is_none());
    }

    fn request(method: Method, path: &str, if_none_match: Option<&str>) -> FileRequest {
        FileRequest {
            method,
            path: path.into(),
            if_none_match: if_none_match.map(str::to_owned),
        }
    }

    #[test]
    fn responses_carry_the_object_metadata() {
        let object = Object {
            body: Bytes::from_static(b"hi\n"),
            content_type: Some("binary/octet-stream".into()),
            etag: Some("\"e1\"".into()),
            last_modified: Some("Tue, 03 Nov 2026 06:30:14 GMT".into()),
            cache_control: None,
        };
        let served = respond(
            StatusCode::OK,
            "index.html",
            &object,
            &request(Method::GET, "/", None),
        );
        assert_eq!(served.status, StatusCode::OK);
        assert_eq!(
            served.headers[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert_eq!(served.headers[header::ETAG], "\"e1\"");
        assert_eq!(served.body, "hi\n");
        let head = respond(
            StatusCode::OK,
            "index.html",
            &object,
            &request(Method::HEAD, "/", None),
        );
        assert_eq!(head.headers[header::CONTENT_LENGTH], "3");
        assert!(head.body.is_empty());
        let unchanged = respond(
            StatusCode::OK,
            "index.html",
            &object,
            &request(Method::GET, "/", Some("\"e0\", \"e1\"")),
        );
        assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED);
        assert!(unchanged.body.is_empty());
    }
}
