//! Running a helper program with a bounded wait.
//!
//! The monitor must never hang on a child: the program may not exit (a stuck CLI, a Keychain
//! prompt nobody answers), and even once it has, a process it left in the background can still
//! hold its stdout open. So the child runs in its own process group, which is killed as a whole
//! if it outlives the timeout, and stdout is read on a helper thread the caller never joins.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// How long to keep reading after the child exits, for output a background process still holds.
const DRAIN: Duration = Duration::from_millis(250);
/// How often to check whether the child has exited while no output arrives.
const TICK: Duration = Duration::from_millis(50);
/// The most stdout kept.
const MAX_OUTPUT: u64 = 1 << 20;

#[derive(Debug)]
pub(crate) struct Finished {
    pub status: ExitStatus,
    pub stdout: String,
}

#[derive(Debug)]
pub(crate) enum Error {
    Spawn(std::io::Error),
    Wait(std::io::Error),
    /// Still running at the deadline; its process group was killed.
    TimedOut,
}

/// Runs `command` with stderr discarded, writing `input` to its stdin (or giving it none), and
/// waits at most `timeout`. Stdin closes once `done` matches a line of output.
pub(crate) fn run(
    mut command: Command,
    input: Option<&str>,
    done: impl Fn(&str) -> bool,
    timeout: Duration,
) -> Result<Finished, Error> {
    let mut child = command
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(Error::Spawn)?;
    let deadline = Instant::now() + timeout;
    let mut stdin = child.stdin.take();
    if let (Some(pipe), Some(input)) = (stdin.as_mut(), input) {
        let _ = pipe.write_all(input.as_bytes());
    }
    let stdout = child.stdout.take().expect("piped");
    let (tx, lines) = mpsc::channel();
    // Read on a helper thread so a chatty child can't fill the pipe and stall while we wait. It
    // ends at EOF, which a background process holding the pipe may delay indefinitely: nothing
    // waits for it.
    std::thread::spawn(move || {
        for line in BufReader::new(stdout.take(MAX_OUTPUT)).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let mut out = String::new();
    let mut exited: Option<(ExitStatus, Instant)> = None;
    loop {
        if exited.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => exited = Some((status, Instant::now())),
                Ok(None) if Instant::now() < deadline => {}
                Ok(None) => {
                    kill_group(&child);
                    let _ = child.wait();
                    return Err(Error::TimedOut);
                }
                Err(err) => {
                    kill_group(&child);
                    return Err(Error::Wait(err));
                }
            }
        }
        let now = Instant::now();
        let wait = match exited {
            Some((_, at)) => (at + DRAIN).saturating_duration_since(now),
            None => TICK.min(deadline.saturating_duration_since(now)),
        };
        match lines.recv_timeout(wait) {
            Ok(line) => {
                if done(&line) {
                    stdin = None; // EOF: a scripted server exits once it has answered.
                }
                out.push_str(&line);
                out.push('\n');
            }
            Err(RecvTimeoutError::Timeout) if exited.is_none() => {}
            // Stdout closed, or the child exited and what it left behind kept stdout open past
            // the drain.
            Err(_) => break,
        }
    }
    drop(stdin);
    let status = match exited {
        Some((status, _)) => status,
        None => wait_until(&mut child, deadline)?,
    };
    Ok(Finished { status, stdout: out })
}

/// Waits for a child that closed its stdout to exit, killing its group at the deadline.
fn wait_until(child: &mut std::process::Child, deadline: Instant) -> Result<ExitStatus, Error> {
    loop {
        match child.try_wait().map_err(Error::Wait)? {
            Some(status) => return Ok(status),
            None if Instant::now() < deadline => std::thread::sleep(TICK),
            None => {
                kill_group(child);
                let _ = child.wait();
                return Err(Error::TimedOut);
            }
        }
    }
}

/// Kills the child and anything it started in its process group.
fn kill_group(child: &std::process::Child) {
    use rustix::process::{Pid, Signal, kill_process_group};
    if let Some(pid) = Pid::from_raw(child.id() as i32) {
        let _ = kill_process_group(pid, Signal::KILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn returns_output_and_status() {
        let done = run(sh("echo one; echo two; exit 3"), None, |_| false, Duration::from_secs(5)).unwrap();
        assert_eq!(done.stdout, "one\ntwo\n");
        assert_eq!(done.status.code(), Some(3));
    }

    #[test]
    fn a_background_process_holding_stdout_does_not_hold_us() {
        // Before bounding the reader, this waited for the whole `sleep`.
        let started = Instant::now();
        let done = run(sh("sleep 30 & echo hi"), None, |_| false, Duration::from_secs(10)).unwrap();
        assert!(done.status.success());
        assert_eq!(done.stdout, "hi\n");
        assert!(started.elapsed() < DRAIN + Duration::from_secs(1), "took {:?}", started.elapsed());
    }

    #[test]
    fn a_child_that_never_exits_times_out_with_its_group() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived");
        let script = format!("(sleep 0.4; touch {}) & sleep 30", marker.display());
        let started = Instant::now();
        assert!(matches!(run(sh(&script), None, |_| false, Duration::from_millis(100)), Err(Error::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        // The background subshell was in the group, so it died before touching the marker.
        std::thread::sleep(Duration::from_millis(600));
        assert!(!marker.exists());
    }

    #[test]
    fn stdin_closes_once_answered() {
        // `cat` echoes its input and exits at EOF, which only comes once `done` matches.
        let done = run(Command::new("cat"), Some("a\nb\n"), |line| line == "b", Duration::from_secs(5)).unwrap();
        assert_eq!(done.stdout, "a\nb\n");
        assert!(done.status.success());
    }
}
