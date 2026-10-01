//! The sampling daemon: one thread that reads the machine every few seconds while someone is
//! looking, and not at all while nobody is.
//!
//! It starts paused. While paused it sleeps on its command channel with no timeout, so a hidden
//! profile costs nothing. Dropping the [`Monitor`] stops it.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::Source;
use crate::sample::Sample;

/// How often a reading is taken while the profile is shown.
pub const INTERVAL: Duration = Duration::from_secs(5);
/// The first reading after resuming comes this soon: CPU and network rates need two reads, and
/// waiting a whole interval for the gauges to fill would feel broken.
pub const WARMUP: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct Timing {
    warmup: Duration,
    interval: Duration,
}

enum Command {
    SetRoots(Vec<u32>),
    SetActive(bool),
}

pub struct Monitor {
    tx: mpsc::Sender<Command>,
}

impl Monitor {
    /// Starts the daemon, paused. `make` builds the source on the monitor thread (probing GPUs
    /// can take a moment); `on_sample` runs there too.
    pub fn spawn<S: Source>(
        make: impl FnOnce() -> S + Send + 'static,
        on_sample: impl Fn(Sample) + Send + 'static,
    ) -> Self {
        Self::spawn_with(Timing { warmup: WARMUP, interval: INTERVAL }, make, on_sample)
    }

    fn spawn_with<S: Source>(
        timing: Timing,
        make: impl FnOnce() -> S + Send + 'static,
        on_sample: impl Fn(Sample) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("system-monitor".into())
            .spawn(move || run(timing, make, &rx, &on_sample))
            .expect("spawn system monitor");
        Self { tx }
    }

    /// The processes whose trees are totalled in [`Sample::trees`] (terminals' shells).
    pub fn set_roots(&self, roots: Vec<u32>) {
        let _ = self.tx.send(Command::SetRoots(roots));
    }

    /// Starts or pauses sampling.
    pub fn set_active(&self, active: bool) {
        let _ = self.tx.send(Command::SetActive(active));
    }
}

fn run<S: Source>(timing: Timing, make: impl FnOnce() -> S, rx: &mpsc::Receiver<Command>, on_sample: &impl Fn(Sample)) {
    let mut source: Option<S> = None;
    let mut make = Some(make);
    let mut roots = Vec::new();
    // `None` while paused.
    let mut due: Option<Instant> = None;
    loop {
        let command = match due {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Command::SetRoots(r)) => roots = r,
            Ok(Command::SetActive(true)) if due.is_none() => {
                // Built on first use, so an app that never shows the profile never probes.
                let source = source.get_or_insert_with(|| make.take().expect("made once")());
                // A fresh baseline, so the first rates cover the warmup, not the whole pause.
                source.prime();
                due = Some(Instant::now() + timing.warmup);
            }
            Ok(Command::SetActive(true)) => {}
            Ok(Command::SetActive(false)) => due = None,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if let (Some(at), Some(source)) = (due, source.as_mut())
            && Instant::now() >= at
        {
            on_sample(source.sample(&roots));
            due = Some(Instant::now() + timing.interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    use super::*;
    use crate::sample::{Cpu, Memory, Network};

    #[derive(Default)]
    struct Log {
        made: u32,
        primed: u32,
        roots: Vec<Vec<u32>>,
    }

    struct Fake(Arc<Mutex<Log>>);

    impl Source for Fake {
        fn prime(&mut self) {
            self.0.lock().unwrap().primed += 1;
        }

        fn sample(&mut self, roots: &[u32]) -> Sample {
            self.0.lock().unwrap().roots.push(roots.to_vec());
            Sample {
                at: SystemTime::now(),
                cpu: Cpu { usage: 1.0, cores: 1, brand: String::new(), load: None },
                memory: Memory::default(),
                gpus: Vec::new(),
                gpu_note: None,
                network: Network::default(),
                processes: Vec::new(),
                trees: Vec::new(),
            }
        }
    }

    #[test]
    fn samples_only_while_active() {
        let log = Arc::new(Mutex::new(Log::default()));
        let (tx, rx) = mpsc::channel();
        let made = log.clone();
        let timing = Timing { warmup: Duration::from_millis(60), interval: Duration::from_secs(60) };
        let monitor = Monitor::spawn_with(
            timing,
            move || {
                made.lock().unwrap().made += 1;
                Fake(made.clone())
            },
            move |s| {
                let _ = tx.send(s);
            },
        );

        // Paused at start: nothing is built and nothing arrives.
        let quiet = timing.warmup * 4;
        assert!(rx.recv_timeout(quiet).is_err());
        assert_eq!(log.lock().unwrap().made, 0);

        monitor.set_roots(vec![42]);
        monitor.set_active(true);
        let started = Instant::now();
        rx.recv_timeout(Duration::from_secs(5)).expect("a first reading after the warmup");
        assert!(started.elapsed() >= timing.warmup);
        {
            let log = log.lock().unwrap();
            assert_eq!((log.made, log.primed), (1, 1));
            assert_eq!(log.roots, vec![vec![42]]);
        }

        // Pausing stops the schedule; resuming re-primes but doesn't rebuild.
        monitor.set_active(false);
        assert!(rx.recv_timeout(quiet).is_err());
        monitor.set_active(true);
        monitor.set_active(true);
        rx.recv_timeout(Duration::from_secs(5)).expect("sampling resumes");
        // The interval is long, so nothing more comes.
        assert!(rx.recv_timeout(quiet).is_err());
        let log = log.lock().unwrap();
        assert_eq!((log.made, log.primed), (1, 2));
    }
}
