//! User settings, stored as JSON in the app config directory.

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// "shortcut" | "ctrl_alt" (hold to talk, tap to type) | "double_ctrl". The last two need the
    /// Windows keyboard hook; the shortcut below always works as well.
    pub wake_mode: String,
    /// Accelerator such as "Ctrl+Alt+Space". Empty disables it.
    pub shortcut: String,
    /// "claude" | "codex"
    pub provider: String,
    pub claude_model: String,
    /// Empty means "whatever the Codex CLI is configured to use".
    pub codex_model: String,
    /// Attach a screenshot of the active monitor to each question.
    pub include_screen: bool,
    /// Exclude the overlay from screenshots, recordings and screen sharing.
    pub hide_from_capture: bool,
    pub hide_on_blur: bool,
    pub autostart: bool,
    /// Optional explicit paths to the CLIs. Empty means "find on PATH".
    pub claude_path: String,
    pub codex_path: String,
    /// How long an idle Claude session stays in memory before it is closed.
    pub keep_warm_minutes: u32,
    /// Speech model: "tiny.en", "base.en", "small.en", "base" or "small".
    pub voice_model: String,
    /// Screen edge the overlay is docked to, and how far along it (0 to 1).
    pub dock_edge: crate::dock::Edge,
    pub dock_along: f64,
    /// How solid the overlay background is, from 0.3 (see-through) to 1.0.
    pub opacity: f64,
    /// Show a soft glow around the screen while listening.
    pub edge_glow: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            wake_mode: if cfg!(windows) { "ctrl_alt" } else { "shortcut" }.into(),
            shortcut: "Ctrl+Alt+Space".into(),
            provider: "claude".into(),
            claude_model: "haiku".into(),
            codex_model: String::new(),
            include_screen: true,
            hide_from_capture: true,
            hide_on_blur: false,
            autostart: false,
            claude_path: String::new(),
            codex_path: String::new(),
            keep_warm_minutes: 5,
            voice_model: "base.en".into(),
            dock_edge: crate::dock::Edge::Top,
            dock_along: 0.5,
            opacity: 0.92,
            edge_glow: true,
        }
    }
}

impl Settings {
    pub fn load(dir: &PathBuf) -> Self {
        fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &PathBuf) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(dir.join("settings.json"), json).map_err(|e| e.to_string())
    }
}
