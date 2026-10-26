use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn full_strength() -> f32 {
    1.0
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub input_device: Option<String>,
    pub noise_cancel: bool,
    #[serde(default = "full_strength")]
    pub strength: f32,
    #[serde(default = "enabled")]
    pub clarity_boost: bool,
    #[serde(default = "enabled")]
    pub voice_leveler: bool,
    #[serde(default = "enabled")]
    pub low_cut: bool,
    #[serde(default = "enabled")]
    pub minimize_to_tray: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            input_device: None,
            noise_cancel: true,
            strength: 1.0,
            clarity_boost: true,
            voice_leveler: true,
            low_cut: true,
            minimize_to_tray: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = Self::path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    fn path() -> PathBuf {
        let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("Frisp").join("config.json")
    }
}
