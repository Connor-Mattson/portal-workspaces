//! End-to-end checks against real PTYs and shells.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pw_term::{GridSize, Session, SessionConfig, TermEvent};

const SIZE: GridSize = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };

fn spawn(cwd: &Path, program: &str, args: &[&str]) -> (Session, mpsc::Receiver<TermEvent>) {
    let (tx, rx) = mpsc::channel();
    let mut config = SessionConfig::new(cwd.to_owned(), SIZE);
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
