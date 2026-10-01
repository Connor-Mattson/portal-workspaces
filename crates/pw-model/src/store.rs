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
pub fn migrate(mut value: Value) -> Result<Value, String> {
    loop {
        let version = value.get("schema_version").and_then(Value::as_u64).ok_or("missing schema_version")?;
        match version {
            v if v == u64::from(SCHEMA_VERSION) => return Ok(value),
            // 1 → 2: usage profiles.
            1 => {
                let doc = value.as_object_mut().ok_or("state is not an object")?;
                doc.insert("usage_profiles".into(), Value::Array(Vec::new()));
                doc.insert("schema_version".into(), 2.into());
            }
            // 2 → 3: detached panes. `ui.terminal_window` defaults like every other `UiPrefs` field.
            2 => {
                let doc = value.as_object_mut().ok_or("state is not an object")?;
                for ws in doc.get_mut("workspaces").and_then(Value::as_array_mut).into_iter().flatten() {
                    ws.as_object_mut()
                        .ok_or("workspace is not an object")?
                        .insert("detached".into(), Value::Array(Vec::new()));
                }
                doc.insert("schema_version".into(), 3.into());
            }
            // 3 → 4: Editor mode. Every workspace starts as agents; `editor` and `ui.editor_font_size` default.
            3 => {
                let doc = value.as_object_mut().ok_or("state is not an object")?;
                for ws in doc.get_mut("workspaces").and_then(Value::as_array_mut).into_iter().flatten() {
                    ws.as_object_mut().ok_or("workspace is not an object")?.insert("mode".into(), "agents".into());
                }
                doc.insert("schema_version".into(), 4.into());
            }
            // 4 → 5: the system profile. `ui.system_expanded` defaults (open) like every `UiPrefs` field.
            4 => {
                let doc = value.as_object_mut().ok_or("state is not an object")?;
                doc.insert("schema_version".into(), 5.into());
            }
            // 5 → 6: desktop notifications. `ui.notifications` defaults (on) like every `UiPrefs` field.
            5 => {
                let doc = value.as_object_mut().ok_or("state is not an object")?;
                doc.insert("schema_version".into(), 6.into());
            }
            v => return Err(format!("unsupported schema_version {v} (this build reads {SCHEMA_VERSION})")),
        }
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
    use crate::usage::{Provider, UsageProfile};
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
    fn version_1_files_are_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let v1 = r#"{"schema_version": 1, "workspaces": [], "active": null,
            "ui": {"sidebar_collapsed": true, "font_size": 14.0, "window": null}}"#;
        fs::write(&path, v1).unwrap();
        let LoadOutcome::Loaded(state) = load(&path) else { panic!("expected Loaded") };
        assert_eq!(state.schema_version, SCHEMA_VERSION);
        assert!(state.usage_profiles.is_empty());
        assert!(state.ui.sidebar_collapsed);
        assert!(state.ui.usage_expanded);
    }

    #[test]
    fn version_2_files_are_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = sample();
        let mut v2 = serde_json::to_value(&state).unwrap();
        v2["schema_version"] = 2.into();
        for ws in v2["workspaces"].as_array_mut().unwrap() {
            ws.as_object_mut().unwrap().remove("detached");
        }
        v2["ui"].as_object_mut().unwrap().remove("terminal_window");
        fs::write(&path, serde_json::to_vec(&v2).unwrap()).unwrap();
        let LoadOutcome::Loaded(loaded) = load(&path) else { panic!("expected Loaded") };
        assert_eq!(loaded, state);
    }

    #[test]
    fn version_3_files_are_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = sample();
        let mut v3 = serde_json::to_value(&state).unwrap();
        v3["schema_version"] = 3.into();
        for ws in v3["workspaces"].as_array_mut().unwrap() {
            let ws = ws.as_object_mut().unwrap();
            ws.remove("mode");
            ws.remove("editor");
        }
        v3["ui"].as_object_mut().unwrap().remove("editor_font_size");
        fs::write(&path, serde_json::to_vec(&v3).unwrap()).unwrap();
        let LoadOutcome::Loaded(loaded) = load(&path) else { panic!("expected Loaded") };
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.ui, state.ui);
        for (got, want) in loaded.workspaces.iter().zip(&state.workspaces) {
            assert_eq!(got.mode, crate::Mode::Agents);
            assert_eq!((&got.layout, &got.panes, &got.detached), (&want.layout, &want.panes, &want.detached));
            assert_eq!(got.editor.groups.len(), 1);
            assert!(!got.panes.contains_key(&got.editor.terminal.id));
        }
    }

    #[test]
    fn version_4_files_are_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = sample();
        let mut v4 = serde_json::to_value(&state).unwrap();
        v4["schema_version"] = 4.into();
        v4["ui"].as_object_mut().unwrap().remove("system_expanded");
        fs::write(&path, serde_json::to_vec(&v4).unwrap()).unwrap();
        let LoadOutcome::Loaded(loaded) = load(&path) else { panic!("expected Loaded") };
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert!(loaded.ui.system_expanded);
        assert_eq!(loaded, state);
    }

    #[test]
    fn version_5_files_are_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = sample();
        let mut v5 = serde_json::to_value(&state).unwrap();
        v5["schema_version"] = 5.into();
        v5["ui"].as_object_mut().unwrap().remove("notifications");
        fs::write(&path, serde_json::to_vec(&v5).unwrap()).unwrap();
        let LoadOutcome::Loaded(loaded) = load(&path) else { panic!("expected Loaded") };
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert!(loaded.ui.notifications);
        assert_eq!(loaded, state);
    }

    #[test]
    fn editor_sessions_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = sample();
        let ws = &mut state.workspaces[1];
        ws.mode = crate::Mode::Editor;
        let group = ws.editor.focused;
        let spec = crate::GroupSpec {
            tabs: vec![crate::OpenFile { path: "src/main.rs".into(), line: 12, column: 4, preview: false }],
            active: Some("src/main.rs".into()),
        };
        ws.editor.split(group, crate::Axis::Vertical, crate::GroupId::new(), spec).unwrap();
        ws.editor.expanded = vec!["src".into()];
        ws.editor.show_terminal = false;
        state.ui.editor_font_size = 15.0;
        save(&path, &state).unwrap();
        assert_eq!(load(&path).into_state(), state);
    }

    #[test]
    fn detached_panes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = sample();
        let pane = crate::PaneId::new();
        state.workspaces[0].detached.push(pane);
        state.workspaces[0].panes.insert(pane, crate::PaneSpec { cwd: "/a/sub".into() });
        state.ui.terminal_window = Some(crate::WindowGeometry { width: 800.0, height: 500.0 });
        save(&path, &state).unwrap();
        assert_eq!(load(&path).into_state(), state);
    }

    #[test]
    fn usage_profiles_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = sample();
        state.usage_profiles = vec![
            UsageProfile::new("Work", Provider::Claude, "alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'"),
            UsageProfile::new("Codex", Provider::Codex, ""),
        ];
        let duplicate = state.usage_profiles[0].clone();
        save(&path, &state).unwrap();
        assert_eq!(load(&path).into_state().usage_profiles, state.usage_profiles);

        state.usage_profiles.push(duplicate);
        save(&path, &state).unwrap();
        assert_eq!(load(&path).into_state().usage_profiles.len(), 2);
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
