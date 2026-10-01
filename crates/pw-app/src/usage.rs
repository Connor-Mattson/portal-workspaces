//! Tracked AI-usage profiles and their latest readings.
//!
//! The polling itself happens on `pw_usage`'s monitor thread; this registry keeps what the
//! drawer shows and forwards profile changes to the monitor. Readings are not persisted: they're
//! back within seconds of launch, and saving them would rewrite the state file every 2 minutes.

use std::collections::HashMap;
use std::time::SystemTime;

use iced::Subscription;
use pw_model::{ProfileId, UsageProfile};
use pw_usage::{Failure, MIN_REFRESH_GAP, Monitor, Providers, Report, Update};

use crate::inbox::{self, Inbox};

#[derive(Debug, Default)]
pub struct ProfileStatus {
    /// The latest numbers, kept through later failures so the drawer can show them dimmed.
    pub last: Option<Report>,
    /// Why the most recent poll had no numbers.
    pub failure: Option<Failure>,
    /// When the last poll finished.
    pub polled_at: Option<SystemTime>,
    pub next_poll: Option<SystemTime>,
    /// A poll was asked for and hasn't answered yet.
    pub checking: bool,
}

pub struct Usage {
    profiles: Vec<UsageProfile>,
    statuses: HashMap<ProfileId, ProfileStatus>,
    monitor: Monitor,
    inbox: Inbox<Update>,
}

impl Usage {
    pub fn new(profiles: Vec<UsageProfile>) -> Self {
        let (tx, inbox) = inbox::channel("pw-usage-updates");
        let monitor = Monitor::spawn(Providers::new(crate::sessions::home_dir()), move |update| {
            let _ = tx.unbounded_send(update);
        });
        monitor.set_profiles(profiles.clone());
        let statuses =
            profiles.iter().map(|p| (p.id, ProfileStatus { checking: true, ..Default::default() })).collect();
        Self { profiles, statuses, monitor, inbox }
    }

    pub fn events(&self) -> Subscription<Update> {
        self.inbox.subscription()
    }

    pub fn profiles(&self) -> &[UsageProfile] {
        &self.profiles
    }

    pub fn profile(&self, id: ProfileId) -> Option<&UsageProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn status(&self, id: ProfileId) -> Option<&ProfileStatus> {
        self.statuses.get(&id)
    }

    pub fn apply(&mut self, update: Update) {
        // A reading for a profile deleted while it was being polled.
        let Some(status) = self.statuses.get_mut(&update.profile) else { return };
        match update.result {
            Ok(report) => {
                status.last = Some(report);
                status.failure = None;
            }
            Err(failure) => status.failure = Some(failure),
        }
        status.polled_at = Some(SystemTime::now());
        status.next_poll = Some(update.next_poll);
        status.checking = false;
    }

    /// Adds a profile, or replaces the one with the same id. A profile pointed somewhere new
    /// forgets its old numbers.
    pub fn upsert(&mut self, profile: UsageProfile) {
        let moved = match self.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => {
                let moved = existing.provider != profile.provider || existing.instructions != profile.instructions;
                *existing = profile.clone();
                moved
            }
            None => {
                self.profiles.push(profile.clone());
                true
            }
        };
        if moved {
            self.statuses.insert(profile.id, ProfileStatus { checking: true, ..Default::default() });
        }
        self.monitor.set_profiles(self.profiles.clone());
    }

    pub fn remove(&mut self, id: ProfileId) {
        self.profiles.retain(|p| p.id != id);
        self.statuses.remove(&id);
        self.monitor.set_profiles(self.profiles.clone());
    }

    /// Polls every profile now. Ones polled moments ago are skipped (as the monitor does).
    pub fn refresh(&mut self) {
        let now = SystemTime::now();
        for status in self.statuses.values_mut() {
            let recent = status.polled_at.and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d < MIN_REFRESH_GAP);
            status.checking |= !recent;
        }
        self.monitor.refresh(None);
    }
}
