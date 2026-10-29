//! The app's lease. The backend grants it to one instance for 30 seconds;
//! the holder renews it with its report every 10 seconds. An instance
//! counts the 30 seconds from when it sent the report, so it never thinks
//! it holds the lease longer than the backend does.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long a grant lasts.
pub const LEASE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Lease {
    until: Mutex<Option<Instant>>,
}

impl Lease {
    /// The answer to a report sent at `sent`.
    pub fn granted(&self, granted: bool, sent: Instant) {
        *self.until.lock().expect("the lease is never poisoned") = granted.then(|| sent + LEASE);
    }

    /// Whether this instance holds the lease at `now`.
    pub fn held(&self, now: Instant) -> bool {
        self.until
            .lock()
            .expect("the lease is never poisoned")
            .is_some_and(|until| now < until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lease_lasts_30_seconds_from_the_report() {
        let lease = Lease::default();
        let sent = Instant::now();
        assert!(!lease.held(sent));
        lease.granted(true, sent);
        assert!(lease.held(sent + Duration::from_secs(29)));
        // Reports that fail do not renew it: it ends on its own.
        assert!(!lease.held(sent + Duration::from_secs(30)));
        // A renewal moves the end.
        lease.granted(true, sent + Duration::from_secs(10));
        assert!(lease.held(sent + Duration::from_secs(39)));
        // Another instance got it.
        lease.granted(false, sent + Duration::from_secs(20));
        assert!(!lease.held(sent + Duration::from_secs(21)));
    }
}
