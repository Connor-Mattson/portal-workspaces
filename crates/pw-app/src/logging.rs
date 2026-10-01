//! Logs to stderr and to a size-bounded file next to the state file, and records panics there.
//!
//! Launched from a desktop entry or an app bundle, stderr goes nowhere, so the file is what's
//! left after something goes wrong. It holds at most two files of [`MAX_FILE`] bytes:
//! `portal-workspaces.log` and the one before it, `portal-workspaces.log.1`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::persist::Flush;

const FILE_NAME: &str = "portal-workspaces.log";
const MAX_FILE: u64 = 2 * 1024 * 1024;
/// How long the panic hook waits for a log line another thread is writing.
const LOCK_WAIT: Duration = Duration::from_millis(200);

/// Where logs go: a `logs` folder beside the state file.
pub fn log_dir(state_path: &Path) -> PathBuf {
    state_path.parent().unwrap_or(Path::new(".")).join("logs")
}

/// Starts logging to stderr and, if it can be opened, the log file in `dir`.
pub fn init(dir: &Path) -> Option<Log> {
    // The binary's crate is `portal_workspaces`, not `pw_app`: the app's own events go by that name.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!("warn,{}=info", env!("CARGO_CRATE_NAME")).into());
    let log = Log::open(dir, MAX_FILE);
    let file = log.clone().map(|log| tracing_subscriber::fmt::layer().with_ansi(false).with_writer(log));
    tracing_subscriber::registry().with(filter).with(tracing_subscriber::fmt::layer()).with(file).init();
    if log.is_none() {
        tracing::warn!(dir = %dir.display(), "couldn't open the log file; logging to stderr only");
    }
    log
}

/// After the default report (to stderr), writes the panic and a backtrace to the log file and,
/// if it was the UI thread that panicked (the app is going down), waits briefly for state already
/// handed to the saver to reach the disk.
///
/// Nothing here can deadlock on a lock the panicking thread holds: the log file is only tried
/// for a moment, and the saver is asked over a channel with a timeout.
pub fn install_panic_hook(log: Option<Log>, flush: Flush) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let thread = std::thread::current();
        let report = report(
            thread.name().unwrap_or("unnamed"),
            &payload(info.payload()),
            info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).as_deref(),
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        if let Some(log) = &log {
            log.write_report(&report);
        }
        if thread.name() == Some("main") {
            flush.wait(Duration::from_secs(2));
        }
    }));
}

fn payload(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(not a string)".into())
}

/// One panic, as it appears in the log file.
fn report(thread: &str, message: &str, location: Option<&str>, backtrace: &str) -> String {
    let now = jiff::Timestamp::now();
    let at = location.unwrap_or("an unknown location");
    format!("{now} PANIC thread '{thread}' panicked at {at}:\n{message}\nbacktrace:\n{backtrace}\n")
}

/// The log file, rotated once it passes its size.
#[derive(Clone)]
pub struct Log(Arc<Mutex<LogFile>>);

impl Log {
    fn open(dir: &Path, max: u64) -> Option<Self> {
        fs::create_dir_all(dir).ok()?;
        let path = dir.join(FILE_NAME);
        let file = OpenOptions::new().create(true).append(true).open(&path).ok()?;
        let written = file.metadata().map_or(0, |m| m.len());
        Some(Self(Arc::new(Mutex::new(LogFile { path, file, written, max }))))
    }

    fn lock(&self) -> MutexGuard<'_, LogFile> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes from the panic hook, giving up rather than waiting on a lock that may never come
    /// free (the panic may have happened mid-write, on this thread).
    fn write_report(&self, report: &str) {
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match self.0.try_lock() {
                Ok(mut file) => {
                    let _ = file.write_all(report.as_bytes());
                    return;
                }
                Err(TryLockError::Poisoned(file)) => {
                    let mut file = file.into_inner();
                    let _ = file.write_all(report.as_bytes());
                    return;
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::WouldBlock) => return,
            }
        }
    }
}

impl<'a> MakeWriter<'a> for Log {
    type Writer = LogWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter(self.lock())
    }
}

/// The log file, held for one event.
pub struct LogWriter<'a>(MutexGuard<'a, LogFile>);

impl Write for LogWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

pub struct LogFile {
    path: PathBuf,
    file: File,
    written: u64,
    max: u64,
}

impl LogFile {
    /// Moves the full file to `.1` (replacing the older one) and starts an empty one.
    fn rotate(&mut self) -> io::Result<()> {
        let old = self.path.with_extension("log.1");
        fs::rename(&self.path, &old)?;
        self.file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written > 0 && self.written + buf.len() as u64 > self.max {
            // If rotating fails, keep appending: an oversized log beats a lost one.
            let _ = self.rotate();
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn rotates_at_its_size_and_keeps_one_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::open(dir.path(), 100).unwrap();
        let line = |c: char| format!("{}\n", c.to_string().repeat(39));
        for c in ['a', 'b', 'c', 'd', 'e', 'f', 'g'] {
            log.make_writer().write_all(line(c).as_bytes()).unwrap();
        }
        // 40 bytes a line, 100 a file: two lines each, the oldest files gone.
        assert_eq!(read(dir.path().join(FILE_NAME)), line('g'));
        assert_eq!(read(dir.path().join("portal-workspaces.log.1")), line('e') + &line('f'));
        let total: u64 = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum();
        assert!(total <= 2 * 100);
    }

    #[test]
    fn appends_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        Log::open(dir.path(), 1000).unwrap().make_writer().write_all(b"first\n").unwrap();
        Log::open(dir.path(), 1000).unwrap().make_writer().write_all(b"second\n").unwrap();
        assert_eq!(read(dir.path().join(FILE_NAME)), "first\nsecond\n");
    }

    #[test]
    fn a_panic_report_is_written_even_while_the_log_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::open(dir.path(), 1000).unwrap();
        let report = report("main", "boom", Some("src/app.rs:1:2"), "0: pw_app::main");
        assert!(report.contains("PANIC thread 'main' panicked at src/app.rs:1:2:\nboom\nbacktrace:\n0: pw_app::main"));
        // Held (as if the panic struck mid-write on this thread): the hook gives up, no deadlock.
        let held = log.make_writer();
        let started = Instant::now();
        log.write_report(&report);
        assert!(started.elapsed() < LOCK_WAIT * 2);
        drop(held);
        log.write_report(&report);
        assert_eq!(read(dir.path().join(FILE_NAME)), report);
    }
}
