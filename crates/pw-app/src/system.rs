//! The system profile's latest reading and a few minutes of history.
//!
//! Sampling happens on `pw_system`'s monitor thread, and only while the profile is on screen
//! (the drawer's section is open, or the rail shows it). Readings are never persisted.

use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use iced::Subscription;
use pw_system::{Monitor, Sample, Sampler};

use crate::inbox::{self, Inbox};

/// How far back the hover cards' sparklines reach.
pub const HISTORY: Duration = Duration::from_secs(5 * 60);

/// One reading, reduced to what the sparklines draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub at: SystemTime,
    /// Percent of the machine.
    pub cpu: f32,
    /// Percent busy of the busiest GPU.
    pub gpu: Option<f32>,
    /// Percent of memory in use.
    pub memory: f32,
    /// Bytes per second, both directions.
    pub network: u64,
}

impl Point {
    fn of(sample: &Sample) -> Self {
        Self {
            at: sample.at,
            cpu: sample.cpu.usage,
            gpu: sample.gpu().and_then(|g| g.usage),
            memory: sample.memory.percent(),
            network: sample.network.rx + sample.network.tx,
        }
    }
}

pub struct SystemProfile {
    latest: Option<Sample>,
    history: VecDeque<Point>,
    monitor: Monitor,
    inbox: Inbox<Sample>,
    /// What the monitor was last told, so it's only told about changes.
    roots: Vec<u32>,
    active: bool,
}

impl SystemProfile {
    /// Starts the monitor, paused; [`SystemProfile::set_active`] starts it.
    pub fn new() -> Self {
        let (tx, inbox) = inbox::channel("pw-system-samples");
        let monitor = Monitor::spawn(Sampler::new, move |sample| {
            let _ = tx.unbounded_send(sample);
        });
        Self { latest: None, history: VecDeque::new(), monitor, inbox, roots: Vec::new(), active: false }
    }

    pub fn events(&self) -> Subscription<Sample> {
        self.inbox.subscription()
    }

    pub fn latest(&self) -> Option<&Sample> {
        self.latest.as_ref()
    }

    /// Oldest first.
    pub fn history(&self) -> &VecDeque<Point> {
        &self.history
    }

    pub fn apply(&mut self, sample: Sample) {
        // A reading that was on its way when sampling paused.
        if !self.active {
            return;
        }
        self.history.push_back(Point::of(&sample));
        let oldest = sample.at.checked_sub(HISTORY).unwrap_or(SystemTime::UNIX_EPOCH);
        while self.history.front().is_some_and(|p| p.at < oldest) {
            self.history.pop_front();
        }
        self.latest = Some(sample);
    }

    /// Samples while the profile is on screen. Pausing keeps nothing: a reading from before the
    /// pause would be shown as current when it reopens.
    pub fn set_active(&mut self, active: bool) {
        if active == self.active {
            return;
        }
        self.active = active;
        if !active {
            self.latest = None;
        }
        self.monitor.set_active(active);
    }

    /// The terminals' shells, whose process trees are totalled per workspace.
    pub fn track(&mut self, mut roots: Vec<u32>) {
        roots.sort_unstable();
        if roots != self.roots {
            self.roots = roots.clone();
            self.monitor.set_roots(roots);
        }
    }
}
