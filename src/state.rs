//! State persistence — only save zoom level.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedCanvasState {
    pub zoom: f64,
}

fn state_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata).join("win-canvas").join("state.json")
}

pub fn save_state(state: &SavedCanvasState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(&path, json);
    }
}

pub fn load_state() -> Option<SavedCanvasState> {
    let path = state_path();
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}
