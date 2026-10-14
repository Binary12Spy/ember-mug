use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::types::{Preset, RgbColor, TemperatureUnit};

// ─── Settings struct ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub presets: Vec<Preset>,
    pub temperature_unit: TemperatureUnit,
    pub notify_on_temperature_reached: bool,
    pub notify_at_battery_percentage: u8,
    pub last_target_temp: f64,
    pub led_color: RgbColor,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            presets: default_presets(),
            temperature_unit: TemperatureUnit::Fahrenheit,
            notify_on_temperature_reached: true,
            notify_at_battery_percentage: 15,
            last_target_temp: 55.0,
            led_color: RgbColor { r: 255, g: 147, b: 41, a: 255 },
        }
    }
}

pub fn default_presets() -> Vec<Preset> {
    vec![
        Preset { id: "1".to_string(), name: "Latte".to_string(),  temperature: 52.00 },
        Preset { id: "2".to_string(), name: "Coffee".to_string(), temperature: 55.56 },
        Preset { id: "3".to_string(), name: "Tea".to_string(),    temperature: 60.00 },
    ]
}

// ─── Persistence ─────────────────────────────────────────────────────────────

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ember-mug-rs").join("settings.json"))
}

pub fn load_settings() -> Settings {
    let Some(path) = config_path() else {
        return Settings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    let Some(path) = config_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, content)?;
    Ok(())
}
