//! Whether a terminal is busy: printing steadily (an agent's spinner, a build), then quiet again.
//!
//! Output is stamped on the IO thread as it's read; becoming busy needs no timer. Going quiet does,
//! so one shared watcher thread sleeps until the earliest moment a busy terminal could count as
//! quiet. With nothing busy it blocks without a timeout. It wakes the app only on a transition.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use crate::events::{Shared, TermEvent};

/// When output counts as work, and when work counts as finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityConfig {
    /// Output with no gap longer than `quiet_after` for this long makes the terminal busy.
    pub busy_after: Duration,
    /// A busy terminal that prints nothing for this long is idle again.
    pub quiet_after: Duration,
}

impl Default for ActivityConfig {
    fn default() -> Self {
        Self { busy_after: Duration::from_secs(3), quiet_after: Duration::from_secs(4) }
    }
}

/// A session's output timing. Times are milliseconds since `epoch`, plus one (0 = never).
pub(crate) struct Activity {
    config: ActivityConfig,
    epoch: Instant,
    last_output: AtomicU64,
    burst_start: AtomicU64,
    busy: AtomicBool,
}

impl Activity {
    pub fn new(config: ActivityConfig) -> Self {
        Self {
            config,
            epoch: Instant::now(),
            last_output: AtomicU64::new(0),
            burst_start: AtomicU64::new(0),
            busy: AtomicBool::new(false),
        }
    }

    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }

    fn at(&self, stamp: u64) -> Instant {
        self.epoch + Duration::from_millis(stamp.saturating_sub(1))
    }

    /// When a busy terminal will have been quiet long enough.
    fn quiet_at(&self) -> Instant {
        self.at(self.last_output.load(Ordering::Acquire)) + self.config.quiet_after
    }
}

/// Stamps output just read from the PTY. Called on the session's IO thread.
pub(crate) fn output(shared: &Arc<Shared>) {
    let activity = &shared.activity;
    let now = activity.now();
    let previous = activity.last_output.swap(now, Ordering::AcqRel);
    let quiet = activity.config.quiet_after.as_millis() as u64;
    if previous == 0 || now - previous >= quiet {
        activity.burst_start.store(now, Ordering::Release);
    }
    let busy_for = now - activity.burst_start.load(Ordering::Acquire);
    if busy_for >= activity.config.busy_after.as_millis() as u64 && !activity.busy.swap(true, Ordering::AcqRel) {
        (shared.sink)(TermEvent::Busy);
        let _ = watcher().send(Arc::downgrade(shared));
    }
}

/// Ends a busy spell if it has been quiet long enough. Returns whether the session is done being
/// watched (idle again, or gone).
fn settle(shared: &Shared, now: Instant) -> bool {
    let activity = &shared.activity;
    if now < activity.quiet_at() {
        return false;
    }
    if activity.busy.swap(false, Ordering::AcqRel) {
        let start = activity.at(activity.burst_start.load(Ordering::Acquire));
        let end = activity.at(activity.last_output.load(Ordering::Acquire));
        (shared.sink)(TermEvent::Idle { worked: end.saturating_duration_since(start) });
    }
    true
}

fn watcher() -> &'static Sender<Weak<Shared>> {
    static WATCHER: OnceLock<Sender<Weak<Shared>>> = OnceLock::new();
    WATCHER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("pw-term-activity".into())
            .spawn(move || watch(rx))
            .expect("spawn activity watcher");
        tx
    })
}

fn watch(rx: Receiver<Weak<Shared>>) {
    let mut busy: Vec<Weak<Shared>> = Vec::new();
    loop {
        let next = busy.iter().filter_map(Weak::upgrade).map(|s| s.activity.quiet_at()).min();
        let received = match next {
            // Nothing busy: sleep until something is.
            None => match rx.recv() {
                Ok(shared) => Some(shared),
                Err(_) => return,
            },
            Some(deadline) => match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(shared) => Some(shared),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };
        busy.extend(received);
        let now = Instant::now();
        busy.retain(|weak| weak.upgrade().is_some_and(|shared| !settle(&shared, now)));
    }
}
