//! Loading and saving [`PersistedState`] as JSON.
//!
//! Saves are atomic (temp file + rename), and a file that can't be read is moved aside rather
//! than crashing the app or being silently overwritten.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::state::{PersistedState, SCHEMA_VERSION};

#[derive(Debug)]
pub enum LoadOutcome {
    /// No state file yet (first run).
    Fresh,
    Loaded(PersistedState),
    /// The file was unreadable; it was moved to `backup` and the app starts empty.
    Recovered {
        backup: PathBuf,
        reason: String,
    },
}

impl LoadOutcome {
    pub fn into_state(self) -> PersistedState {
        match self {
            LoadOutcome::Loaded(state) => state,
            LoadOutcome::Fresh | LoadOutcome::Recovered { .. } => PersistedState::default(),
        }
    }
}

pub fn load(path: &Path) -> LoadOutcome {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return LoadOutcome::Fresh,
        Err(err) => return move_aside(path, format!("read failed: {err}")),
    };
    match parse(&bytes) {
        Ok(state) => LoadOutcome::Loaded(state.repaired()),
        Err(reason) => move_aside(path, reason),
    }
}

fn parse(bytes: &[u8]) -> Result<PersistedState, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON: {e}"))?;
    let value = migrate(value)?;
    serde_json::from_value(value).map_err(|e| format!("unexpected shape: {e}"))
}

/// Upgrades an older document to [`SCHEMA_VERSION`]. Add one arm per version bump.
pub fn migrate(value: Value) -> Result<Value, String> {
    let version = value.get("schema_version").and_then(Value::as_u64).ok_or("missing schema_version")?;
    match version {
        v if v == u64::from(SCHEMA_VERSION) => Ok(value),
        v => Err(format!("unsupported schema_version {v} (this build reads {SCHEMA_VERSION})")),
    }
}

fn move_aside(path: &Path, reason: String) -> LoadOutcome {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut backup = path.as_os_str().to_owned();
    backup.push(format!(".bak-{stamp}"));
    let backup = PathBuf::from(backup);
    if let Err(err) = fs::rename(path, &backup) {
        tracing::warn!(%err, "could not move unreadable state file aside");
    }
    tracing::warn!(%reason, backup = %backup.display(), "state file unreadable; starting fresh");
    LoadOutcome::Recovered { backup, reason }
}

/// Writes `state` to `path` atomically, creating parent directories as needed.
pub fn save(path: &Path, state: &PersistedState) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Preset;
    use crate::workspace::Workspace;

    fn sample() -> PersistedState {
        let a = Workspace::new("alpha", "/a".into(), Preset::Columns(2));
        let b = Workspace::new("beta", "/b".into(), Preset::Grid { cols: 3, rows: 2 });
        PersistedState { active: Some(b.id), workspaces: vec![a, b], ..Default::default() }
    }

    #[test]
    fn missing_file_is_fresh() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load(&dir.path().join("state.json")), LoadOutcome::Fresh));
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.json");
        let state = sample();
        save(&path, &state).unwrap();
        let LoadOutcome::Loaded(loaded) = load(&path) else { panic!("expected Loaded") };
        assert_eq!(loaded, state);
        assert!(!dir.path().join("nested/state.json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"{ not json").unwrap();
        let LoadOutcome::Recovered { backup, .. } = load(&path) else { panic!("expected Recovered") };
        assert!(!path.exists());
        assert_eq!(fs::read(backup).unwrap(), b"{ not json");
    }

    #[test]
    fn future_schema_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, br#"{"schema_version": 999, "workspaces": []}"#).unwrap();
        let outcome = load(&path);
        let LoadOutcome::Recovered { reason, backup } = outcome else { panic!("expected Recovered") };
        assert!(reason.contains("999"));
        assert!(backup.exists());
    }

    #[test]
    fn dangling_active_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = sample();
        state.active = Some(crate::WorkspaceId::new());
        save(&path, &state).unwrap();
        let loaded = load(&path).into_state();
        assert_eq!(loaded.active, Some(state.workspaces[0].id));
    }
}
