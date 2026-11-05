//! Counts per visitor: what the proxy sends to Statespace. For each request
//! that an arm serves to an assigned visitor, the proxy adds to that
//! visitor's counters in memory: the requests, the 5xx errors, a latency
//! bucket, and each event that matches. Live and overrides are never
//! counted. A visitor whose arm is not ready is counted with no requests and
//! `unavailable: true`.
//!
//! Every 60 seconds and when the proxy stops, the counters that changed are
//! sent as deltas to `POST /v1/runtime/apps/{app}/counts`, in batches with a
//! UUID, and then cleared. A batch that fails is sent again with the same
//! UUID, with backoff. Counting never blocks or slows a request: above
//! 50,000 visitors in memory, the oldest deltas are dropped.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, SecondsFormat, Utc};
use ring::hmac;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::remote::{CountsError, Remote};

/// The upper bounds of the latency buckets in milliseconds. The last bucket
/// has no bound. Never change them: Statespace adds the buckets of every
/// proxy.
pub const LATENCY_BOUNDS: [f64; 10] = [
    5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0,
];
/// How many latency buckets a visitor has.
pub const BUCKETS: usize = LATENCY_BOUNDS.len() + 1;
/// The most entries in one batch.
const BATCH_ENTRIES: usize = 5_000;
/// The most visitors whose deltas the proxy keeps in memory.
const MOST_VISITORS: usize = 50_000;
/// The longest wait before a failed batch is sent again.
const MOST_BACK_OFF: Duration = Duration::from_secs(60);
/// How often a batch is sent while the account's storage is full, before it
/// is dropped.
const FULL_TRIES: u32 = 5;

/// The bucket of a latency: the first bucket whose upper bound is at least
/// `ms`, else the last one.
pub fn latency_bucket(ms: f64) -> usize {
    LATENCY_BOUNDS
        .iter()
        .position(|bound| ms <= *bound)
        .unwrap_or(BUCKETS - 1)
}

/// The hash of a subject that leaves the proxy:
/// `hex(HMAC-SHA256(key = app salt, message = subject))`. The raw subject
/// never leaves it. This is not the assignment point (`assign::bucket`).
pub fn subject_hash(salt: &str, subject: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, salt.as_bytes());
    hmac::sign(&key, subject.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One visitor of one arm of one version of an experiment.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Visitor {
    pub experiment: String,
    pub version: i64,
    /// The arm's full commit.
    pub arm: String,
    /// The subject hash, from [`subject_hash`].
    pub subject: String,
}

/// What changed for a visitor since the last send.
#[derive(Clone, Debug, PartialEq)]
pub struct Delta {
    first_at: DateTime<Utc>,
    last_at: DateTime<Utc>,
    /// Whether the visitor's arm was not ready, and no arm request counted.
    unavailable: bool,
    requests: u64,
    errors: u64,
    latency: [u64; BUCKETS],
    events: BTreeMap<String, u64>,
}

impl Delta {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            first_at: now,
            last_at: now,
            unavailable: false,
            requests: 0,
            errors: 0,
            latency: [0; BUCKETS],
            events: BTreeMap::new(),
        }
    }
}

/// The deltas in memory, oldest first.
#[derive(Default)]
struct Pending {
    deltas: HashMap<Visitor, (u64, Delta)>,
    order: BTreeMap<u64, Visitor>,
    next: u64,
}

/// The deltas of visitors, in order.
type Entries = Vec<(Visitor, Delta)>;

/// A batch to send: its UUID, and its entries.
struct Batch {
    id: String,
    entries: Entries,
    /// How often Statespace answered that the account's storage is full.
    full: u32,
}

pub struct Counter {
    pending: Mutex<Pending>,
    /// The entries in batches that wait to be sent.
    held: AtomicUsize,
    /// Visitors whose deltas were dropped because memory was full.
    dropped: AtomicU64,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            pending: Mutex::default(),
            held: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }
}

impl Counter {
    /// A visitor whose arm is not ready: the request goes to live, and the
    /// visitor counts for the split check only.
    pub fn unavailable(&self, visitor: Visitor) {
        self.add(visitor, |delta, new| {
            if new {
                delta.unavailable = true;
            }
        });
    }

    /// A request that an arm served: its status, the time to the response
    /// headers, and the events that matched.
    pub fn served(&self, visitor: Visitor, status: u16, ms: f64, events: &[&str]) {
        self.add(visitor, |delta, _| {
            delta.unavailable = false;
            delta.requests += 1;
            if (500..600).contains(&status) {
                delta.errors += 1;
            }
            delta.latency[latency_bucket(ms)] += 1;
            for name in events {
                *delta.events.entry((*name).to_owned()).or_default() += 1;
            }
        });
    }

    fn add(&self, visitor: Visitor, change: impl FnOnce(&mut Delta, bool)) {
        let now = Utc::now();
        let mut pending = self.pending.lock().expect("counts are never poisoned");
        let pending = &mut *pending;
        if let Some((_, delta)) = pending.deltas.get_mut(&visitor) {
            delta.last_at = now;
            change(delta, false);
            return;
        }
        let mut delta = Delta::new(now);
        change(&mut delta, true);
        let order = pending.next;
        pending.next += 1;
        pending.order.insert(order, visitor.clone());
        pending.deltas.insert(visitor, (order, delta));
        let room = MOST_VISITORS.saturating_sub(self.held.load(Ordering::Relaxed));
        while pending.deltas.len() > room {
            let Some((_, oldest)) = pending.order.pop_first() else {
                break;
            };
            pending.deltas.remove(&oldest);
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// How many visitors' deltas were dropped because memory was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Take every delta, oldest first.
    fn take(&self) -> Entries {
        let mut pending = self.pending.lock().expect("counts are never poisoned");
        let Pending { deltas, order, .. } = std::mem::take(&mut *pending);
        let mut deltas = deltas;
        order
            .into_values()
            .filter_map(|visitor| {
                let (_, delta) = deltas.remove(&visitor)?;
                Some((visitor, delta))
            })
            .collect()
    }

    /// Send the deltas every `every` (60 seconds) until `stop` turns true,
    /// then send what is left once.
    pub async fn deliver(
        self: Arc<Self>,
        remote: Remote,
        every: Duration,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut outbox: VecDeque<Batch> = VecDeque::new();
        let mut flushed = Instant::now();
        let mut back_off = Duration::from_secs(1);
        let mut failing = false;
        loop {
            let wait = match outbox.is_empty() || !failing {
                true => every.saturating_sub(flushed.elapsed()),
                false => back_off.min(every.saturating_sub(flushed.elapsed())),
            };
            let stopping = tokio::select! {
                _ = tokio::time::sleep(wait) => false,
                _ = stop.wait_for(|stop| *stop) => true,
            };
            if stopping || flushed.elapsed() >= every {
                outbox.extend(batches(self.take()));
                flushed = Instant::now();
            }
            self.held.store(
                outbox.iter().map(|batch| batch.entries.len()).sum(),
                Ordering::Relaxed,
            );
            match self.send(&remote, &mut outbox).await {
                Ok(()) => {
                    failing = false;
                    back_off = Duration::from_secs(1);
                }
                Err(error) => {
                    if !failing {
                        eprintln!("[ssp] could not send counts: {error:#}; trying again");
                    }
                    failing = true;
                    back_off = (back_off * 2).min(MOST_BACK_OFF);
                }
            }
            self.held.store(
                outbox.iter().map(|batch| batch.entries.len()).sum(),
                Ordering::Relaxed,
            );
            if stopping {
                return;
            }
        }
    }

    /// Send the batches in order. A batch that Statespace refuses (400) is
    /// dropped. For versions that do not run (409), their entries are dropped
    /// and the others go again in a new batch. While the account's storage is
    /// full (507), a batch is sent again a few times, then dropped. The first
    /// other failure stops, and its batch waits for the next try with the
    /// same UUID.
    async fn send(&self, remote: &Remote, outbox: &mut VecDeque<Batch>) -> anyhow::Result<()> {
        while let Some(batch) = outbox.front_mut() {
            match remote.send_counts(&body(batch)).await {
                Ok(()) => {}
                Err(CountsError::Refused(message)) => eprintln!(
                    "[ssp] dropped the counts of {} visitors: {message}",
                    batch.entries.len()
                ),
                Err(CountsError::NotRunning(message, versions)) => {
                    let (dropped, kept): (Entries, Entries) = std::mem::take(&mut batch.entries)
                        .into_iter()
                        .partition(|(visitor, _)| {
                            versions.is_empty()
                                || versions.iter().any(|(experiment, version)| {
                                    visitor.experiment == *experiment && visitor.version == *version
                                })
                        });
                    eprintln!(
                        "[ssp] dropped the counts of {} visitors: {message}",
                        dropped.len()
                    );
                    outbox.pop_front();
                    if !kept.is_empty() {
                        outbox.push_front(Batch {
                            id: uuid(),
                            entries: kept,
                            full: 0,
                        });
                    }
                    continue;
                }
                Err(CountsError::Full(message)) => {
                    batch.full += 1;
                    if batch.full < FULL_TRIES {
                        return Err(anyhow::anyhow!("{message}"));
                    }
                    eprintln!(
                        "[ssp] dropped the counts of {} visitors: {message}",
                        batch.entries.len()
                    );
                }
                Err(CountsError::Other(error)) => return Err(error),
            }
            outbox.pop_front();
        }
        Ok(())
    }
}

/// Batches of at most 5,000 entries, each of one experiment version, so a
/// version that stopped (409) drops only its own counts.
fn batches(entries: Entries) -> Vec<Batch> {
    let mut versions: Vec<((String, i64), Entries)> = Vec::new();
    for (visitor, delta) in entries {
        let version = (visitor.experiment.clone(), visitor.version);
        match versions.iter_mut().find(|(key, _)| *key == version) {
            Some((_, list)) => list.push((visitor, delta)),
            None => versions.push((version, vec![(visitor, delta)])),
        }
    }
    let mut batches = Vec::new();
    for (_, mut list) in versions {
        while !list.is_empty() {
            let rest = list.split_off(list.len().min(BATCH_ENTRIES));
            batches.push(Batch {
                id: uuid(),
                entries: std::mem::replace(&mut list, rest),
                full: 0,
            });
        }
    }
    batches
}

/// The request body of a batch.
fn body(batch: &Batch) -> Value {
    let time = |at: &DateTime<Utc>| at.to_rfc3339_opts(SecondsFormat::Secs, true);
    let counts: Vec<Value> = batch
        .entries
        .iter()
        .map(|(visitor, delta)| {
            json!({
                "experiment": visitor.experiment,
                "version": visitor.version,
                "arm": visitor.arm,
                "subject": visitor.subject,
                "first_at": time(&delta.first_at),
                "last_at": time(&delta.last_at),
                "unavailable": delta.unavailable,
                "requests": delta.requests,
                "errors": delta.errors,
                "latency": delta.latency,
                "events": delta.events,
            })
        })
        .collect();
    json!({ "batch": batch.id, "counts": counts })
}

/// A random UUID (version 4).
fn uuid() -> String {
    let mut bytes = rand::random::<u128>().to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visitor(experiment: &str, subject: &str) -> Visitor {
        Visitor {
            experiment: experiment.into(),
            version: 4,
            arm: "a".repeat(40),
            subject: subject_hash("s", subject),
        }
    }

    #[test]
    fn subjects_leave_as_an_hmac_of_the_app_salt() {
        // The shared vector: salt "s", subject "v_1".
        assert_eq!(
            subject_hash("s", "v_1"),
            "cf22ab595d8c337278d4556c963a4cf3ad5d2323326a9c051dcb17e298f7d89b"
        );
        assert_ne!(subject_hash("t", "v_1"), subject_hash("s", "v_1"));
    }

    #[test]
    fn latency_buckets_hold_their_upper_bounds() {
        assert_eq!(BUCKETS, 11);
        assert_eq!(latency_bucket(0.0), 0);
        assert_eq!(latency_bucket(5.0), 0);
        assert_eq!(latency_bucket(5.001), 1);
        assert_eq!(latency_bucket(10.0), 1);
        assert_eq!(latency_bucket(25.0), 2);
        assert_eq!(latency_bucket(84.0), 4);
        assert_eq!(latency_bucket(1000.0), 7);
        assert_eq!(latency_bucket(5000.0), 9);
        assert_eq!(latency_bucket(5000.1), 10);
        assert_eq!(latency_bucket(f64::INFINITY), 10);
        assert_eq!(latency_bucket(f64::NAN), 10);
    }

    #[test]
    fn deltas_add_up_per_visitor() {
        let counter = Counter::default();
        counter.served(visitor("hero", "v_1"), 201, 3.0, &["signup"]);
        counter.served(visitor("hero", "v_1"), 503, 120.0, &[]);
        counter.served(visitor("hero", "v_1"), 200, 7000.0, &["signup", "view"]);
        counter.unavailable(visitor("hero", "v_2"));
        // A visitor that an arm serves later is no longer unavailable.
        counter.unavailable(visitor("hero", "v_3"));
        counter.served(visitor("hero", "v_3"), 200, 1.0, &[]);
        let entries = counter.take();
        assert_eq!(entries.len(), 3);
        let batch = Batch {
            id: uuid(),
            entries,
            full: 0,
        };
        let body = body(&batch);
        let counts = body["counts"].as_array().unwrap();
        assert_eq!(counts[0]["subject"], subject_hash("s", "v_1"));
        assert_eq!(counts[0]["requests"], 3);
        assert_eq!(counts[0]["errors"], 1);
        assert_eq!(
            counts[0]["latency"],
            json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1])
        );
        assert_eq!(counts[0]["events"], json!({"signup": 2, "view": 1}));
        assert_eq!(counts[0]["unavailable"], false);
        assert_eq!(counts[0]["version"], 4);
        assert_eq!(counts[1]["unavailable"], true);
        assert_eq!(counts[1]["requests"], 0);
        assert_eq!(counts[1]["events"], json!({}));
        assert_eq!(counts[2]["unavailable"], false);
        let mut keys: Vec<&str> = counts[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "arm",
                "errors",
                "events",
                "experiment",
                "first_at",
                "last_at",
                "latency",
                "requests",
                "subject",
                "unavailable",
                "version"
            ]
        );
        assert!(counts[0]["first_at"].as_str().unwrap().ends_with('Z'));
        // The deltas are cleared once taken.
        assert!(counter.take().is_empty());
    }

    #[test]
    fn batches_hold_one_version_and_at_most_5000_entries() {
        let counter = Counter::default();
        for index in 0..5_001 {
            counter.served(visitor("hero", &format!("v_{index}")), 200, 1.0, &[]);
        }
        counter.served(visitor("pricing", "v_1"), 200, 1.0, &[]);
        let batches = batches(counter.take());
        let sizes: Vec<usize> = batches.iter().map(|batch| batch.entries.len()).collect();
        assert_eq!(sizes, [5_000, 1, 1]);
        assert_eq!(batches[2].entries[0].0.experiment, "pricing");
        assert_ne!(batches[0].id, batches[1].id);
        let id = &batches[0].id;
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }

    #[test]
    fn a_full_memory_drops_the_oldest_visitors() {
        let counter = Counter::default();
        counter.held.store(MOST_VISITORS - 2, Ordering::Relaxed);
        for subject in ["v_1", "v_2", "v_3"] {
            counter.served(visitor("hero", subject), 200, 1.0, &[]);
        }
        assert_eq!(counter.dropped(), 1);
        let kept: Vec<String> = counter
            .take()
            .into_iter()
            .map(|(visitor, _)| visitor.subject)
            .collect();
        assert_eq!(kept, [subject_hash("s", "v_2"), subject_hash("s", "v_3")]);
    }
}
