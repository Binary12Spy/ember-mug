use std::collections::VecDeque;
use std::time::Instant;

use crate::types::{
    LiquidState, TemperatureUnit, BATTERY_CHARGE_RATE, BATTERY_DRAIN_RATE_HEATING,
    BATTERY_DRAIN_RATE_MAINTAINING, MAX_TEMP_CELSIUS, MIN_TEMP_CELSIUS,
};

// ─── Temperature ─────────────────────────────────────────────────────────────

pub fn format_temperature(temp: f64, unit: TemperatureUnit) -> String {
    match unit {
        TemperatureUnit::Celsius => format!("{:.1}°C", temp),
        TemperatureUnit::Fahrenheit => {
            format!("{}°F", celsius_to_fahrenheit(temp).round() as i64)
        }
    }
}

pub fn celsius_to_fahrenheit(celsius: f64) -> f64 {
    celsius * 9.0 / 5.0 + 32.0
}

pub fn fahrenheit_to_celsius(fahrenheit: f64) -> f64 {
    (fahrenheit - 32.0) * 5.0 / 9.0
}

pub fn clamp_temperature(temp: f64) -> f64 {
    temp.max(MIN_TEMP_CELSIUS).min(MAX_TEMP_CELSIUS)
}

// ─── Liquid State ────────────────────────────────────────────────────────────

pub fn get_liquid_state_text(
    state: LiquidState,
    current_temp: f64,
    target_temp: f64,
) -> &'static str {
    match state {
        LiquidState::Empty => "Empty",
        LiquidState::Filling => "Filling",
        _ => {
            let diff = (current_temp - target_temp).abs();
            if diff < 1.0 {
                "At temp"
            } else if current_temp < target_temp {
                "Heating"
            } else {
                "Cooling"
            }
        }
    }
}

// ─── Battery ─────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub fn format_battery_level(level: u8) -> String {
    format!("{}%", level)
}

pub fn get_battery_bar(level: u8, total_segments: usize) -> (usize, usize) {
    let filled = ((level as f64 / 100.0) * total_segments as f64).round() as usize;
    let empty = total_segments.saturating_sub(filled);
    (filled, empty)
}

// ─── Duration / Time estimation ──────────────────────────────────────────────

pub fn format_duration(minutes: f64) -> String {
    if minutes < 1.0 {
        return "< 1 min".to_string();
    }
    if minutes < 60.0 {
        return format!("{} min", minutes.round() as u64);
    }
    let hours = (minutes / 60.0).floor() as u64;
    let mins = (minutes % 60.0).round() as u64;
    if mins == 0 {
        format!("{}h", hours)
    } else {
        format!("{}h {}m", hours, mins)
    }
}

pub fn estimate_time_to_target_temp(
    current_temp: f64,
    target_temp: f64,
    liquid_state: LiquidState,
    dynamic_rate: Option<f64>,
) -> Option<f64> {
    if liquid_state == LiquidState::Empty {
        return None;
    }
    let temp_diff = (target_temp - current_temp).abs();
    if temp_diff < 0.5 {
        return Some(0.0);
    }
    if let Some(rate) = dynamic_rate {
        if rate.abs() > 0.05 {
            let is_heating = current_temp < target_temp;
            let is_rate_heating = rate > 0.0;
            if is_heating == is_rate_heating {
                return Some(temp_diff / rate.abs());
            }
        }
    }
    let rate_per_minute = if current_temp < target_temp { 1.0 } else { 0.5 };
    Some(temp_diff / rate_per_minute)
}

pub fn estimate_battery_life(
    battery_level: u8,
    is_charging: bool,
    liquid_state: LiquidState,
    dynamic_rate: Option<f64>,
) -> Option<f64> {
    let level = battery_level as f64;
    if is_charging {
        if let Some(rate) = dynamic_rate {
            if rate > 0.1 {
                let remaining = 100.0 - level;
                return if remaining <= 0.0 { Some(0.0) } else { Some(remaining / rate) };
            }
        }
        let remaining = 100.0 - level;
        return if remaining <= 0.0 { Some(0.0) } else { Some(remaining / BATTERY_CHARGE_RATE) };
    }
    if battery_level == 0 {
        return Some(0.0);
    }
    if let Some(rate) = dynamic_rate {
        if rate < -0.01 {
            return Some(level / rate.abs());
        }
    }
    let drain_rate = match liquid_state {
        LiquidState::Heating => BATTERY_DRAIN_RATE_HEATING,
        LiquidState::StableTemperature | LiquidState::Cooling => BATTERY_DRAIN_RATE_MAINTAINING,
        _ => 0.1,
    };
    Some(level / drain_rate)
}

// ─── Rate calculation (EMA inputs) ───────────────────────────────────────────

/// Calculates rate of change (value/minute) over a rolling window.
pub fn calculate_rate(history: &VecDeque<(f64, Instant)>, window_ms: u64) -> f64 {
    if history.len() < 2 {
        return 0.0;
    }
    let now = Instant::now();
    let window = std::time::Duration::from_millis(window_ms);
    let window_start = now.checked_sub(window).unwrap_or(now);

    let items: Vec<_> = history.iter().filter(|(_, t)| *t >= window_start).collect();

    if items.len() < 2 {
        // Fall back to the last two points if they're recent enough
        let last = history.back().unwrap();
        let second_last = &history[history.len() - 2];
        if now.duration_since(last.1).as_millis() as u64 > window_ms {
            return 0.0;
        }
        let val_diff = last.0 - second_last.0;
        let time_diff_min = last.1.duration_since(second_last.1).as_secs_f64() / 60.0;
        return if time_diff_min > 0.0 { val_diff / time_diff_min } else { 0.0 };
    }

    let first = items.first().unwrap();
    let last = items.last().unwrap();
    let val_diff = last.0 - first.0;
    let time_diff_min = last.1.duration_since(first.1).as_secs_f64() / 60.0;
    if time_diff_min > 0.0 { val_diff / time_diff_min } else { 0.0 }
}

pub fn apply_ema(prev: f64, new_raw: f64, alpha: f64) -> f64 {
    if prev == 0.0 && new_raw != 0.0 {
        return new_raw; // Initialise from first real value
    }
    alpha * new_raw + (1.0 - alpha) * prev
}

// ─── Colors ──────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}
