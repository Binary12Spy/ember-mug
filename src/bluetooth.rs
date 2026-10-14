use anyhow::{anyhow, Result};
use btleplug::api::{
    Central, CentralEvent, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use futures::StreamExt;
use log::debug;
use rand::RngCore;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::types::{
    chars, LiquidState, MugState, RgbColor, TemperatureUnit, MAX_TEMP_CELSIUS,
    MIN_TEMP_CELSIUS,
};

// ─── Public channel messages ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum BleEvent {
    Scanning(bool),
    MugFound(String),
    Connected,
    Disconnected,
    StateChange(MugState),
    Error(String),
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum BleCommand {
    StartScanning,
    SetTargetTemp(f64),
    SetTemperatureUnit(TemperatureUnit),
    SetLedColor(RgbColor),
    Disconnect,
    ForgetAndRepair,
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn parse_uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).expect("Invalid UUID constant")
}

fn u16le_to_temp(data: &[u8]) -> f64 {
    let raw = u16::from_le_bytes([data[0], data[1]]);
    raw as f64 * 0.01
}

fn temp_to_u16le(temp_c: f64) -> [u8; 2] {
    let clamped = temp_c.max(MIN_TEMP_CELSIUS).min(MAX_TEMP_CELSIUS);
    let raw = (clamped * 100.0).round() as u16;
    raw.to_le_bytes()
}

// ─── BLE task ────────────────────────────────────────────────────────────────

/// Spawn the background BLE task.  Returns a (cmd_tx, event_rx) pair for the UI to use.
pub fn spawn_ble_task() -> (mpsc::Sender<BleCommand>, mpsc::Receiver<BleEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<BleCommand>(32);
    let (event_tx, event_rx) = mpsc::channel::<BleEvent>(64);

    tokio::spawn(async move {
        run_ble(cmd_rx, event_tx).await;
    });

    (cmd_tx, event_rx)
}

/// Top-level BLE async task.
async fn run_ble(mut cmd_rx: mpsc::Receiver<BleCommand>, event_tx: mpsc::Sender<BleEvent>) {
    // Obtain adapter
    let manager = match Manager::new().await {
        Ok(m) => m,
        Err(e) => {
            let _ = event_tx.send(BleEvent::Error(format!("Bluetooth manager error: {e}"))).await;
            return;
        }
    };
    let adapters = match manager.adapters().await {
        Ok(a) if !a.is_empty() => a,
        Ok(_) => {
            let _ = event_tx.send(BleEvent::Error("No Bluetooth adapter found".to_string())).await;
            return;
        }
        Err(e) => {
            let _ = event_tx.send(BleEvent::Error(format!("Adapter error: {e}"))).await;
            return;
        }
    };
    let adapter = adapters.into_iter().next().unwrap();
    debug!("BLE adapter ready");

    let mut mug_state = MugState::default();
    // Currently connected peripheral + its characteristics map
    let mut connection: Option<(Peripheral, HashMap<Uuid, btleplug::api::Characteristic>)> = None;
    // Notification channel (populated after connect)
    let (notif_tx, mut notif_rx) = mpsc::channel::<btleplug::api::ValueNotification>(64);

    let mut poll_ticker = tokio::time::interval(Duration::from_secs(2));
    poll_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Central events stream (device discovered / connected / disconnected)
    let mut central_events = match adapter.events().await {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(BleEvent::Error(format!("Central events error: {e}"))).await;
            return;
        }
    };

    loop {
        tokio::select! {
            // ── Commands from UI ──────────────────────────────────────────────
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    BleCommand::StartScanning => {
                        debug!("StartScanning command received");
                        let _ = event_tx.send(BleEvent::Scanning(true)).await;
                        if let Err(e) = adapter.start_scan(ScanFilter::default()).await {
                            let _ = event_tx.send(BleEvent::Error(format!("Scan error: {e}"))).await;
                        }
                    }

                    BleCommand::SetTargetTemp(temp) => {
                        if let Some((ref peripheral, ref char_map)) = connection {
                            let data = temp_to_u16le(temp);
                            if let Err(e) = write_char(peripheral, char_map, chars::TARGET_TEMP, &data).await {
                                let _ = event_tx.send(BleEvent::Error(format!("Set temp failed: {e}"))).await;
                            } else {
                                // Re-read to confirm
                                tokio::time::sleep(Duration::from_millis(200)).await;
                                if let Some(updated) = read_target_temp(peripheral, char_map).await {
                                    mug_state.target_temp = updated;
                                    let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                                }
                            }
                        }
                    }

                    BleCommand::SetTemperatureUnit(unit) => {
                        if let Some((ref peripheral, ref char_map)) = connection {
                            let data = [unit as u8];
                            if let Err(e) = write_char(peripheral, char_map, chars::TEMP_UNIT, &data).await {
                                let _ = event_tx.send(BleEvent::Error(format!("Set unit failed: {e}"))).await;
                            } else {
                                mug_state.temperature_unit = unit;
                                let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                            }
                        }
                    }

                    BleCommand::SetLedColor(color) => {
                        if let Some((ref peripheral, ref char_map)) = connection {
                            let data = [color.r, color.g, color.b, color.a];
                            if let Err(e) = write_char(peripheral, char_map, chars::LED_COLOR, &data).await {
                                let _ = event_tx.send(BleEvent::Error(format!("Set color failed: {e}"))).await;
                            } else {
                                tokio::time::sleep(Duration::from_millis(200)).await;
                                if let Some(updated) = read_led_color(peripheral, char_map).await {
                                    mug_state.color = updated;
                                    let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                                }
                            }
                        }
                    }

                    BleCommand::Disconnect => {
                        if let Some((ref peripheral, _)) = connection {
                            let _ = peripheral.disconnect().await;
                        }
                        connection = None;
                        mug_state.connected = false;
                        let _ = event_tx.send(BleEvent::Disconnected).await;
                        let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                    }

                    BleCommand::ForgetAndRepair => {
                        debug!("ForgetAndRepair: disconnecting and clearing peripheral");
                        if let Some((ref peripheral, _)) = connection {
                            let _ = peripheral.disconnect().await;
                        }
                        connection = None;
                        mug_state.connected = false;
                        let _ = event_tx.send(BleEvent::Disconnected).await;
                        let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                    }
                }
            }

            // ── Central events (device discovered / connected / disconnected) ─
            central_event = central_events.next() => {
                let Some(event) = central_event else { continue };
                match event {
                    CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => {
                        if connection.is_some() { continue; } // Already connected
                        let Ok(peripheral) = adapter.peripheral(&id).await else { continue };
                        let Ok(Some(props)) = peripheral.properties().await else { continue };
                        let name = props.local_name.unwrap_or_default();
                        if !name.to_lowercase().contains("ember") { continue; }

                        debug!("Found Ember mug: \"{name}\"");
                        let _ = event_tx.send(BleEvent::MugFound(name.clone())).await;
                        let _ = adapter.stop_scan().await;
                        let _ = event_tx.send(BleEvent::Scanning(false)).await;

                        // Wait for adapter to settle after stopping scan
                        tokio::time::sleep(Duration::from_millis(500)).await;

                        // Connect with retry
                        match connect_with_retry(
                            &peripheral,
                            &name,
                            &notif_tx,
                        ).await {
                            Ok((char_map, state)) => {
                                mug_state = state;
                                mug_state.connected = true;
                                mug_state.mug_name = name;
                                connection = Some((peripheral, char_map));
                                let _ = event_tx.send(BleEvent::Connected).await;
                                let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                            }
                            Err(e) => {
                                let _ = event_tx.send(BleEvent::Error(e.to_string())).await;
                            }
                        }
                    }

                    CentralEvent::DeviceDisconnected(id) => {
                        let is_our_mug = connection.as_ref()
                            .map(|(p, _)| p.id() == id)
                            .unwrap_or(false);
                        if is_our_mug {
                            debug!("Mug disconnected");
                            connection = None;
                            mug_state.connected = false;
                            let _ = event_tx.send(BleEvent::Disconnected).await;
                            let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                        }
                    }

                    _ => {}
                }
            }

            // ── Push notifications from mug ────────────────────────────────────
            notif = notif_rx.recv() => {
                let Some(notif) = notif else { continue };
                let Some((ref peripheral, ref char_map)) = connection else { continue };

                let event_type = notif.value.first().copied().unwrap_or(0);
                debug!("Push event type={event_type}");

                match event_type {
                    1 => { // Battery changed
                        if let Some((level, charging)) = read_battery(peripheral, char_map).await {
                            mug_state.battery_level = level;
                            mug_state.is_charging = charging;
                            let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                        }
                    }
                    2 => { mug_state.is_charging = true;  let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await; }
                    3 => { mug_state.is_charging = false; let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await; }
                    4 => { // Target temp changed
                        if let Some(t) = read_target_temp(peripheral, char_map).await {
                            mug_state.target_temp = t;
                            let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                        }
                    }
                    5 => { // Current temp changed
                        if let Some(t) = read_current_temp(peripheral, char_map).await {
                            mug_state.current_temp = t;
                            let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                        }
                    }
                    8 => { // Liquid state changed
                        if let Some(ls) = read_liquid_state(peripheral, char_map).await {
                            mug_state.liquid_state = ls;
                            let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
                        }
                    }
                    _ => {}
                }
            }

            // ── Polling tick ──────────────────────────────────────────────────
            _ = poll_ticker.tick() => {
                let Some((ref peripheral, ref char_map)) = connection else { continue };
                if !mug_state.connected { continue; }

                if let Some(t) = read_current_temp(peripheral, char_map).await {
                    mug_state.current_temp = t;
                }
                if let Some(ls) = read_liquid_state(peripheral, char_map).await {
                    mug_state.liquid_state = ls;
                }
                let _ = event_tx.send(BleEvent::StateChange(mug_state.clone())).await;
            }
        }
    }
}

// ─── Connection logic ─────────────────────────────────────────────────────────

/// Connect, discover characteristics, make writable, test writes, subscribe to
/// notifications, and read all initial values.  Returns the characteristic map
/// and the initial MugState (connected/mug_name set by the caller).
async fn connect_with_retry(
    peripheral: &Peripheral,
    name: &str,
    notif_tx: &mpsc::Sender<btleplug::api::ValueNotification>,
) -> Result<(HashMap<Uuid, btleplug::api::Characteristic>, MugState)> {
    const MAX_RETRIES: u32 = 3;
    let mut last_err = anyhow!("Unknown error");

    for attempt in 1..=MAX_RETRIES {
        debug!("Connection attempt {attempt}/{MAX_RETRIES} to \"{name}\"");
        match try_connect(peripheral, notif_tx).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_err = e;
                debug!("Attempt {attempt} failed: {last_err}");
                if attempt < MAX_RETRIES {
                    let delay = Duration::from_millis(1000 * 2u64.pow(attempt - 1));
                    debug!("Retrying in {}ms", delay.as_millis());
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(last_err)
}

async fn try_connect(
    peripheral: &Peripheral,
    notif_tx: &mpsc::Sender<btleplug::api::ValueNotification>,
) -> Result<(HashMap<Uuid, btleplug::api::Characteristic>, MugState)> {
    // Connect with timeout
    tokio::time::timeout(Duration::from_secs(10), peripheral.connect())
        .await
        .map_err(|_| anyhow!("Connection timed out – make sure your mug is nearby and awake"))?
        .map_err(|e| anyhow!("Connect failed: {e}"))?;

    // Discover services
    peripheral.discover_services().await
        .map_err(|e| anyhow!("Service discovery failed: {e}"))?;

    // Build characteristic map
    let char_map: HashMap<Uuid, _> = peripheral.characteristics()
        .into_iter()
        .map(|c| (c.uuid, c))
        .collect();

    debug!("Found {} characteristics", char_map.len());

    // Trigger pairing by reading DSK
    debug!("Reading DSK to trigger pairing…");
    read_raw(peripheral, &char_map, chars::DSK).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Make mug writable (UDSK dance)
    make_writable(peripheral, &char_map).await;

    // Test that writes actually work
    if !test_write_capability(peripheral, &char_map).await {
        return Err(anyhow!(
            "Your Ember mug does not allow write commands.\n\
             Please pair your mug with the official Ember app first, then reconnect.\n\
             See: https://apps.apple.com/app/ember-smart-mug/id1095189177"
        ));
    }

    // Subscribe to push events and spawn notification forwarder
    subscribe_push_events(peripheral, &char_map, notif_tx).await?;

    // Read initial values
    let state = read_all_values(peripheral, &char_map).await;

    Ok((char_map, state))
}

// ─── UDSK / write-capability dance ───────────────────────────────────────────

async fn make_writable(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) {
    // Check if UDSK is already set
    if let Some(data) = read_raw(peripheral, char_map, chars::UDSK).await {
        let all_zeros = data.iter().all(|&b| b == 0);
        if !all_zeros {
            debug!("UDSK already set – device should be writable");
            return;
        }
    }

    debug!("Writing fresh UDSK…");
    let mut udsk = vec![0u8; 20];
    rand::thread_rng().fill_bytes(&mut udsk);

    if let Err(e) = write_char(peripheral, char_map, chars::UDSK, &udsk).await {
        debug!("UDSK write failed: {e}");
    } else {
        debug!("UDSK written");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

async fn test_write_capability(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> bool {
    // Read current LED color, write a slightly different one, read back, restore
    let Some(orig_data) = read_raw(peripheral, char_map, chars::LED_COLOR).await else {
        return false;
    };
    if orig_data.len() < 4 { return false; }

    let orig = [orig_data[0], orig_data[1], orig_data[2], orig_data[3]];
    let test = [(orig[0].wrapping_add(10)), orig[1], orig[2], orig[3]];

    if let Err(e) = write_char(peripheral, char_map, chars::LED_COLOR, &test).await {
        debug!("Write test failed: {e}");
        return false;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    let verify = read_raw(peripheral, char_map, chars::LED_COLOR).await
        .filter(|d| d.len() >= 1);

    // Restore original
    let _ = write_char(peripheral, char_map, chars::LED_COLOR, &orig).await;

    match verify {
        Some(d) => {
            let passed = d[0] == test[0];
            debug!("Write test {}: expected r={}, got r={}", if passed { "PASSED" } else { "FAILED" }, test[0], d[0]);
            passed
        }
        None => false,
    }
}

// ─── Notifications ────────────────────────────────────────────────────────────

async fn subscribe_push_events(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
    notif_tx: &mpsc::Sender<btleplug::api::ValueNotification>,
) -> Result<()> {
    let uuid = parse_uuid(chars::PUSH_EVENTS);
    let Some(char) = char_map.get(&uuid) else {
        debug!("PUSH_EVENTS characteristic not found, skipping notifications");
        return Ok(());
    };
    peripheral.subscribe(char).await
        .map_err(|e| anyhow!("Subscribe failed: {e}"))?;

    let mut stream = peripheral.notifications().await
        .map_err(|e| anyhow!("Notification stream failed: {e}"))?;

    let tx = notif_tx.clone();
    tokio::spawn(async move {
        while let Some(notif) = stream.next().await {
            if tx.send(notif).await.is_err() {
                break;
            }
        }
    });

    Ok(())
}

// ─── Read helpers ─────────────────────────────────────────────────────────────

async fn read_raw(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
    uuid_str: &str,
) -> Option<Vec<u8>> {
    let uuid = parse_uuid(uuid_str);
    let char = char_map.get(&uuid)?;
    peripheral.read(char).await.ok()
}

async fn read_current_temp(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> Option<f64> {
    let data = read_raw(peripheral, char_map, chars::CURRENT_TEMP).await?;
    if data.len() >= 2 { Some(u16le_to_temp(&data)) } else { None }
}

async fn read_target_temp(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> Option<f64> {
    let data = read_raw(peripheral, char_map, chars::TARGET_TEMP).await?;
    if data.len() >= 2 { Some(u16le_to_temp(&data)) } else { None }
}

async fn read_battery(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> Option<(u8, bool)> {
    let data = read_raw(peripheral, char_map, chars::BATTERY).await?;
    if data.len() >= 2 { Some((data[0], data[1] == 1)) } else { None }
}

async fn read_liquid_state(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> Option<LiquidState> {
    let data = read_raw(peripheral, char_map, chars::LIQUID_STATE).await?;
    data.first().and_then(|&b| LiquidState::from_u8(b))
}

async fn read_led_color(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> Option<crate::types::RgbColor> {
    let data = read_raw(peripheral, char_map, chars::LED_COLOR).await?;
    if data.len() >= 4 {
        Some(crate::types::RgbColor { r: data[0], g: data[1], b: data[2], a: data[3] })
    } else { None }
}

async fn read_mug_name(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> Option<String> {
    let data = read_raw(peripheral, char_map, chars::MUG_NAME).await?;
    let s = String::from_utf8_lossy(&data).trim_matches('\0').trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

async fn read_all_values(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
) -> MugState {
    let mut state = MugState::default();

    if let Some(n) = read_mug_name(peripheral, char_map).await {
        state.mug_name = n;
    }
    if let Some(t) = read_current_temp(peripheral, char_map).await {
        state.current_temp = t;
    }
    if let Some(t) = read_target_temp(peripheral, char_map).await {
        state.target_temp = t;
    }
    if let Some((level, charging)) = read_battery(peripheral, char_map).await {
        state.battery_level = level;
        state.is_charging = charging;
    }
    if let Some(ls) = read_liquid_state(peripheral, char_map).await {
        state.liquid_state = ls;
    }
    if let Some(data) = read_raw(peripheral, char_map, chars::TEMP_UNIT).await {
        if let Some(unit) = data.first().and_then(|&b| TemperatureUnit::from_u8(b)) {
            state.temperature_unit = unit;
        }
    }
    if let Some(color) = read_led_color(peripheral, char_map).await {
        state.color = color;
    }

    state
}

// ─── Write helper ─────────────────────────────────────────────────────────────

async fn write_char(
    peripheral: &Peripheral,
    char_map: &HashMap<Uuid, btleplug::api::Characteristic>,
    uuid_str: &str,
    data: &[u8],
) -> Result<()> {
    let uuid = parse_uuid(uuid_str);
    let char = char_map.get(&uuid)
        .ok_or_else(|| anyhow!("Characteristic not found: {uuid_str}"))?;

    let write_type = if char.properties.contains(CharPropFlags::WRITE) {
        WriteType::WithResponse
    } else {
        WriteType::WithoutResponse
    };

    peripheral.write(char, data, write_type).await
        .map_err(|e| anyhow!("Write failed: {e}"))
}
