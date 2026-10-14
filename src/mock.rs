//! Mock BLE task — simulates a connected Ember Mug in software.
//!
//! Activated with `EMBER_MOCK=1` (or `=true`).  Exposes the exact same
//! `(mpsc::Sender<BleCommand>, mpsc::Receiver<BleEvent>)` pair as the real
//! `spawn_ble_task`, so the rest of the app is completely unaware.

use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::bluetooth::{BleCommand, BleEvent};
use crate::types::{
    LiquidState, MugState, RgbColor, TemperatureUnit, BATTERY_CHARGE_RATE,
    BATTERY_DRAIN_RATE_HEATING, BATTERY_DRAIN_RATE_MAINTAINING, MAX_TEMP_CELSIUS, MIN_TEMP_CELSIUS,
};

// ─── Public API (identical signature to spawn_ble_task) ───────────────────────

pub fn spawn_mock_task() -> (mpsc::Sender<BleCommand>, mpsc::Receiver<BleEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<BleCommand>(32);
    let (event_tx, event_rx) = mpsc::channel::<BleEvent>(64);

    tokio::spawn(run_mock(cmd_rx, event_tx));

    (cmd_tx, event_rx)
}

pub fn is_mock_mode() -> bool {
    matches!(
        std::env::var("EMBER_MOCK").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

// ─── Simulation constants ─────────────────────────────────────────────────────

const SCAN_DELAY_MS: u64 = 1_500;
const CONNECT_DELAY_MS: u64 = 800;
const UPDATE_INTERVAL_MS: u64 = 500; // simulation tick (2 Hz)
const TEMP_CHANGE_RATE: f64 = 0.5;   // °C per second

const INITIAL_BATTERY: f64 = 75.0;
const INITIAL_CURRENT_TEMP: f64 = 45.0;
const INITIAL_TARGET_TEMP: f64 = 55.0;
const MOCK_MUG_NAME: &str = "Mock Ember Mug";

// ─── Task ─────────────────────────────────────────────────────────────────────

async fn run_mock(
    mut cmd_rx: mpsc::Receiver<BleCommand>,
    event_tx: mpsc::Sender<BleEvent>,
) {
    // ── Phase 1: simulate scan ────────────────────────────────────────────────
    let _ = event_tx.send(BleEvent::Scanning(true)).await;
    tokio::time::sleep(Duration::from_millis(SCAN_DELAY_MS)).await;
    let _ = event_tx.send(BleEvent::MugFound(MOCK_MUG_NAME.to_string())).await;
    let _ = event_tx.send(BleEvent::Scanning(false)).await;

    // ── Phase 2: simulate connect ─────────────────────────────────────────────
    tokio::time::sleep(Duration::from_millis(CONNECT_DELAY_MS)).await;

    let mut state = MugState {
        connected: true,
        battery_level: INITIAL_BATTERY as u8,
        is_charging: false,
        current_temp: INITIAL_CURRENT_TEMP,
        target_temp: INITIAL_TARGET_TEMP,
        liquid_state: LiquidState::Cooling,
        temperature_unit: TemperatureUnit::Celsius,
        color: RgbColor { r: 255, g: 147, b: 41, a: 255 },
        mug_name: MOCK_MUG_NAME.to_string(),
    };

    // Use f64 internally for smooth simulation, quantise before emitting
    let mut battery_f: f64 = INITIAL_BATTERY;

    let _ = event_tx.send(BleEvent::Connected).await;
    let _ = event_tx.send(BleEvent::StateChange(state.clone())).await;

    // ── Phase 3: simulation loop ──────────────────────────────────────────────
    let mut sim_ticker = interval(Duration::from_millis(UPDATE_INTERVAL_MS));
    sim_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_tick = Instant::now();

    loop {
        tokio::select! {
            // Commands from the UI
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                let changed = handle_command(cmd, &mut state, &mut battery_f);
                if changed {
                    let _ = event_tx.send(BleEvent::StateChange(state.clone())).await;
                }
            }

            // Simulation tick
            _ = sim_ticker.tick() => {
                let now = Instant::now();
                let delta_s = now.duration_since(last_tick).as_secs_f64();
                last_tick = now;

                if simulate_tick(&mut state, &mut battery_f, delta_s) {
                    let _ = event_tx.send(BleEvent::StateChange(state.clone())).await;
                }
            }
        }
    }
}

// ─── Command handler ──────────────────────────────────────────────────────────

/// Returns true if state changed and should be emitted.
fn handle_command(cmd: BleCommand, state: &mut MugState, _battery_f: &mut f64) -> bool {
    match cmd {
        BleCommand::SetTargetTemp(temp) => {
            let clamped = temp.max(MIN_TEMP_CELSIUS).min(MAX_TEMP_CELSIUS);
            state.target_temp = clamped;
            // Immediately update liquid state direction
            update_liquid_state(state);
            true
        }
        BleCommand::SetTemperatureUnit(unit) => {
            state.temperature_unit = unit;
            true
        }
        BleCommand::SetLedColor(color) => {
            state.color = color;
            true
        }
        BleCommand::Disconnect | BleCommand::ForgetAndRepair => {
            state.connected = false;
            true
        }
        // StartScanning while connected is a no-op in mock mode
        BleCommand::StartScanning => false,
    }
}

// ─── Physics simulation ───────────────────────────────────────────────────────

/// Advance the simulation by `delta_s` seconds.  Returns true if anything changed.
fn simulate_tick(state: &mut MugState, battery_f: &mut f64, delta_s: f64) -> bool {
    if !state.connected { return false; }

    let mut changed = false;

    // ── Temperature ───────────────────────────────────────────────────────────
    if state.liquid_state != LiquidState::Empty {
        let diff = state.target_temp - state.current_temp;
        if diff.abs() > 0.05 {
            let max_change = TEMP_CHANGE_RATE * delta_s;
            let change = diff.signum() * diff.abs().min(max_change);
            state.current_temp = (state.current_temp + change * 100.0).round() / 100.0;
            update_liquid_state(state);
            changed = true;
        } else if state.liquid_state != LiquidState::StableTemperature {
            state.current_temp = state.target_temp;
            state.liquid_state = LiquidState::StableTemperature;
            changed = true;
        }
    }

    // ── Battery ───────────────────────────────────────────────────────────────
    let battery_delta = if state.is_charging {
        (BATTERY_CHARGE_RATE / 60.0) * delta_s
    } else {
        let drain = match state.liquid_state {
            LiquidState::Heating => BATTERY_DRAIN_RATE_HEATING,
            LiquidState::StableTemperature | LiquidState::Cooling => BATTERY_DRAIN_RATE_MAINTAINING,
            _ => 0.05,
        };
        -(drain / 60.0) * delta_s
    };

    let new_battery = (*battery_f + battery_delta).clamp(0.0, 100.0);
    if (new_battery - *battery_f).abs() > 0.1 {
        *battery_f = new_battery;
        state.battery_level = new_battery.round() as u8;
        changed = true;
    }

    changed
}

fn update_liquid_state(state: &mut MugState) {
    if state.liquid_state == LiquidState::Empty || state.liquid_state == LiquidState::Filling {
        return;
    }
    let diff = state.target_temp - state.current_temp;
    state.liquid_state = if diff > 0.5 {
        LiquidState::Heating
    } else if diff < -0.5 {
        LiquidState::Cooling
    } else {
        LiquidState::StableTemperature
    };
}
