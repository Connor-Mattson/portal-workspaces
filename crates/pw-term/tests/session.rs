//! End-to-end checks against real PTYs and shells.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pw_term::{ActivityConfig, GridSize, Session, SessionConfig, TermEvent};

const SIZE: GridSize = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };

fn spawn(cwd: &Path, program: &str, args: &[&str]) -> (Session, mpsc::Receiver<TermEvent>) {
    spawn_with(cwd, program, args, ActivityConfig::default())
}

fn spawn_with(
    cwd: &Path,
    program: &str,
    args: &[&str],
    activity: ActivityConfig,
) -> (Session, mpsc::Receiver<TermEvent>) {
    let (tx, rx) = mpsc::channel();
    let mut config = SessionConfig::new(cwd.to_owned(), SIZE);
    config.activity = activity;
    config.shell = Some((program.to_owned(), args.iter().map(|s| s.to_string()).collect()));
    let session = Session::spawn(config, move |event| {
        let _ = tx.send(event);
    })
    .expect("spawn");
    (session, rx)
}

/// Polls `check` until it holds, taking a fresh snapshot after every wakeup.
fn wait_for(session: &Session, rx: &mpsc::Receiver<TermEvent>, what: &str, check: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = session.snapshot(true).text();
        if check(&text) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}; screen:\n{text}");
        let _ = rx.recv_timeout(Duration::from_millis(100));
    }
}

#[test]
fn output_reaches_the_grid() {
    let dir = tempfile::tempdir().unwrap();
    let (session, rx) = spawn(dir.path(), "/bin/sh", &["-c", "printf 'hello from pty'; sleep 5"]);
    wait_for(&session, &rx, "output", |t| t.contains("hello from pty"));
}

#[test]
fn wakeups_are_coalesced_until_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (session, rx) =
        spawn(dir.path(), "/bin/sh", &["-c", "for i in 1 2 3 4 5 6 7 8 9 10; do echo line$i; done; sleep 5"]);
    wait_for(&session, &rx, "all lines", |t| t.contains("line10"));
    // Everything was printed; without a new snapshot no further wakeups may be queued.
    while rx.try_recv().is_ok() {}
    std::thread::sleep(Duration::from_millis(200));
    assert!(rx.try_recv().is_err());
}

#[test]
fn exit_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, rx) = spawn(dir.path(), "/bin/sh", &["-c", "exit 3"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(TermEvent::Exited(code)) = rx.recv_timeout(Duration::from_millis(200)) {
            assert_eq!(code, Some(3));
            return;
        }
    }
    panic!("no exit event");
}

#[test]
fn typing_reaches_the_shell_and_cwd_is_tracked() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let (session, rx) = spawn(dir.path(), "/bin/sh", &[]);
    let start = std::fs::canonicalize(dir.path()).unwrap();
    assert_eq!(session.current_dir().map(|p| std::fs::canonicalize(p).unwrap()), Some(start.clone()));

    session.write(&b"cd sub && echo moved\r"[..]);
    wait_for(&session, &rx, "cd", |t| t.contains("moved"));
    assert_eq!(session.current_dir().map(|p| std::fs::canonicalize(p).unwrap()), Some(start.join("sub")));
}

#[test]
fn resize_reaches_the_pty() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, rx) = spawn(dir.path(), "/bin/sh", &[]);
    session.resize(GridSize { cols: 100, rows: 30, ..SIZE });
    session.write(&b"stty size\r"[..]);
    wait_for(&session, &rx, "stty size", |t| t.contains("30 100"));
    assert_eq!(session.snapshot(true).cols, 100);
}

#[test]
fn deferred_resizes_reach_the_pty_once_settled() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, rx) = spawn(dir.path(), "/bin/sh", &[]);
    session.write(&b"n=0; trap 'n=$((n+1))' WINCH; echo ready\r"[..]);
    wait_for(&session, &rx, "trap", |t| t.contains("ready"));
    // A drag: many sizes, none of them taken yet.
    for cols in 81..=120 {
        session.defer_resize(GridSize { cols, rows: 30, ..SIZE });
    }
    assert_eq!(session.size(), SIZE);
    assert_eq!(session.target_size(), GridSize { cols: 120, rows: 30, ..SIZE });
    assert!(session.settle());
    assert!(!session.settle());
    assert_eq!(session.size().cols, 120);
    session.write(&b"echo winches=$n; stty size\r"[..]);
    wait_for(&session, &rx, "stty size", |t| t.contains("30 120"));
    // Forty steps of the drag, one resize for the program.
    assert!(session.snapshot(true).text().contains("winches=1"));

    // Dragged back to where it started: nothing to settle.
    session.defer_resize(GridSize { cols: 90, ..session.size() });
    session.defer_resize(session.size());
    assert!(!session.settle());
    // A resize now drops one that was deferred.
    session.defer_resize(GridSize { cols: 90, ..session.size() });
    session.resize(GridSize { cols: 100, ..session.size() });
    assert!(!session.settle());
    assert_eq!(session.size().cols, 100);
}

#[test]
fn select_all_copies_the_history_and_screen() {
    let dir = tempfile::tempdir().unwrap();
    let (session, rx) = spawn(dir.path(), "/bin/sh", &["-c", "printf 'alpha\\nbeta'; sleep 5"]);
    wait_for(&session, &rx, "output", |t| t.contains("beta"));
    session.select_all();
    // Ends at the last line with text, not the screen's empty rows.
    assert_eq!(session.take_visible_selection().as_deref(), Some("alpha\nbeta"));
    // Taking it dropped it, so the next Ctrl+C interrupts.
    assert_eq!(session.take_visible_selection(), None);
}

#[test]
fn only_a_selection_on_screen_is_taken() {
    let dir = tempfile::tempdir().unwrap();
    let (session, rx) = spawn(dir.path(), "/bin/sh", &["-c", "seq 1 100; sleep 5"]);
    wait_for(&session, &rx, "output", |t| t.contains("100"));
    // 78…100, then the cursor's empty line.
    let first = pw_term::GridPoint { row: 0, col: 0 };
    session.start_selection(pw_term::SelectionKind::Line, first, false);
    // A page up moves the selected line below the screen.
    session.scroll_page(true);
    assert_eq!(session.take_visible_selection(), None);
    session.scroll_page(false);
    assert_eq!(session.take_visible_selection().as_deref(), Some("78\n"));
}

fn char_key(c: char) -> pw_term::KeyInput {
    pw_term::KeyInput { key: pw_term::Key::Char(c), mods: pw_term::Mods::default(), text: Some(c.to_string()) }
}

#[test]
fn typing_redraws_at_once_only_when_it_moves_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let (session, rx) = spawn(dir.path(), "/bin/sh", &["-c", "seq 1 100; cat >/dev/null"]);
    wait_for(&session, &rx, "output", |t| t.contains("100"));

    // At the live output with nothing selected, nothing changes until the echo arrives, so the
    // keystroke itself mustn't cost a redraw.
    assert!(!session.send_key(&char_key('a')));
    assert!(!session.paste("pasted"));

    // Scrolled back: typing jumps to the bottom at once.
    session.scroll_page(true);
    assert!(session.snapshot(true).display_offset > 0);
    assert!(session.send_key(&char_key('b')));
    assert_eq!(session.snapshot(true).display_offset, 0);
    session.scroll_page(true);
    assert!(session.paste("x"));
    assert_eq!(session.snapshot(true).display_offset, 0);

    // A selection is dropped at once.
    session.start_selection(pw_term::SelectionKind::Line, pw_term::GridPoint { row: 0, col: 0 }, false);
    assert!(session.send_key(&char_key('c')));
    assert_eq!(session.selection_text(), None);
    session.select_all();
    assert!(session.paste("y"));
    assert_eq!(session.selection_text(), None);
}

/// Collects events (other than wakeups) for up to `duration`, or until `last` arrives.
fn events_until(
    rx: &mpsc::Receiver<TermEvent>,
    duration: Duration,
    last: impl Fn(&TermEvent) -> bool,
) -> Vec<TermEvent> {
    let deadline = Instant::now() + duration;
    let mut events = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(TermEvent::Wakeup) => {}
            Ok(event) => {
                let done = last(&event);
                events.push(event);
                if done {
                    // Anything right behind it would be a mistake too.
                    events.extend(
                        rx.recv_timeout(Duration::from_millis(100)).into_iter().filter(|e| *e != TermEvent::Wakeup),
                    );
                    break;
                }
            }
            Err(_) => break,
        }
    }
    events
}

#[test]
fn notifications_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, rx) =
        spawn(dir.path(), "/bin/sh", &["-c", r"printf '\033]9;Agent turn complete\007\033]9;4;1;50\007'; sleep 5"]);
    let events = events_until(&rx, Duration::from_secs(5), |e| matches!(e, TermEvent::Notify { .. }));
    assert_eq!(events, [TermEvent::Notify { title: None, body: "Agent turn complete".into() }]);
}

const FAST: ActivityConfig =
    ActivityConfig { busy_after: Duration::from_millis(300), quiet_after: Duration::from_millis(300) };

#[test]
fn steady_output_is_busy_then_idle() {
    let dir = tempfile::tempdir().unwrap();
    let script = "i=0; while [ $i -lt 10 ]; do echo tick; sleep 0.1; i=$((i+1)); done; sleep 5";
    let (_session, rx) = spawn_with(dir.path(), "/bin/sh", &["-c", script], FAST);
    let events = events_until(&rx, Duration::from_secs(5), |e| matches!(e, TermEvent::Idle { .. }));
    let [TermEvent::Busy, TermEvent::Idle { worked }] = &events[..] else { panic!("got {events:?}") };
    assert!(*worked >= Duration::from_millis(300), "worked {worked:?}");
}

#[test]
fn a_short_burst_is_not_busy() {
    let dir = tempfile::tempdir().unwrap();
    let (_session, rx) = spawn_with(dir.path(), "/bin/sh", &["-c", "echo once; sleep 5"], FAST);
    assert_eq!(events_until(&rx, Duration::from_millis(800), |_| true), []);
}
