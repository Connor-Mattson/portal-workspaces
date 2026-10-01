//! Where state lives on disk, and a background writer so saving never blocks a frame.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use pw_model::{PersistedState, store};

pub fn state_path() -> PathBuf {
    if let Ok(path) = std::env::var("PORTAL_WORKSPACES_STATE") {
        return PathBuf::from(path);
    }
    directories::ProjectDirs::from("", "", "Portal Workspaces")
        .map(|d| d.config_dir().join("state.json"))
        .unwrap_or_else(|| crate::sessions::home_dir().join(".portal-workspaces.json"))
}

/// Writes state on its own thread, one file write at a time. Only the newest pending state is
/// written.
pub struct Saver {
    tx: mpsc::Sender<Job>,
}

struct Job {
    /// `None` only asks to be told once what's queued is written (see [`Flush`]).
    state: Option<PersistedState>,
    /// Signalled once this state (or a newer one) is on disk.
    done: Option<mpsc::Sender<()>>,
}

impl Saver {
    pub fn new(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("state-saver".into())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    let mut waiters: Vec<_> = job.done.take().into_iter().collect();
                    while let Ok(mut newer) = rx.try_recv() {
                        waiters.extend(newer.done.take());
                        if newer.state.is_some() {
                            job = newer;
                        }
                    }
                    if let Some(state) = &job.state
                        && let Err(err) = store::save(&path, state)
                    {
                        tracing::error!(%err, path = %path.display(), "saving state failed");
                    }
                    for waiter in waiters {
                        let _ = waiter.send(());
                    }
                }
            })
            .expect("spawn saver thread");
        Self { tx }
    }

    pub fn save(&self, state: PersistedState) {
        let _ = self.tx.send(Job { state: Some(state), done: None });
    }

    /// Saves and waits until it is on disk. Used on quit.
    pub fn save_and_wait(&self, state: PersistedState) {
        wait_for(&self.tx, Some(state), Duration::from_secs(2));
    }

    /// A handle that waits for what's queued, for the panic hook (which can't reach the app's
    /// state).
    pub fn flusher(&self) -> Flush {
        Flush(self.tx.clone())
    }
}

/// Waits until the states already handed to a [`Saver`] are on disk.
pub struct Flush(mpsc::Sender<Job>);

impl Flush {
    pub fn wait(&self, timeout: Duration) {
        wait_for(&self.0, None, timeout);
    }
}

fn wait_for(tx: &mpsc::Sender<Job>, state: Option<PersistedState>, timeout: Duration) {
    let (done, wait) = mpsc::channel();
    if tx.send(Job { state, done: Some(done) }).is_ok() {
        let _ = wait.recv_timeout(timeout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flush_waits_for_queued_states_and_never_replaces_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let saver = Saver::new(path.clone());
        let flush = saver.flusher();
        let mut state = PersistedState::default();
        for size in [13.0, 14.0, 15.0] {
            state.ui.font_size = size;
            saver.save(state.clone());
        }
        flush.wait(Duration::from_secs(5));
        assert_eq!(store::load(&path).into_state().ui.font_size, 15.0);
    }
}
