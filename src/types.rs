use serde::{Deserialize, Serialize};

// ─── Liquid State ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LiquidState {
    #[default]
    Empty = 1,
    Filling = 2,
    Cooling = 4,
    Heating = 5,
    StableTemperature = 6,
}

impl LiquidState {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Empty),
            2 => Some(Self::Filling),
            4 => Some(Self::Cooling),
            5 => Some(Self::Heating),
            6 => Some(Self::StableTemperature),
            _ => None,
        }
    }
}

// ─── Temperature Unit ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TemperatureUnit {
    Celsius = 0,
    #[default]
    Fahrenheit = 1,
}

impl TemperatureUnit {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Celsius),
            1 => Some(Self::Fahrenheit),
            _ => None,
        }
    }
}

// ─── LED Color ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for RgbColor {
    fn default() -> Self {
        Self { r: 255, g: 147, b: 41, a: 255 } // Warm orange
    }
}

// ─── Mug State ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MugState {
    pub connected: bool,
    pub battery_level: u8,
    pub is_charging: bool,
    pub current_temp: f64,  // Always Celsius
    pub target_temp: f64,   // Always Celsius
    pub liquid_state: LiquidState,
    pub temperature_unit: TemperatureUnit,
    pub color: RgbColor,
    pub mug_name: String,
}

impl Default for MugState {
    fn default() -> Self {
        Self {
            connected: false,
            battery_level: 0,
            is_charging: false,
            current_temp: 0.0,
            target_temp: 55.0,
            liquid_state: LiquidState::Empty,
            temperature_unit: TemperatureUnit::Celsius,
            color: RgbColor::default(),
            mug_name: String::new(),
        }
    }
}

// ─── Preset ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub temperature: f64, // Always Celsius
}

// ─── Constants ───────────────────────────────────────────────────────────────

pub const MIN_TEMP_CELSIUS: f64 = 50.0;
pub const MAX_TEMP_CELSIUS: f64 = 63.0;

pub const BATTERY_DRAIN_RATE_HEATING: f64 = 0.5;     // % per minute
pub const BATTERY_DRAIN_RATE_MAINTAINING: f64 = 0.2;  // % per minute
pub const BATTERY_CHARGE_RATE: f64 = 1.5;             // % per minute

// ─── Ember BLE UUIDs ─────────────────────────────────────────────────────────

/// Ember service UUID (unused at runtime — btleplug scans by name — kept for reference).
#[allow(dead_code)]
pub const EMBER_SERVICE_UUID: &str = "fc543622-236c-4c94-8fa9-944a3e5353fa";

/// Characteristic UUIDs (hex strings -> standard UUID format).
pub mod chars {
    pub const MUG_NAME: &str      = "fc540001-236c-4c94-8fa9-944a3e5353fa";
    pub const CURRENT_TEMP: &str  = "fc540002-236c-4c94-8fa9-944a3e5353fa";
    pub const TARGET_TEMP: &str   = "fc540003-236c-4c94-8fa9-944a3e5353fa";
    pub const TEMP_UNIT: &str     = "fc540004-236c-4c94-8fa9-944a3e5353fa";
    pub const BATTERY: &str       = "fc540007-236c-4c94-8fa9-944a3e5353fa";
    pub const LIQUID_STATE: &str  = "fc540008-236c-4c94-8fa9-944a3e5353fa";
    pub const DSK: &str           = "fc54000e-236c-4c94-8fa9-944a3e5353fa";
    pub const UDSK: &str          = "fc54000f-236c-4c94-8fa9-944a3e5353fa";
    pub const PUSH_EVENTS: &str   = "fc540012-236c-4c94-8fa9-944a3e5353fa";
    pub const LED_COLOR: &str     = "fc540014-236c-4c94-8fa9-944a3e5353fa";
}
