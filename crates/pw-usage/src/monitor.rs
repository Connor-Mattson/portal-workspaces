//! The usage daemon: one thread that polls every profile on a schedule.
//!
//! The thread sleeps until the next profile is due (or a command arrives), polls the due profiles
//! in parallel, and hands each result to a callback. Dropping the [`Monitor`] stops it.

use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use pw_model::{ProfileId, UsageProfile};

use crate::report::{Failure, Report};
use crate::{Poller, ProviderState};

/// How often a healthy profile is polled.
pub const INTERVAL: Duration = Duration::from_secs(120);
/// The longest wait between attempts after repeated failures.
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);
/// A manual refresh within this long of the last poll is ignored (no hammering the endpoints).
pub const MIN_REFRESH_GAP: Duration = Duration::from_secs(10);

/// One poll's outcome, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub profile: ProfileId,
    pub result: Result<Report, Failure>,
    /// When the monitor will try again.
    pub next_poll: SystemTime,
}

enum Command {
    SetProfiles(Vec<UsageProfile>),
    Refresh(Option<ProfileId>),
}

pub struct Monitor {
    tx: mpsc::Sender<Command>,
}

impl Monitor {
    /// Starts the daemon. `on_update` runs on the monitor thread.
    pub fn spawn(poller: impl Poller, on_update: impl Fn(Update) + Send + Sync + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("usage-monitor".into())
            .spawn(move || run(&poller, &rx, &on_update))
            .expect("spawn usage monitor");
        Self { tx }
    }

    /// Replaces the tracked profiles. New or changed profiles are polled right away.
    pub fn set_profiles(&self, profiles: Vec<UsageProfile>) {
        let _ = self.tx.send(Command::SetProfiles(profiles));
    }

    /// Polls one profile (or all of them) now, unless they were polled moments ago.
    pub fn refresh(&self, profile: Option<ProfileId>) {
        let _ = self.tx.send(Command::Refresh(profile));
    }
}

struct Slot {
    profile: UsageProfile,
    state: ProviderState,
    due: Instant,
    last_poll: Option<Instant>,
    failures: u32,
}

fn run(poller: &impl Poller, rx: &mpsc::Receiver<Command>, on_update: &(impl Fn(Update) + Sync)) {
    let mut slots: Vec<Slot> = Vec::new();
    loop {
        let next_due = slots.iter().map(|s| s.due).min();
        let command = match next_due {
            Some(due) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Command::SetProfiles(profiles)) => set_profiles(&mut slots, profiles),
            Ok(Command::Refresh(target)) => {
                let now = Instant::now();
                for slot in slots.iter_mut().filter(|s| target.is_none_or(|t| t == s.profile.id)) {
                    if slot.last_poll.is_none_or(|t| now.duration_since(t) >= MIN_REFRESH_GAP) {
                        slot.due = now;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        let now = Instant::now();
        let mut due: Vec<&mut Slot> = slots.iter_mut().filter(|s| s.due <= now).collect();
        if due.is_empty() {
            continue;
        }
        // Profiles are independent (different hosts, files and CLIs), so one slow one never
        // holds up the rest.
        thread::scope(|scope| {
            for slot in due.iter_mut() {
                scope.spawn(move || {
                    let result = poller.poll(&slot.profile, &mut slot.state);
                    let finished = Instant::now();
                    slot.failures = if result.is_ok() { 0 } else { slot.failures + 1 };
                    let delay = next_delay(&result, slot.failures);
                    slot.due = finished + delay;
                    slot.last_poll = Some(finished);
                    on_update(Update { profile: slot.profile.id, result, next_poll: SystemTime::now() + delay });
                });
            }
        });
    }
}

fn set_profiles(slots: &mut Vec<Slot>, profiles: Vec<UsageProfile>) {
    let mut old: HashMap<ProfileId, Slot> = slots.drain(..).map(|s| (s.profile.id, s)).collect();
    let now = Instant::now();
    for profile in profiles {
        match old.remove(&profile.id) {
            // Only a rename: keep the schedule.
            Some(mut slot)
                if slot.profile.provider == profile.provider && slot.profile.instructions == profile.instructions =>
            {
                slot.profile = profile;
                slots.push(slot);
            }
            _ => slots.push(Slot { profile, state: ProviderState::default(), due: now, last_poll: None, failures: 0 }),
        }
    }
}

/// How long to wait after a poll. Successes and idle accounts (a cheap local check) come back on
/// the regular interval; failures back off so a dead network or a bad sign-in isn't hammered.
pub(crate) fn next_delay(result: &Result<Report, Failure>, failures: u32) -> Duration {
    let backoff = || INTERVAL.saturating_mul(1 << failures.saturating_sub(1).min(4)).min(MAX_BACKOFF);
    match result {
        Ok(_) | Err(Failure::Idle { .. } | Failure::NotFound(_)) => INTERVAL,
        Err(Failure::RateLimited { retry_after }) => (*retry_after).max(backoff()),
        Err(Failure::SignedOut(_) | Failure::Unavailable(_)) => backoff(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use pw_model::Provider;

    use super::*;

    fn unavailable() -> Result<Report, Failure> {
        Err(Failure::Unavailable("offline".into()))
    }

    #[test]
    fn backoff_doubles_up_to_a_cap() {
        let ok = Ok(Report::new(Vec::new(), SystemTime::UNIX_EPOCH, true));
        assert_eq!(next_delay(&ok, 0), INTERVAL);
        assert_eq!(next_delay(&unavailable(), 1), Duration::from_secs(120));
        assert_eq!(next_delay(&unavailable(), 2), Duration::from_secs(240));
        assert_eq!(next_delay(&unavailable(), 3), Duration::from_secs(480));
        assert_eq!(next_delay(&unavailable(), 4), MAX_BACKOFF);
        assert_eq!(next_delay(&unavailable(), 40), MAX_BACKOFF);
        let limited = Err(Failure::RateLimited { retry_after: Duration::from_secs(600) });
        assert_eq!(next_delay(&limited, 1), Duration::from_secs(600));
        assert_eq!(next_delay(&Err(Failure::Idle { hint: "claude".into() }), 9), INTERVAL);
    }

    #[test]
    fn renames_keep_the_schedule_and_changes_reset_it() {
        let mut slots = Vec::new();
        let a = UsageProfile::new("A", Provider::Claude, "");
        set_profiles(&mut slots, vec![a.clone()]);
        let later = Instant::now() + INTERVAL;
        slots[0].due = later;

        let renamed = UsageProfile { name: "Work".into(), ..a.clone() };
        set_profiles(&mut slots, vec![renamed]);
        assert_eq!(slots[0].due, later);
        assert_eq!(slots[0].profile.name, "Work");

        let moved = UsageProfile { instructions: "~/.claude-work".into(), ..a };
        set_profiles(&mut slots, vec![moved]);
        assert!(slots[0].due < later);

        set_profiles(&mut slots, Vec::new());
        assert!(slots.is_empty());
    }

    /// Answers instantly and records who was asked.
    struct Fake(Arc<Mutex<Vec<ProfileId>>>);

    impl Poller for Fake {
        fn poll(&self, profile: &UsageProfile, _: &mut ProviderState) -> Result<Report, Failure> {
            self.0.lock().unwrap().push(profile.id);
            Ok(Report::new(Vec::new(), SystemTime::now(), true))
        }
    }

    #[test]
    fn polls_new_profiles_and_honours_refresh_gaps() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let monitor = Monitor::spawn(Fake(asked.clone()), move |u: Update| {
            let _ = tx.send(u.profile);
        });
        let a = UsageProfile::new("A", Provider::Codex, "");
        let b = UsageProfile::new("B", Provider::Claude, "");
        monitor.set_profiles(vec![a.clone(), b.clone()]);
        let mut got =
            vec![rx.recv_timeout(Duration::from_secs(5)).unwrap(), rx.recv_timeout(Duration::from_secs(5)).unwrap()];
        got.sort();
        let mut want = vec![a.id, b.id];
        want.sort();
        assert_eq!(got, want);

        // Just polled, so a refresh is ignored rather than hitting the provider again.
        monitor.refresh(None);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        assert_eq!(asked.lock().unwrap().len(), 2);

        drop(monitor);
    }
}
