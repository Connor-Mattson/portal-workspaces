//! Where state lives on disk, and a background writer so saving never blocks a frame.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

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
    state: PersistedState,
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
                        job = newer;
                    }
                    if let Err(err) = store::save(&path, &job.state) {
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
        let _ = self.tx.send(Job { state, done: None });
    }

    /// Saves and waits until it is on disk. Used on quit.
    pub fn save_and_wait(&self, state: PersistedState) {
        let (done, wait) = mpsc::channel();
        if self.tx.send(Job { state, done: Some(done) }).is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(2));
        }
    }
}
