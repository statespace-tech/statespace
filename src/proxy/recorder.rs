//! Rows the proxy sends to Statespace for the control and the arms: runs,
//! requests, stdout, stderr, events, and lifecycle. Live is never recorded.
//! Recording never blocks or slows the app: when the buffer is full, rows
//! are dropped and counted.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use super::{assign::Reason, orchestrator::Side, remote::Remote};

/// Rows waiting to be sent, at most.
const BUFFER: usize = 50_000;
/// Rows per batch, at most.
const BATCH: usize = 100;
/// How long the proxy remembers that it recorded a subject's run.
const RUN_MEMORY: Duration = Duration::from_secs(30 * 60);
/// The tables of a batch, in order.
pub const TABLES: [&str; 6] = [
    "runs",
    "requests",
    "stdout",
    "stderr",
    "events",
    "lifecycle",
];

/// Where a row comes from: the side, the arm and its version, and the live commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Origin {
    pub side: Side,
    /// The arm's full commit, or `None` for the control.
    pub arm: Option<String>,
    /// The arm's version, or `None` for the control.
    pub version: Option<i64>,
    /// The live commit.
    pub live: Option<String>,
}

/// One finished request of the control or an arm.
pub struct RequestRow<'a> {
    pub subject: Option<&'a str>,
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub status: u16,
    pub ms: f64,
    pub first_byte_ms: Option<f64>,
    pub request_bytes: Option<u64>,
    pub response_bytes: Option<u64>,
    pub over: bool,
}

type RunKey = (Origin, String, Reason);

pub struct Recorder {
    sender: Option<mpsc::Sender<(&'static str, Value)>>,
    dropped: AtomicU64,
    runs: Mutex<HashMap<RunKey, Instant>>,
}

impl Recorder {
    /// A recorder that sends to `remote`.
    pub fn start(remote: Remote) -> Self {
        let (sender, receiver) = mpsc::channel(BUFFER);
        tokio::spawn(deliver(remote, receiver));
        Self::with_sender(Some(sender))
    }

    pub fn with_sender(sender: Option<mpsc::Sender<(&'static str, Value)>>) -> Self {
        Self {
            sender,
            dropped: AtomicU64::new(0),
            runs: Mutex::default(),
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn send(&self, table: &'static str, origin: &Origin, fields: Value) {
        let Some(sender) = &self.sender else {
            return;
        };
        let mut row = json!({
            "id": id(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "side": origin.side,
            "arm": origin.arm,
            "version": origin.version,
            "live": origin.live,
        });
        if let (Value::Object(row), Value::Object(fields)) = (&mut row, fields) {
            row.extend(fields);
        }
        if sender.try_send((table, row)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a subject's run the first time this proxy routes it, per
    /// unit, version, and reason.
    pub fn run(&self, origin: &Origin, subject: &str, reason: Reason) {
        let key = (origin.clone(), subject.to_owned(), reason);
        {
            let mut runs = self.runs.lock().expect("run memory is never poisoned");
            let now = Instant::now();
            if runs
                .get(&key)
                .is_some_and(|seen| now.duration_since(*seen) < RUN_MEMORY)
            {
                return;
            }
            if runs.len() > 200_000 {
                runs.retain(|_, seen| now.duration_since(*seen) < RUN_MEMORY);
            }
            runs.insert(key, now);
        }
        self.send(
            "runs",
            origin,
            json!({ "subject": subject, "reason": reason.as_str() }),
        );
    }

    pub fn request(&self, origin: &Origin, row: RequestRow<'_>) {
        self.send(
            "requests",
            origin,
            json!({
                "subject": row.subject,
                "method": row.method,
                "path": row.path,
                "query": row.query,
                "status": row.status,
                "ms": row.ms,
                "first_byte_ms": row.first_byte_ms,
                "request_bytes": row.request_bytes,
                "response_bytes": row.response_bytes,
                "override": row.over,
            }),
        );
    }

    /// One output line, redacted already.
    pub fn output(&self, origin: &Origin, table: &'static str, text: &str) {
        self.send(table, origin, json!({ "text": text }));
    }

    pub fn event(&self, origin: &Origin, name: &str, subject: Option<&str>) {
        self.send(
            "events",
            origin,
            json!({ "name": name, "subject": subject }),
        );
    }

    pub fn lifecycle(&self, origin: &Origin, event: &str, data: Value) {
        self.send("lifecycle", origin, json!({ "event": event, "data": data }));
    }
}

/// Send rows in batches of at most 100. A batch that keeps failing is
/// dropped and counted.
async fn deliver(remote: Remote, mut receiver: mpsc::Receiver<(&'static str, Value)>) {
    let mut pending: Vec<(&'static str, Value)> = Vec::new();
    loop {
        let deadline = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(deadline);
        while pending.len() < BATCH {
            tokio::select! {
                received = receiver.recv() => match received {
                    Some(row) => pending.push(row),
                    None if pending.is_empty() => return,
                    None => break,
                },
                _ = &mut deadline => break,
            }
        }
        if pending.is_empty() {
            continue;
        }
        let batch = batch(pending.drain(..));
        let mut delay = Duration::from_millis(500);
        for attempt in 1..=4 {
            match remote.send_events(batch.clone()).await {
                Ok(()) => break,
                Err(error) if attempt == 4 => {
                    eprintln!("[ssp] dropped {} rows: {error:#}", count(&batch));
                }
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
            }
        }
    }
}

/// `{"runs": [...], "requests": [...], ...}` with every table.
pub fn batch(rows: impl Iterator<Item = (&'static str, Value)>) -> Value {
    let mut batch: Map<String, Value> = TABLES
        .iter()
        .map(|table| ((*table).to_owned(), json!([])))
        .collect();
    for (table, row) in rows {
        if let Some(Value::Array(rows)) = batch.get_mut(table) {
            rows.push(row);
        }
    }
    Value::Object(batch)
}

fn count(batch: &Value) -> usize {
    TABLES
        .iter()
        .filter_map(|table| batch[*table].as_array())
        .map(Vec::len)
        .sum()
}

pub fn id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm() -> Origin {
        Origin {
            side: Side::Arm,
            arm: Some("a".repeat(40)),
            version: Some(3),
            live: Some("b".repeat(40)),
        }
    }

    #[tokio::test]
    async fn rows_have_the_agreed_shapes() {
        let (sender, mut receiver) = mpsc::channel(20);
        let recorder = Recorder::with_sender(Some(sender));
        let control = Origin {
            side: Side::Control,
            arm: None,
            version: None,
            live: Some("b".repeat(40)),
        };
        recorder.run(&control, "u_1", Reason::Assigned);
        recorder.run(&control, "u_1", Reason::Assigned);
        recorder.request(
            &arm(),
            RequestRow {
                subject: Some("u_1"),
                method: "GET",
                path: "/",
                query: "a=1",
                status: 200,
                ms: 3.5,
                first_byte_ms: Some(2.0),
                request_bytes: None,
                response_bytes: Some(10),
                over: true,
            },
        );
        recorder.output(&arm(), "stderr", "boom");
        recorder.event(&arm(), "lead", None);
        recorder.lifecycle(&control, "started", json!({"image": "x"}));
        let mut rows = Vec::new();
        while let Ok(row) = receiver.try_recv() {
            rows.push(row);
        }
        let tables: Vec<&str> = rows.iter().map(|(table, _)| *table).collect();
        assert_eq!(
            tables,
            ["runs", "requests", "stderr", "events", "lifecycle"]
        );
        let keys = |row: &Value| {
            let mut keys: Vec<String> = row.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        let base = ["arm", "id", "live", "side", "timestamp", "version"];
        let with = |extra: &[&str]| {
            let mut all: Vec<String> = base
                .iter()
                .chain(extra)
                .map(|key| (*key).to_owned())
                .collect();
            all.sort();
            all
        };
        assert_eq!(keys(&rows[0].1), with(&["subject", "reason"]));
        assert_eq!(rows[0].1["side"], "control");
        assert_eq!(rows[0].1["arm"], Value::Null);
        assert_eq!(rows[0].1["version"], Value::Null);
        assert_eq!(
            keys(&rows[1].1),
            with(&[
                "subject",
                "method",
                "path",
                "query",
                "status",
                "ms",
                "first_byte_ms",
                "request_bytes",
                "response_bytes",
                "override"
            ])
        );
        assert_eq!(rows[1].1["side"], "arm");
        assert_eq!(rows[1].1["version"], 3);
        assert_eq!(rows[1].1["override"], true);
        assert_eq!(keys(&rows[2].1), with(&["text"]));
        assert_eq!(keys(&rows[3].1), with(&["name", "subject"]));
        assert_eq!(keys(&rows[4].1), with(&["event", "data"]));

        let batch = batch(rows.into_iter());
        assert_eq!(count(&batch), 5);
        assert_eq!(batch["stdout"], json!([]));
    }

    #[test]
    fn a_full_buffer_drops_rows() {
        let (sender, _receiver) = mpsc::channel(1);
        let recorder = Recorder::with_sender(Some(sender));
        recorder.event(&arm(), "a", None);
        recorder.event(&arm(), "b", None);
        assert_eq!(recorder.dropped(), 1);
    }
}
