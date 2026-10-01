//! Usage monitor for Portal Workspaces: how much of each AI subscription's 5-hour and weekly
//! limits is used, polled in the background.
//!
//! **Polling never spends tokens.** The only network calls are read-only usage and quota queries
//! (see [`http::Endpoint`]), and the only programs run are provider CLIs renewing their own sign-in
//! (see [`vendor::VendorCmd`]). Neither list has a way to run a model, and neither takes free-form
//! input.
//!
//! Like `pw-term`, this crate has no GUI types: the app gets plain [`Update`]s through a callback.

mod antigravity;
mod claude;
mod codex;
mod discover;
pub mod http;
mod monitor;
mod report;
mod secrets;
pub mod vendor;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pw_model::{Provider, UsageProfile};

pub use discover::{Candidate, describe, discover};
pub use monitor::{INTERVAL, MIN_REFRESH_GAP, Monitor, Update};
pub use report::{Failure, Report, Span, Window};

/// Polls one profile. The real implementation is [`Providers`]; tests substitute their own.
pub trait Poller: Send + Sync + 'static {
    fn poll(&self, profile: &UsageProfile, state: &mut ProviderState) -> Result<Report, Failure>;
}

/// What a profile's poller remembers between polls.
#[derive(Debug, Default)]
pub struct ProviderState {
    last_renewal: Option<Instant>,
    /// The Google Cloud project serving an Antigravity account (asked for once).
    project: Option<String>,
}

/// A CLI is asked to renew its sign-in at most this often per profile.
const RENEWAL_GAP: Duration = Duration::from_secs(15 * 60);

impl ProviderState {
    /// Whether a renewal may run now; if so, it counts as started.
    pub(crate) fn may_renew(&mut self) -> bool {
        let now = Instant::now();
        if self.last_renewal.is_some_and(|t| now.duration_since(t) < RENEWAL_GAP) {
            return false;
        }
        self.last_renewal = Some(now);
        true
    }

    /// Lifts the throttle, so the next poll may renew right away.
    pub(crate) fn allow_renewal(&mut self) {
        self.last_renewal = None;
    }
}

/// The real providers: Claude, Codex and Antigravity.
pub struct Providers {
    home: PathBuf,
    http: http::Http,
}

impl Providers {
    /// `home` is the user's home directory (where `~` points in profile instructions).
    pub fn new(home: PathBuf) -> Self {
        Self { home, http: http::Http::default() }
    }
}

impl Poller for Providers {
    fn poll(&self, profile: &UsageProfile, state: &mut ProviderState) -> Result<Report, Failure> {
        let env = profile.env(&self.home).map_err(Failure::NotFound)?;
        match profile.provider {
            Provider::Claude => claude::poll(&env, state, &self.http),
            Provider::Codex => codex::poll(&env, state, &self.http),
            Provider::Antigravity => antigravity::poll(&env, state, &self.http),
        }
    }
}

/// `~/…` for paths under the home directory, for messages.
pub(crate) fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// "max" → "Max", "plus pro" → "Plus Pro".
pub(crate) fn title_case(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewals_are_throttled() {
        let mut state = ProviderState::default();
        assert!(state.may_renew());
        assert!(!state.may_renew());
        state.allow_renewal();
        assert!(state.may_renew());
    }

    #[test]
    fn titles() {
        assert_eq!(title_case("max"), "Max");
        assert_eq!(title_case("oauth apps"), "Oauth Apps");
    }
}
