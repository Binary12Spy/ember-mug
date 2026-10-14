use crate::types::LiquidState;
use ratatui::style::Color;

// ─── Theme ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub primary: Color,
    pub border: Color,
    pub text: Color,
    pub dim_text: Color,
}

impl Theme {
    pub const HEATING: Self = Self {
        primary: Color::Rgb(255, 107, 53),
        border: Color::Rgb(255, 107, 53),
        text: Color::White,
        dim_text: Color::Gray,
    };

    pub const COOLING: Self = Self {
        primary: Color::Rgb(74, 144, 217),
        border: Color::Rgb(74, 144, 217),
        text: Color::White,
        dim_text: Color::Gray,
    };

    pub const STABLE: Self = Self {
        primary: Color::Yellow,
        border: Color::Yellow,
        text: Color::White,
        dim_text: Color::Gray,
    };

    pub const EMPTY: Self = Self {
        primary: Color::Gray,
        border: Color::Gray,
        text: Color::White,
        dim_text: Color::DarkGray,
    };

    pub const FILLING: Self = Self {
        primary: Color::Cyan,
        border: Color::Cyan,
        text: Color::White,
        dim_text: Color::Gray,
    };

    pub const DISCONNECTED: Self = Self {
        primary: Color::DarkGray,
        border: Color::DarkGray,
        text: Color::White,
        dim_text: Color::DarkGray,
    };
}

pub fn get_theme(liquid_state: LiquidState, connected: bool) -> Theme {
    if !connected {
        return Theme::DISCONNECTED;
    }
    match liquid_state {
        LiquidState::Empty => Theme::EMPTY,
        LiquidState::Filling => Theme::FILLING,
        LiquidState::StableTemperature => Theme::STABLE,
        LiquidState::Heating => Theme::HEATING,
        LiquidState::Cooling => Theme::COOLING,
    }
}

// ─── Preset LED colors ────────────────────────────────────────────────────────

use crate::types::RgbColor;

pub struct LedPreset {
    pub name: &'static str,
    pub color: RgbColor,
}

pub const LED_PRESETS: &[LedPreset] = &[
    LedPreset { name: "Orange", color: RgbColor { r: 255, g: 147, b:  41, a: 255 } },
    LedPreset { name: "Red",    color: RgbColor { r: 255, g:   0, b:   0, a: 255 } },
    LedPreset { name: "Green",  color: RgbColor { r:   0, g: 255, b:   0, a: 255 } },
    LedPreset { name: "Blue",   color: RgbColor { r:   0, g: 128, b: 255, a: 255 } },
    LedPreset { name: "Purple", color: RgbColor { r: 128, g:   0, b: 255, a: 255 } },
    LedPreset { name: "Pink",   color: RgbColor { r: 255, g: 105, b: 180, a: 255 } },
    LedPreset { name: "White",  color: RgbColor { r: 255, g: 255, b: 255, a: 255 } },
    LedPreset { name: "Teal",   color: RgbColor { r:   0, g: 255, b: 200, a: 255 } },
];

pub fn find_led_preset_index(color: &RgbColor) -> Option<usize> {
    LED_PRESETS.iter().position(|p| {
        p.color.r == color.r && p.color.g == color.g && p.color.b == color.b
    })
}
