pub mod settings;
pub mod state;

// Re-export surface, not scaffolding -- see the note in github/mod.rs.
#[allow(unused_imports)]
pub use settings::{Appearance, BranchWatch, RepoConfig, Rules, Session, Settings};
pub use state::{AppState, ProtectedBranchesCache};

use std::fs;
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE: &str = "settings.json";
pub const STATE_FILE: &str = "state.json";

/// Load JSON, or fall back to `Default` after moving the unreadable
/// file aside. Never returns an error: a corrupt file must not stop
/// the app from starting.
fn load_or_default<T: serde::de::DeserializeOwned + Default>(path: &Path) -> (T, bool) {
    let Ok(raw) = fs::read_to_string(path) else {
        return (T::default(), false);
    };
    match serde_json::from_str(&raw) {
        Ok(v) => (v, false),
        Err(_) => {
            let _ = fs::rename(path, path.with_extension("json.corrupt"));
            (T::default(), true)
        }
    }
}

fn save<T: serde::Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write-then-rename so an interrupted write can't truncate the file.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)
}

pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}
pub fn state_path(dir: &Path) -> PathBuf {
    dir.join(STATE_FILE)
}

pub fn load_settings(dir: &Path) -> (Settings, bool) {
    load_or_default(&settings_path(dir))
}
pub fn save_settings(dir: &Path, s: &Settings) -> std::io::Result<()> {
    save(&settings_path(dir), s)
}
pub fn load_state(dir: &Path) -> (AppState, bool) {
    load_or_default(&state_path(dir))
}
pub fn save_state(dir: &Path, s: &AppState) -> std::io::Result<()> {
    save(&state_path(dir), s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_settings() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            poll_interval_secs: 300,
            ..Default::default()
        };
        save_settings(dir.path(), &s).unwrap();
        let (loaded, recovered) = load_settings(dir.path());
        assert_eq!(loaded, s);
        assert!(!recovered);
    }

    #[test]
    fn missing_file_yields_defaults_without_flagging_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let (s, recovered) = load_settings(dir.path());
        assert_eq!(s, Settings::default());
        assert!(!recovered);
    }

    #[test]
    fn corrupt_file_is_moved_aside_and_flagged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(settings_path(dir.path()), "{not json").unwrap();

        let (s, recovered) = load_settings(dir.path());

        assert_eq!(s, Settings::default());
        assert!(recovered, "caller must be able to notify the user");
        assert!(dir.path().join("settings.json.corrupt").exists());
    }
}
