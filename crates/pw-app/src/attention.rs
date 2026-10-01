//! What a terminal wants from you: it finished, or it needs your input (see ADR 0012).
//!
//! A terminal raises attention only while you aren't watching it, and loses it when it gets the
//! keys. The drawer, the rail, pane title bars and the window title all read the same
//! [`Status`], so they never disagree.

use pw_model::PaneId;

/// How urgent: needing you outranks having finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Finished,
    NeedsInput,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Finished => "Finished",
            Kind::NeedsInput => "Needs input",
        }
    }
}

/// Where the news came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The program said so: a notification or the bell.
    Signal,
    /// It printed steadily, then went quiet (see `pw_term::ActivityConfig`).
    Quiet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attention {
    pub kind: Kind,
    pub source: Source,
    /// What the program said, if it said anything.
    pub message: Option<String>,
    /// When it was raised, app-wide, so the oldest is answered first.
    pub seq: u64,
}

/// Reads a notification: anything asking for permission, approval, an answer or input needs you;
/// anything else (a turn complete) is news that it finished.
pub fn classify(title: Option<&str>, body: &str) -> Kind {
    const ASKS: [&str; 9] =
        ["permission", "approv", "input", "waiting", "question", "confirm", "allow", "needs your", "respond"];
    let text = format!("{} {body}", title.unwrap_or_default()).to_lowercase();
    if ASKS.iter().any(|word| text.contains(word)) { Kind::NeedsInput } else { Kind::Finished }
}

/// Puts `new` on a terminal that may already want something. A higher kind replaces a lower one;
/// at the same kind the newer message wins but the place in line is kept. Returns whether the
/// kind went up, which is when it's worth notifying.
pub fn raise(current: &mut Option<Attention>, new: Attention) -> bool {
    match current {
        Some(old) if old.kind > new.kind => false,
        Some(old) if old.kind == new.kind => {
            if new.message.is_some() {
                old.message = new.message;
            }
            if new.source == Source::Signal {
                old.source = Source::Signal;
            }
            false
        }
        _ => {
            *current = Some(new);
            true
        }
    }
}

/// One terminal, as [`Status::of`] sees it.
pub struct PaneState<'a> {
    pub pane: PaneId,
    pub working: bool,
    pub attention: Option<&'a Attention>,
}

/// What a group of terminals (a workspace, or its agent panes) wants, worst first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// The most urgent kind among them.
    pub kind: Option<Kind>,
    /// How many are at that kind.
    pub count: usize,
    /// The oldest terminal at that kind: where a jump goes.
    pub first: Option<(u64, PaneId)>,
    /// Its message.
    pub message: Option<String>,
    /// Any of them is busy.
    pub working: bool,
}

impl Status {
    pub fn of<'a>(panes: impl IntoIterator<Item = PaneState<'a>>) -> Self {
        let mut status = Status::default();
        for p in panes {
            status.working |= p.working;
            let Some(a) = p.attention else { continue };
            if status.kind.is_some_and(|k| k > a.kind) {
                continue;
            }
            if status.kind != Some(a.kind) {
                status.kind = Some(a.kind);
                status.count = 0;
                status.first = None;
            }
            status.count += 1;
            if status.first.is_none_or(|(seq, _)| a.seq < seq) {
                status.first = Some((a.seq, p.pane));
                status.message = a.message.clone();
            }
        }
        status
    }

    /// "Needs input", "2 need input", "Finished", "3 finished".
    pub fn summary(&self) -> Option<String> {
        let kind = self.kind?;
        Some(match (kind, self.count) {
            (_, 0 | 1) => kind.label().to_owned(),
            (Kind::NeedsInput, n) => format!("{n} need input"),
            (Kind::Finished, n) => format!("{n} finished"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(kind: Kind, seq: u64) -> Attention {
        Attention { kind, source: Source::Signal, message: None, seq }
    }

    #[test]
    fn agent_messages_are_classified() {
        assert_eq!(classify(None, "Claude needs your permission to use Bash"), Kind::NeedsInput);
        assert_eq!(classify(None, "Claude is waiting for your input"), Kind::NeedsInput);
        assert_eq!(classify(Some("Codex"), "Approval requested: rm -rf target"), Kind::NeedsInput);
        assert_eq!(classify(Some("Codex"), "Agent turn complete"), Kind::Finished);
        assert_eq!(classify(None, "Build finished"), Kind::Finished);
    }

    #[test]
    fn needing_input_outranks_finishing() {
        let mut current = None;
        assert!(raise(&mut current, att(Kind::Finished, 1)));
        assert!(raise(&mut current, att(Kind::NeedsInput, 2)));
        assert!(!raise(&mut current, att(Kind::Finished, 3)));
        assert_eq!(current.as_ref().map(|a| (a.kind, a.seq)), Some((Kind::NeedsInput, 2)));
    }

    #[test]
    fn a_repeat_keeps_its_place_and_takes_the_message() {
        let mut current = Some(Attention { source: Source::Quiet, ..att(Kind::Finished, 1) });
        let news = Attention { message: Some("Agent turn complete".into()), ..att(Kind::Finished, 5) };
        assert!(!raise(&mut current, news));
        let current = current.unwrap();
        assert_eq!((current.seq, current.source), (1, Source::Signal));
        assert_eq!(current.message.as_deref(), Some("Agent turn complete"));
    }

    #[test]
    fn status_picks_the_oldest_of_the_worst() {
        let (a, b, c) = (PaneId::new(), PaneId::new(), PaneId::new());
        let finished = att(Kind::Finished, 1);
        let late = Attention { message: Some("late".into()), ..att(Kind::NeedsInput, 9) };
        let early = Attention { message: Some("early".into()), ..att(Kind::NeedsInput, 4) };
        let status = Status::of([
            PaneState { pane: a, working: false, attention: Some(&finished) },
            PaneState { pane: b, working: true, attention: Some(&late) },
            PaneState { pane: c, working: false, attention: Some(&early) },
        ]);
        assert_eq!(status.kind, Some(Kind::NeedsInput));
        assert_eq!(status.first, Some((4, c)));
        assert_eq!(status.message.as_deref(), Some("early"));
        assert_eq!(status.summary().as_deref(), Some("2 need input"));
        assert!(status.working);
    }

    #[test]
    fn quiet_panes_have_no_status() {
        let status = Status::of([PaneState { pane: PaneId::new(), working: false, attention: None }]);
        assert_eq!(status, Status::default());
        assert_eq!(status.summary(), None);
    }
}
