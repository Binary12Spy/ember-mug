mod app;
mod bluetooth;
mod mock;
mod settings;
mod theme;
mod types;
mod ui;
mod utils;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use std::time::Duration;
use tokio::time::interval;

use app::App;
use bluetooth::spawn_ble_task;
use mock::{is_mock_mode, spawn_mock_task};

// ─── Entry point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Set up logging (enabled via RUST_LOG=debug)
    env_logger::init();

    // Set up terminal
    let mut terminal = ratatui::init();
    terminal.clear()?;

    let result = run(&mut terminal).await;

    // Always restore terminal on exit
    ratatui::restore();

    result
}

// ─── Main event loop ──────────────────────────────────────────────────────────

async fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    // Start the BLE background task (real or mock)
    let (cmd_tx, mut ble_rx) = if is_mock_mode() {
        log::info!("Mock mode enabled (EMBER_MOCK)");
        spawn_mock_task()
    } else {
        spawn_ble_task()
    };

    let mut app = App::new(cmd_tx);

    // Start scanning immediately on launch
    app.start_scanning();

    // UI tick (100ms = 10 fps)
    let mut tick_timer = interval(Duration::from_millis(100));
    tick_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Crossterm async event stream
    let mut event_stream = EventStream::new();

    loop {
        // Draw the current frame
        terminal.draw(|frame| ui::draw(frame, &app))?;

        tokio::select! {
            // ── UI timer tick ─────────────────────────────────────────────────
            _ = tick_timer.tick() => {
                app.tick();
            }

            // ── Terminal keyboard / resize events ─────────────────────────────
            maybe_event = event_stream.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        if app.handle_key(key) {
                            break; // quit
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => {
                        // ratatui handles resize automatically on next draw
                    }
                    None => break, // stream ended (terminal closed)
                    _ => {}
                }
            }

            // ── BLE events from background task ───────────────────────────────
            ble_event = ble_rx.recv() => {
                match ble_event {
                    Some(event) => app.handle_ble_event(event),
                    None => break, // BLE task shut down
                }
            }
        }
    }

    Ok(())
}
