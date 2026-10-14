use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::bluetooth::{BleCommand, BleEvent};
use crate::settings::{load_settings, save_settings, Settings};
use crate::theme::{find_led_preset_index, get_theme, Theme, LED_PRESETS};
use crate::types::{LiquidState, MugState, Preset, RgbColor, TemperatureUnit};
use crate::utils::{
    apply_ema, calculate_rate, celsius_to_fahrenheit, clamp_temperature, fahrenheit_to_celsius,
};

// ─── View / Control state ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Main,
    Settings,
    Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingState {
    Idle,
    Setting,
    Retrying,
    Error,
}

// ─── App ──────────────────────────────────────────────────────────────────────

const SMOOTHING_ALPHA: f64 = 0.25;
const DEBOUNCE_MS: u64 = 1000;
const RETRY_TIMEOUT_MS: u64 = 5000;
const MAX_RETRIES: u32 = 1;

pub struct App {
    // BLE channels
    pub cmd_tx: mpsc::Sender<BleCommand>,

    // Mug & UI state
    pub mug_state: MugState,
    pub is_scanning: bool,
    pub error: Option<String>,
    pub found_mug_name: Option<String>,

    // View
    pub view_mode: ViewMode,

    // Temperature control
    pub desired_temp: f64,        // What the user currently wants (Celsius)
    pub temp_loading: LoadingState,
    pub temp_retry_count: u32,
    pub temp_setting_since: Option<Instant>,

    // Temperature debounce
    pub temp_pending: Option<(f64, Instant)>, // (celsius, when_user_last_changed)

    // LED color control
    pub desired_led_color: RgbColor,
    pub led_loading: LoadingState,
    pub led_retry_count: u32,
    pub led_setting_since: Option<Instant>,
    pub led_pending: Option<(RgbColor, Instant)>,

    // Settings / presets
    pub settings: Settings,
    pub selected_preset_index: i64,   // -1 = none
    pub settings_selected: usize,     // index in settings view

    // Repair
    pub repair_step: usize,
    pub repair_step_since: Instant,

    // Rates (EMA smoothed, degrees/min or %/min)
    pub temp_rate: f64,
    pub battery_rate: f64,
    pub temp_history: VecDeque<(f64, Instant)>,
    pub battery_history: VecDeque<(f64, Instant)>,

    // Animation
    pub tick_count: u64, // incremented on every UI tick
}

impl App {
    pub fn new(cmd_tx: mpsc::Sender<BleCommand>) -> Self {
        let settings = load_settings();
        let desired_temp = settings.last_target_temp;
        let desired_led_color = settings.led_color;

        Self {
            cmd_tx,
            mug_state: MugState::default(),
            is_scanning: false,
            error: None,
            found_mug_name: None,
            view_mode: ViewMode::Main,
            desired_temp,
            temp_loading: LoadingState::Idle,
            temp_retry_count: 0,
            temp_setting_since: None,
            temp_pending: None,
            desired_led_color,
            led_loading: LoadingState::Idle,
            led_retry_count: 0,
            led_setting_since: None,
            led_pending: None,
            settings,
            selected_preset_index: -1,
            settings_selected: 0,
            repair_step: 0,
            repair_step_since: Instant::now(),
            temp_rate: 0.0,
            battery_rate: 0.0,
            temp_history: VecDeque::new(),
            battery_history: VecDeque::new(),
            tick_count: 0,
        }
    }

    // ── Theme ────────────────────────────────────────────────────────────────

    pub fn theme(&self) -> Theme {
        get_theme(self.mug_state.liquid_state, self.mug_state.connected)
    }

    // ── Local temp unit (settings take priority) ──────────────────────────────

    pub fn temp_unit(&self) -> TemperatureUnit {
        self.settings.temperature_unit
    }

    // ── BLE events from background task ──────────────────────────────────────

    pub fn handle_ble_event(&mut self, event: BleEvent) {
        match event {
            BleEvent::Scanning(s) => {
                self.is_scanning = s;
                if s { self.error = None; }
            }
            BleEvent::MugFound(name) => {
                self.found_mug_name = Some(name);
            }
            BleEvent::Connected => {
                self.error = None;
                self.found_mug_name = None;
                // Push desired temp unit to mug
                let unit = self.settings.temperature_unit;
                let _ = self.cmd_tx.try_send(BleCommand::SetTemperatureUnit(unit));
            }
            BleEvent::Disconnected => {
                self.found_mug_name = None;
                self.temp_history.clear();
                self.battery_history.clear();
                self.temp_rate = 0.0;
                self.battery_rate = 0.0;
                self.temp_loading = LoadingState::Idle;
                self.temp_setting_since = None;
                self.led_loading = LoadingState::Idle;
                self.led_setting_since = None;
            }
            BleEvent::StateChange(new_state) => {
                let now = Instant::now();

                // Update histories
                if new_state.connected && new_state.liquid_state != LiquidState::Empty {
                    let last = self.temp_history.back().copied();
                    if last.map(|(v, _)| v != new_state.current_temp).unwrap_or(true) {
                        self.temp_history.push_back((new_state.current_temp, now));
                        while self.temp_history.front()
                            .map(|(_, t)| now.duration_since(*t) > Duration::from_secs(300))
                            .unwrap_or(false)
                        {
                            self.temp_history.pop_front();
                        }
                        let raw = calculate_rate(&self.temp_history, 2 * 60 * 1000);
                        self.temp_rate = apply_ema(self.temp_rate, raw, SMOOTHING_ALPHA);
                    }
                } else if !new_state.connected {
                    self.temp_history.clear();
                    self.temp_rate = 0.0;
                }

                if new_state.connected {
                    let last = self.battery_history.back().copied();
                    if last.map(|(v, _)| v != new_state.battery_level as f64).unwrap_or(true) {
                        self.battery_history.push_back((new_state.battery_level as f64, now));
                        while self.battery_history.front()
                            .map(|(_, t)| now.duration_since(*t) > Duration::from_secs(600))
                            .unwrap_or(false)
                        {
                            self.battery_history.pop_front();
                        }
                        let raw = calculate_rate(&self.battery_history, 5 * 60 * 1000);
                        self.battery_rate = apply_ema(self.battery_rate, raw, SMOOTHING_ALPHA);
                    }
                }

                // Check if pending temp set has completed
                if self.temp_loading != LoadingState::Idle {
                    if (new_state.target_temp - self.desired_temp).abs() < 0.1 {
                        self.temp_loading = LoadingState::Idle;
                        self.temp_retry_count = 0;
                        self.temp_setting_since = None;
                    }
                }

                // Check if pending LED color set has completed
                if self.led_loading != LoadingState::Idle {
                    if new_state.color == self.desired_led_color {
                        self.led_loading = LoadingState::Idle;
                        self.led_retry_count = 0;
                        self.led_setting_since = None;
                    }
                }

                // Sync desired_temp only when not actively setting
                if self.temp_loading == LoadingState::Idle && self.temp_pending.is_none() {
                    self.desired_temp = new_state.target_temp;
                }
                // Sync desired LED color only when idle
                if self.led_loading == LoadingState::Idle && self.led_pending.is_none() {
                    self.desired_led_color = new_state.color;
                }

                // Auto-select preset
                self.selected_preset_index = self.settings.presets.iter().position(|p| {
                    (p.temperature - new_state.target_temp).abs() < 0.1
                }).map(|i| i as i64).unwrap_or(-1);

                self.mug_state = new_state;
            }
            BleEvent::Error(msg) => {
                self.error = Some(msg);
                self.is_scanning = false;
                self.found_mug_name = None;
            }
        }
    }

    // ── Tick (called on UI timer, ~100ms) ─────────────────────────────────────

    pub fn tick(&mut self) {
        self.tick_count += 1;
        let now = Instant::now();

        // Check temperature debounce
        if let Some((temp, since)) = self.temp_pending {
            if now.duration_since(since) >= Duration::from_millis(DEBOUNCE_MS) {
                self.temp_pending = None;
                self.temp_loading = LoadingState::Setting;
                self.temp_retry_count = 0;
                self.temp_setting_since = Some(now);
                let _ = self.cmd_tx.try_send(BleCommand::SetTargetTemp(temp));
            }
        }

        // Check temperature timeout / retry
        if self.temp_loading == LoadingState::Setting || self.temp_loading == LoadingState::Retrying {
            if let Some(since) = self.temp_setting_since {
                if now.duration_since(since) >= Duration::from_millis(RETRY_TIMEOUT_MS) {
                    if self.temp_retry_count < MAX_RETRIES {
                        self.temp_retry_count += 1;
                        self.temp_loading = LoadingState::Retrying;
                        self.temp_setting_since = Some(now);
                        let _ = self.cmd_tx.try_send(BleCommand::SetTargetTemp(self.desired_temp));
                    } else {
                        self.temp_loading = LoadingState::Error;
                        self.temp_setting_since = None;
                        self.desired_temp = self.mug_state.target_temp;
                    }
                }
            }
        }

        // Clear temp error after 3s
        if self.temp_loading == LoadingState::Error {
            if let Some(since) = self.temp_setting_since {
                if now.duration_since(since) >= Duration::from_secs(3) {
                    self.temp_loading = LoadingState::Idle;
                    self.temp_setting_since = None;
                }
            } else {
                // Make sure we have a timer
                self.temp_setting_since = Some(now);
            }
        }

        // Check LED debounce
        if let Some((color, since)) = self.led_pending {
            if now.duration_since(since) >= Duration::from_millis(DEBOUNCE_MS) {
                self.led_pending = None;
                self.led_loading = LoadingState::Setting;
                self.led_retry_count = 0;
                self.led_setting_since = Some(now);
                let _ = self.cmd_tx.try_send(BleCommand::SetLedColor(color));
            }
        }

        // Check LED timeout / retry
        if self.led_loading == LoadingState::Setting || self.led_loading == LoadingState::Retrying {
            if let Some(since) = self.led_setting_since {
                if now.duration_since(since) >= Duration::from_millis(RETRY_TIMEOUT_MS) {
                    if self.led_retry_count < MAX_RETRIES {
                        self.led_retry_count += 1;
                        self.led_loading = LoadingState::Retrying;
                        self.led_setting_since = Some(now);
                        let _ = self.cmd_tx.try_send(BleCommand::SetLedColor(self.desired_led_color));
                    } else {
                        self.led_loading = LoadingState::Error;
                        self.led_setting_since = None;
                        self.desired_led_color = self.mug_state.color;
                    }
                }
            }
        }

        // Clear LED error after 3s
        if self.led_loading == LoadingState::Error {
            if let Some(since) = self.led_setting_since {
                if now.duration_since(since) >= Duration::from_secs(3) {
                    self.led_loading = LoadingState::Idle;
                    self.led_setting_since = None;
                }
            } else {
                self.led_setting_since = Some(now);
            }
        }

        // Auto-advance repair steps (every 2s)
        if self.view_mode == ViewMode::Repair && self.repair_step < 2 {
            if now.duration_since(self.repair_step_since) >= Duration::from_secs(2) {
                self.repair_step += 1;
                self.repair_step_since = now;
            }
        }
    }

    // ── Keyboard input ────────────────────────────────────────────────────────

    /// Returns true if the app should quit.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Ctrl-C always quits
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            return true;
        }

        match self.view_mode {
            ViewMode::Repair => self.handle_key_repair(key),
            ViewMode::Settings => self.handle_key_settings(key),
            ViewMode::Main => self.handle_key_main(key),
        }
    }

    fn handle_key_main(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Char('q') => return true,

            KeyCode::Esc => {
                if self.view_mode != ViewMode::Main {
                    self.view_mode = ViewMode::Main;
                }
            }

            KeyCode::Char('s') if !self.mug_state.connected && !self.is_scanning => {
                self.start_scanning();
            }

            KeyCode::Char('r') if !self.mug_state.connected => {
                self.start_scanning();
            }

            KeyCode::Char('r') if self.mug_state.connected => {
                self.start_repair();
            }

            KeyCode::Char('o') if self.mug_state.connected => {
                self.view_mode = ViewMode::Settings;
                self.settings_selected = 0;
            }

            KeyCode::Char('u') if self.mug_state.connected => {
                let new_unit = if self.settings.temperature_unit == TemperatureUnit::Celsius {
                    TemperatureUnit::Fahrenheit
                } else {
                    TemperatureUnit::Celsius
                };
                self.set_temperature_unit(new_unit);
            }

            // Preset shortcuts 1–9
            KeyCode::Char(c) if ('1'..='9').contains(&c) => {
                let idx = (c as usize) - ('1' as usize);
                if let Some(preset) = self.settings.presets.get(idx).cloned() {
                    self.apply_preset(&preset);
                }
            }

            // Temperature adjustment with left/right arrows
            KeyCode::Left if self.mug_state.connected => {
                self.adjust_temp(-1);
            }
            KeyCode::Right if self.mug_state.connected => {
                self.adjust_temp(1);
            }

            _ => {}
        }
        false
    }

    fn handle_key_settings(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        let total = 2 + self.settings.presets.len();

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.view_mode = ViewMode::Main;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.settings_selected > 0 {
                    self.settings_selected -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.settings_selected < total - 1 {
                    self.settings_selected += 1;
                }
            }
            KeyCode::Left | KeyCode::Char('h') => self.settings_adjust(-1),
            KeyCode::Right | KeyCode::Char('l') => self.settings_adjust(1),
            _ => {}
        }
        false
    }

    fn handle_key_repair(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        if self.repair_step == 2 {
            match key.code {
                KeyCode::Char('c') => {
                    self.view_mode = ViewMode::Main;
                    self.repair_step = 0;
                    self.start_scanning();
                }
                KeyCode::Esc => {
                    self.view_mode = ViewMode::Main;
                    self.repair_step = 0;
                }
                _ => {}
            }
        }
        false
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    pub fn start_scanning(&mut self) {
        self.error = None;
        self.found_mug_name = None;
        let _ = self.cmd_tx.try_send(BleCommand::StartScanning);
    }

    fn start_repair(&mut self) {
        let _ = self.cmd_tx.try_send(BleCommand::ForgetAndRepair);
        self.view_mode = ViewMode::Repair;
        self.repair_step = 0;
        self.repair_step_since = Instant::now();
    }

    fn set_temperature_unit(&mut self, unit: TemperatureUnit) {
        self.settings.temperature_unit = unit;
        let _ = save_settings(&self.settings);
        if self.mug_state.connected {
            let _ = self.cmd_tx.try_send(BleCommand::SetTemperatureUnit(unit));
        }
    }

    fn apply_preset(&mut self, preset: &Preset) {
        if !self.mug_state.connected { return; }
        let temp = preset.temperature;
        self.desired_temp = temp;
        self.settings.last_target_temp = temp;
        let _ = save_settings(&self.settings);
        self.temp_pending = Some((temp, Instant::now() - Duration::from_millis(DEBOUNCE_MS)));
        // Flush debounce immediately
        self.tick();
    }

    fn adjust_temp(&mut self, direction: i32) {
        let unit = self.settings.temperature_unit;
        let step = if unit == TemperatureUnit::Fahrenheit { 1.0 } else { 0.5 };

        let current_display = if unit == TemperatureUnit::Fahrenheit {
            celsius_to_fahrenheit(self.desired_temp)
        } else {
            self.desired_temp
        };
        let rounded = (current_display / step).round() * step;
        let new_display = rounded + direction as f64 * step;

        let new_celsius = if unit == TemperatureUnit::Fahrenheit {
            fahrenheit_to_celsius(new_display)
        } else {
            new_display
        };

        let clamped = clamp_temperature(new_celsius);
        self.desired_temp = clamped;
        self.temp_pending = Some((clamped, Instant::now()));
    }

    fn settings_adjust(&mut self, direction: i32) {
        let idx = self.settings_selected;
        if idx == 0 {
            // Toggle temp unit
            let new_unit = if self.settings.temperature_unit == TemperatureUnit::Celsius {
                TemperatureUnit::Fahrenheit
            } else {
                TemperatureUnit::Celsius
            };
            self.set_temperature_unit(new_unit);
        } else if idx == 1 {
            // LED color cycle
            let current = find_led_preset_index(&self.desired_led_color);
            let len = LED_PRESETS.len();
            let next = match current {
                None => 0,
                Some(i) => {
                    if direction > 0 { (i + 1) % len } else { (i + len - 1) % len }
                }
            };
            let new_color = LED_PRESETS[next].color;
            self.desired_led_color = new_color;
            self.led_pending = Some((new_color, Instant::now()));
        } else {
            // Edit preset temperature
            let preset_idx = idx - 2;
            if let Some(preset) = self.settings.presets.get(preset_idx).cloned() {
                let unit = self.settings.temperature_unit;
                let step = if unit == TemperatureUnit::Fahrenheit { 1.0 } else { 0.5 };
                let current_display = if unit == TemperatureUnit::Fahrenheit {
                    celsius_to_fahrenheit(preset.temperature)
                } else {
                    preset.temperature
                };
                let rounded = (current_display / step).round() * step;
                let new_display = rounded + direction as f64 * step;
                let new_celsius = if unit == TemperatureUnit::Fahrenheit {
                    fahrenheit_to_celsius(new_display)
                } else {
                    new_display
                };
                self.settings.presets[preset_idx].temperature = new_celsius;
                let _ = save_settings(&self.settings);
            }
        }
    }
}
