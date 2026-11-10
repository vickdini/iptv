use std::{fs, path::PathBuf};

use iptv_core::SavedState;

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("iptv")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("iptv")
}

fn state_path() -> PathBuf {
    config_dir().join("state.json")
}

pub fn load_state() -> SavedState {
    fs::read_to_string(state_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(state: &SavedState) {
    let path = state_path();
    let result = fs::create_dir_all(config_dir()).and_then(|_| {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(state).unwrap_or_default())?;
        fs::rename(&tmp, &path)
    });
    if let Err(e) = result {
        eprintln!("warning: could not save state to {}: {e}", path.display());
    }
}
