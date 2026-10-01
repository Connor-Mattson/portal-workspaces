//! What a poll produces: rate-limit windows, or why there are none.

use std::fmt;
use std::time::{Duration, SystemTime};

/// How long a limit window lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Span {
    FiveHour,
    Weekly,
    /// Any other length, in minutes.
    Other(u32),
}

impl Span {
    pub fn from_minutes(minutes: u32) -> Self {
        match minutes {
            300 => Span::FiveHour,
            10_080 => Span::Weekly,
            m => Span::Other(m),
        }
    }

    pub fn duration(self) -> Duration {
        let minutes = match self {
            Span::FiveHour => 300,
            Span::Weekly => 10_080,
            Span::Other(m) => m,
        };
        Duration::from_secs(u64::from(minutes) * 60)
    }

    pub fn label(self) -> String {
        match self {
            Span::FiveHour => "5h".into(),
            Span::Weekly => "Week".into(),
            Span::Other(m) if m % 1440 == 0 => format!("{}d", m / 1440),
            Span::Other(m) if m % 60 == 0 => format!("{}h", m / 60),
            Span::Other(m) => format!("{m}m"),
        }
    }

    pub fn long_label(self) -> String {
        match self {
            Span::FiveHour => "5-hour".into(),
            Span::Weekly => "Weekly".into(),
            other => other.label(),
        }
    }
}

/// One rate-limit window, e.g. "5h: 42% used, resets at 16:10".
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub span: Span,
    /// Which models the window covers when it isn't all of them ("Opus", "Gemini", …).
    pub scope: Option<String>,
    /// Percent used, 0–100.
    pub used: f32,
    pub resets_at: Option<SystemTime>,
}

impl Window {
    pub fn new(span: Span, scope: Option<String>, used: f32, resets_at: Option<SystemTime>) -> Self {
        Self { span, scope, used: if used.is_finite() { used.clamp(0.0, 100.0) } else { 0.0 }, resets_at }
    }

    /// Percent used at `now`: a window whose reset time has passed is empty again, even if the
    /// reading predates the reset.
    pub fn used_at(&self, now: SystemTime) -> f32 {
        match self.resets_at {
            Some(reset) if reset <= now => 0.0,
            _ => self.used,
        }
    }

    /// How far through the window `now` is, 0–1 (the "pace" mark). `None` without a reset time
    /// or once the window has reset (a new one hasn't started until the account is used).
    pub fn elapsed_at(&self, now: SystemTime) -> Option<f32> {
        let left = self.resets_at?.duration_since(now).ok()?.as_secs_f32();
        let total = self.span.duration().as_secs_f32();
        Some((1.0 - left / total).clamp(0.0, 1.0))
    }

    /// Time until the window resets, if it hasn't.
    pub fn resets_in(&self, now: SystemTime) -> Option<Duration> {
        self.resets_at?.duration_since(now).ok()
    }

    pub fn title(&self) -> String {
        match &self.scope {
            Some(scope) => format!("{} · {scope}", self.span.long_label()),
            None => self.span.long_label(),
        }
    }
}

/// A successful poll.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Grouped by scope (account-wide first), shorter spans first within a group.
    pub windows: Vec<Window>,
    /// Who the account is (an email), when the provider says.
    pub account: Option<String>,
    /// Subscription plan ("Max", "Pro", …).
    pub plan: Option<String>,
    /// When the numbers were true. For live sources this is the poll time; for passive ones
    /// (Codex logs) it's when the CLI last recorded them.
    pub as_of: SystemTime,
    /// Whether the numbers come straight from the provider (vs. the CLI's last record).
    pub live: bool,
}

impl Report {
    pub fn new(mut windows: Vec<Window>, as_of: SystemTime, live: bool) -> Self {
        windows.sort_by(|a, b| (&a.scope, a.span).cmp(&(&b.scope, b.span)));
        Self { windows, account: None, plan: None, as_of, live }
    }

    /// The headline window of a span: the account-wide one, or when every window is scoped (one
    /// per model family), the fullest.
    pub fn window(&self, span: Span) -> Option<&Window> {
        let of_span = || self.windows.iter().filter(move |w| w.span == span);
        of_span().find(|w| w.scope.is_none()).or_else(|| of_span().max_by(|a, b| a.used.total_cmp(&b.used)))
    }
}

/// Why a poll produced no numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The sign-in expired and the CLI didn't renew it: nobody has used this account lately.
    /// `hint` is what to run to wake it up.
    Idle { hint: String },
    /// The provider rejected the credentials.
    SignedOut(String),
    /// No such install or no credentials where the profile points.
    NotFound(String),
    /// Network trouble, a server error or a response we couldn't read.
    Unavailable(String),
    /// Asked too often; try again after the given delay.
    RateLimited { retry_after: Duration },
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Idle { hint } => write!(f, "Idle: sign-in expired. Run {hint} to refresh."),
            Failure::SignedOut(msg) | Failure::NotFound(msg) | Failure::Unavailable(msg) => f.write_str(msg),
            Failure::RateLimited { retry_after } => {
                write!(f, "The provider asked to slow down; retrying in {} min.", retry_after.as_secs().div_ceil(60))
            }
        }
    }
}

/// Parses an RFC 3339 timestamp ("2026-02-20T14:00:00.364238+00:00").
pub(crate) fn parse_time(text: &str) -> Option<SystemTime> {
    text.parse::<jiff::Timestamp>().ok().map(SystemTime::from)
}

pub(crate) fn from_unix(secs: f64) -> Option<SystemTime> {
    (secs.is_finite() && secs > 0.0).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn used_drops_to_zero_after_reset() {
        let w = Window::new(Span::FiveHour, None, 42.0, Some(at(10_000)));
        assert_eq!(w.used_at(at(9_999)), 42.0);
        assert_eq!(w.used_at(at(10_000)), 0.0);
        assert_eq!(Window::new(Span::Weekly, None, 7.0, None).used_at(at(1)), 7.0);
    }

    #[test]
    fn elapsed_is_the_fraction_of_the_window_gone() {
        let w = Window::new(Span::FiveHour, None, 0.0, Some(at(18_000)));
        assert_eq!(w.elapsed_at(at(0)), Some(0.0));
        assert_eq!(w.elapsed_at(at(9_000)), Some(0.5));
        assert_eq!(w.elapsed_at(at(20_000)), None);
    }

    #[test]
    fn used_is_clamped() {
        assert_eq!(Window::new(Span::Weekly, None, 140.0, None).used, 100.0);
        assert_eq!(Window::new(Span::Weekly, None, f32::NAN, None).used, 0.0);
    }

    #[test]
    fn windows_sort_headline_first() {
        let r = Report::new(
            vec![
                Window::new(Span::Weekly, Some("Opus".into()), 1.0, None),
                Window::new(Span::Weekly, None, 2.0, None),
                Window::new(Span::FiveHour, None, 3.0, None),
            ],
            at(0),
            true,
        );
        let order: Vec<_> = r.windows.iter().map(|w| (w.span, w.scope.clone())).collect();
        assert_eq!(order, vec![(Span::FiveHour, None), (Span::Weekly, None), (Span::Weekly, Some("Opus".into()))]);
        assert_eq!(r.window(Span::Weekly).unwrap().used, 2.0);
    }

    #[test]
    fn span_labels() {
        assert_eq!(Span::from_minutes(300), Span::FiveHour);
        assert_eq!(Span::from_minutes(10_080).label(), "Week");
        assert_eq!(Span::from_minutes(1440).label(), "1d");
        assert_eq!(Span::from_minutes(120).label(), "2h");
    }

    #[test]
    fn parses_rfc3339() {
        assert_eq!(parse_time("1970-01-01T00:01:40+00:00"), Some(at(100)));
        assert_eq!(parse_time("1970-01-01T00:01:40.5Z").map(|t| t > at(100)), Some(true));
        assert_eq!(parse_time("soon"), None);
    }
}
