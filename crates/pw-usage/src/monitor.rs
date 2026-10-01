//! The usage daemon: one thread that polls every profile on a schedule.
//!
//! The thread sleeps until the next profile is due (or a command arrives), starts each due poll on
//! its own thread, and hands each result to a callback. A slow or stuck poll holds up only its own
//! profile: the others keep their schedule. Dropping the [`Monitor`] stops it.

use std::collections::HashMap;
use std::sync::Arc;
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
    Renew(ProfileId),
    /// A poll thread finished; `poll` says which one, so a slot replaced meanwhile ignores it.
    Polled {
        profile: ProfileId,
        poll: u64,
        state: ProviderState,
        result: Result<Report, Failure>,
    },
    Stop,
}

pub struct Monitor {
    tx: mpsc::Sender<Command>,
}

impl Monitor {
    /// Starts the daemon. `on_update` runs on the monitor thread.
    pub fn spawn(poller: impl Poller, on_update: impl Fn(Update) + Send + Sync + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let polled = tx.clone();
        thread::Builder::new()
            .name("usage-monitor".into())
            .spawn(move || run(Arc::new(poller), &rx, &polled, &on_update))
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

    /// Polls a profile now and lets its CLI renew an expired sign-in even if it tried moments ago
    /// (a click on an idle card, not the schedule).
    pub fn renew(&self, profile: ProfileId) {
        let _ = self.tx.send(Command::Renew(profile));
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // Poll threads hold senders too, so the channel alone wouldn't disconnect.
        let _ = self.tx.send(Command::Stop);
    }
}

struct Slot {
    profile: UsageProfile,
    /// `None` while a poll has it.
    state: Option<ProviderState>,
    due: Instant,
    last_poll: Option<Instant>,
    failures: u32,
    /// The poll in flight, if any.
    polling: Option<u64>,
    /// A renewal asked for while a poll was in flight, applied when it returns.
    renew: bool,
}

impl Slot {
    fn new(profile: UsageProfile, due: Instant) -> Self {
        Self {
            profile,
            state: Some(ProviderState::default()),
            due,
            last_poll: None,
            failures: 0,
            polling: None,
            renew: false,
        }
    }
}

fn run<P: Poller>(
    poller: Arc<P>,
    rx: &mpsc::Receiver<Command>,
    polled: &mpsc::Sender<Command>,
    on_update: &impl Fn(Update),
) {
    let mut slots: Vec<Slot> = Vec::new();
    let mut polls = 0u64;
    loop {
        let next_due = slots.iter().filter(|s| s.polling.is_none()).map(|s| s.due).min();
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
            Ok(Command::Renew(target)) => {
                if let Some(slot) = slots.iter_mut().find(|s| s.profile.id == target) {
                    match slot.state.as_mut() {
                        Some(state) => state.allow_renewal(),
                        None => slot.renew = true,
                    }
                    slot.due = Instant::now();
                }
            }
            Ok(Command::Polled { profile, poll, mut state, result }) => {
                let Some(slot) = slots.iter_mut().find(|s| s.profile.id == profile && s.polling == Some(poll)) else {
                    continue; // Removed or reconfigured while it ran.
                };
                let finished = Instant::now();
                slot.failures = if result.is_ok() { 0 } else { slot.failures + 1 };
                let delay = next_delay(&result, slot.failures);
                slot.due = finished + delay;
                slot.last_poll = Some(finished);
                if std::mem::take(&mut slot.renew) {
                    state.allow_renewal();
                    slot.due = finished;
                }
                slot.state = Some(state);
                slot.polling = None;
                on_update(Update { profile, result, next_poll: SystemTime::now() + delay });
            }
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        // Profiles are independent (different hosts, files and CLIs), so each poll gets its own
        // thread and one slow one never holds up the rest.
        let now = Instant::now();
        for slot in slots.iter_mut().filter(|s| s.polling.is_none() && s.due <= now) {
            let Some(mut state) = slot.state.take() else { continue };
            polls += 1;
            let poll = polls;
            slot.polling = Some(poll);
            let (poller, polled, profile) = (poller.clone(), polled.clone(), slot.profile.clone());
            let spawned = thread::Builder::new().name("usage-poll".into()).spawn(move || {
                let result = poller.poll(&profile, &mut state);
                let _ = polled.send(Command::Polled { profile: profile.id, poll, state, result });
            });
            if let Err(err) = spawned {
                tracing::warn!(%err, "couldn't start a usage poll");
                slot.polling = None;
                slot.state = Some(ProviderState::default());
                slot.due = now + INTERVAL;
            }
        }
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
            _ => slots.push(Slot::new(profile, now)),
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

        // A renewal is the user asking for one profile in particular, so it goes through.
        monitor.renew(a.id);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), a.id);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());

        drop(monitor);
    }

    /// Hangs on one profile until released; answers the others at once.
    struct Stuck {
        stuck: ProfileId,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl Poller for Stuck {
        fn poll(&self, profile: &UsageProfile, _: &mut ProviderState) -> Result<Report, Failure> {
            if profile.id == self.stuck {
                let _ = self.release.lock().unwrap().recv();
            }
            Ok(Report::new(Vec::new(), SystemTime::now(), true))
        }
    }

    #[test]
    fn a_stuck_profile_does_not_hold_up_the_others() {
        let a = UsageProfile::new("A", Provider::Codex, "");
        let b = UsageProfile::new("B", Provider::Claude, "");
        let (release, gate) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let monitor = Monitor::spawn(Stuck { stuck: a.id, release: Mutex::new(gate) }, move |u: Update| {
            let _ = tx.send(u.profile);
        });
        monitor.set_profiles(vec![a.clone(), b.clone()]);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), b.id);
        // Before polls had their own threads, nothing ran again until A returned.
        monitor.renew(b.id);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), b.id);
        // A renewal asked for while A is polling waits for that poll, then runs.
        monitor.renew(a.id);
        release.send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), a.id);
        release.send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), a.id);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        drop(monitor);
    }
}
