use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, LoadingState, ViewMode};
use crate::mock::is_mock_mode;
use crate::theme::{find_led_preset_index, Theme, LED_PRESETS};
use crate::types::{LiquidState, TemperatureUnit, MAX_TEMP_CELSIUS, MIN_TEMP_CELSIUS};
use crate::utils::{
    estimate_battery_life, estimate_time_to_target_temp, format_duration,
    format_temperature, get_battery_bar, get_liquid_state_text,
};

// ─── Spinner ─────────────────────────────────────────────────────────────────

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(tick: u64) -> &'static str {
    SPINNER[(tick / 2) as usize % SPINNER.len()]
}

// ─── Main draw entry-point ────────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let theme = app.theme();

    match app.view_mode {
        ViewMode::Repair => draw_repair(frame, area, app, theme),
        ViewMode::Settings => draw_settings(frame, area, app, theme),
        ViewMode::Main => draw_main(frame, area, app, theme),
    }
}

// ─── Layout helpers ───────────────────────────────────────────────────────────

/// Split `area` into [header(3), content(flex), help(3)] rows.
fn outer_layout(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // header
            Constraint::Min(0),      // content
            Constraint::Length(3),   // help bar
        ])
        .split(area);
    (chunks[0], chunks[1], chunks[2])
}

// ─── Main view ───────────────────────────────────────────────────────────────

fn draw_main(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let (header_area, content_area, help_area) = outer_layout(area);

    draw_header(frame, header_area, app, theme);
    draw_help(frame, help_area, app, theme);

    if !app.mug_state.connected {
        draw_connection_status(frame, content_area, app, theme);
        return;
    }

    let wide = area.width >= 90;
    if wide {
        draw_main_wide(frame, content_area, app, theme);
    } else {
        draw_main_narrow(frame, content_area, app, theme);
    }
}

fn draw_main_wide(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    // Left: coffee mug (24 wide), Right: 2x2 grid
    let mug_width = 24u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(mug_width),
            Constraint::Min(0),
        ])
        .split(area);

    draw_coffee_mug(frame, chunks[0], app, theme);

    // Right side: 2 rows
    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Row 1: Temperature | Battery
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(right_rows[0]);

    draw_temperature_display(frame, top_cols[0], app, theme);
    draw_battery_display(frame, top_cols[1], app, theme);

    // Row 2: Temp Control | Presets
    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(right_rows[1]);

    draw_temperature_control(frame, bot_cols[0], app, theme);
    draw_presets(frame, bot_cols[1], app, theme);
}

fn draw_main_narrow(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(18),  // coffee mug
            Constraint::Length(7),   // temperature
            Constraint::Length(7),   // battery
            Constraint::Length(6),   // temp control
            Constraint::Min(0),      // presets
        ])
        .split(area);

    draw_coffee_mug(frame, chunks[0], app, theme);
    draw_temperature_display(frame, chunks[1], app, theme);
    draw_battery_display(frame, chunks[2], app, theme);
    draw_temperature_control(frame, chunks[3], app, theme);
    draw_presets(frame, chunks[4], app, theme);
}

// ─── Header ───────────────────────────────────────────────────────────────────

fn draw_header(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let connected = app.mug_state.connected;
    let status = if connected {
        format!("MUG: {}", app.mug_state.mug_name)
    } else {
        "Disconnected".to_string()
    };

    // LED color dot
    let led = &app.desired_led_color;
    // Right-side content
    let right_text = if connected {
        format!("● {status}  ")
    } else {
        format!("{status}  ")
    };

    let right_color = if connected {
        Color::Rgb(led.r, led.g, led.b)
    } else {
        theme.dim_text
    };

    let right_spans: Line = Line::from(vec![
        Span::styled(right_text, Style::default().fg(right_color)),
    ]);

    // We render two paragraphs over the same row to simulate left/right alignment
    let title = if is_mock_mode() {
        " C[_] EMBER MUG  [MOCK] "
    } else {
        " C[_] EMBER MUG "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .border_type(BorderType::Double)
        .title(Span::styled(title, Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)));

    let status_para = Paragraph::new(right_spans)
        .block(block)
        .alignment(Alignment::Right);

    frame.render_widget(status_para, area);
}

// ─── Help bar ────────────────────────────────────────────────────────────────

fn draw_help(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let connected = app.mug_state.connected;

    let mut spans: Vec<Span> = vec![];

    let key = |k: &str, label: &str, t: Theme| {
        vec![
            Span::raw("  "),
            Span::styled(format!("[{k}]"), Style::default().fg(t.primary).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {label}"), Style::default().fg(t.dim_text)),
        ]
    };

    if connected {
        spans.extend(key("1-3", "presets", theme));
        spans.extend(key("←/→", "adjust temp", theme));
        spans.extend(key("u", "unit", theme));
        spans.extend(key("r", "repair", theme));
        spans.extend(key("o", "settings", theme));
    } else {
        if !app.is_scanning && app.error.is_none() {
            spans.extend(key("s", "scan", theme));
        }
        if app.error.is_some() {
            spans.extend(key("r", "retry", theme));
        }
    }
    spans.extend(key("q", "quit", theme));

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));

    let para = Paragraph::new(Line::from(spans))
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(para, area);
}

// ─── Coffee Mug ──────────────────────────────────────────────────────────────

const MUG_BASE: &str = concat!(
    "       {   }\n",
    "    {   }\n",
    "     }_{ __{\n",
    "  .-{   }   }-.\n",
    " (   }     {   )\n",
    " |`-.._____..-'|\n",
    " |             ;--.\n",
    " |            (__  \\\n",
    " |             | )  )\n",
    " |             |/  /\n",
    " |             /  /\n",
    " |            (  /\n",
    " \\             y'\n",
    "  `-.._____..-'",
);

const STEAM: &[&str] = &[
    "       (   )",
    "      (     )",
    "     \\     /",
    "      |   |",
    "       ) ( ",
    "      (   )",
];

fn draw_coffee_mug(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let fi = (app.tick_count / 4) as usize; // change every ~4 ticks (400ms at 100ms tick)

    let steam_lines = [
        STEAM[(fi + 2) % STEAM.len()],
        STEAM[(fi + 1) % STEAM.len()],
        STEAM[fi % STEAM.len()],
    ];

    let mut lines: Vec<Line> = steam_lines
        .iter()
        .map(|l| Line::from(Span::styled(*l, Style::default().fg(theme.primary))))
        .collect();

    for mug_line in MUG_BASE.lines() {
        lines.push(Line::from(Span::styled(
            mug_line.to_string(),
            Style::default().fg(theme.primary),
        )));
    }

    let para = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

// ─── Panel helper ─────────────────────────────────────────────────────────────

/// Draw a titled box panel.  Returns the inner area (inside the borders).
fn panel_block(title: &str, theme: Theme) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .border_type(BorderType::Plain)
}

// ─── Temperature Display ──────────────────────────────────────────────────────

fn draw_temperature_display(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let unit = app.temp_unit();
    let empty = app.mug_state.liquid_state == LiquidState::Empty;
    let current_str = if empty {
        "---".to_string()
    } else {
        format_temperature(app.mug_state.current_temp, unit)
    };
    let target_str = format_temperature(app.desired_temp, unit);

    let state_text = get_liquid_state_text(
        app.mug_state.liquid_state,
        app.mug_state.current_temp,
        app.mug_state.target_temp,
    );

    let time_str = estimate_time_to_target_temp(
        app.mug_state.current_temp,
        app.mug_state.target_temp,
        app.mug_state.liquid_state,
        Some(app.temp_rate),
    )
    .filter(|&t| t > 0.0)
    .map(|t| format!("  •  [~] {} to target", format_duration(t)));

    let lines = vec![
        Line::from(vec![]),
        Line::from(vec![
            Span::styled(format!("{:^10}", "Current"), Style::default().fg(theme.dim_text)),
            Span::styled("  →  ", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:^10}", "Target"), Style::default().fg(theme.dim_text)),
        ]),
        Line::from(vec![
            Span::styled(
                format!("{:^10}", current_str),
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            ),
            Span::raw("     "),
            Span::styled(
                format!("{:^10}", target_str),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![]),
        Line::from({
            let mut s = vec![Span::styled(
                state_text.to_string(),
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            )];
            if let Some(ts) = time_str {
                s.push(Span::styled(ts, Style::default().fg(theme.dim_text)));
            }
            s
        }),
    ];

    let block = panel_block("Temperature", theme);
    let para = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(para, area);
}

// ─── Battery Display ──────────────────────────────────────────────────────────

fn draw_battery_display(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let level = app.mug_state.battery_level;
    let charging = app.mug_state.is_charging;

    let level_str = if charging {
        format!("{}% ~", level)
    } else {
        format!("{}%", level)
    };

    let (filled, empty) = get_battery_bar(level, 20);
    let bar = format!("▐{}{}▌", "█".repeat(filled), "░".repeat(empty));

    let time_str = estimate_battery_life(
        level,
        charging,
        app.mug_state.liquid_state,
        Some(app.battery_rate),
    )
    .map(|t| {
        if charging {
            format!("[~] {} to full", format_duration(t))
        } else {
            format!("[~] {} remaining", format_duration(t))
        }
    });

    let lines = vec![
        Line::from(vec![]),
        Line::from(Span::styled(
            level_str,
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(bar, Style::default().fg(theme.primary))),
        Line::from(
            time_str.map(|s| Span::styled(s, Style::default().fg(theme.dim_text)))
                .unwrap_or_else(|| Span::raw("")),
        ),
    ];

    let block = panel_block("Battery", theme);
    let para = Paragraph::new(lines).block(block).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

// ─── Temperature Control ──────────────────────────────────────────────────────

fn draw_temperature_control(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let unit = app.temp_unit();
    let min_str = format_temperature(MIN_TEMP_CELSIUS, unit);
    let max_str = format_temperature(MAX_TEMP_CELSIUS, unit);

    // Slider
    let total_width = 16usize;
    let clamped = app.desired_temp.max(MIN_TEMP_CELSIUS).min(MAX_TEMP_CELSIUS);
    let norm = (clamped - MIN_TEMP_CELSIUS) / (MAX_TEMP_CELSIUS - MIN_TEMP_CELSIUS);
    let pos = (norm * (total_width - 1) as f64).round() as usize;
    let left = "━".repeat(pos);
    let right = "━".repeat(total_width.saturating_sub(pos + 1));

    let slider_line = Line::from(vec![
        Span::styled(min_str, Style::default().fg(theme.dim_text)),
        Span::raw(" "),
        Span::styled(left, Style::default().fg(theme.primary)),
        Span::styled("◉", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        Span::styled(right, Style::default().fg(theme.dim_text)),
        Span::raw(" "),
        Span::styled(max_str, Style::default().fg(theme.dim_text)),
    ]);

    let status_line = match app.temp_loading {
        LoadingState::Setting => Line::from(Span::styled(
            format!("{} Setting…", spinner_frame(app.tick_count)),
            Style::default().fg(theme.primary),
        )),
        LoadingState::Retrying => Line::from(Span::styled(
            format!("{} Retrying…", spinner_frame(app.tick_count)),
            Style::default().fg(theme.primary),
        )),
        LoadingState::Error => Line::from(Span::styled(
            "✗ Failed to set temperature",
            Style::default().fg(Color::Red),
        )),
        LoadingState::Idle => Line::from(vec![
            Span::styled("[←/→]", Style::default().fg(theme.primary)),
            Span::styled(" Adjust", Style::default().fg(theme.dim_text)),
        ]),
    };

    let lines = vec![Line::from(vec![]), slider_line, status_line];
    let block = panel_block("Temperature Adjust", theme);
    let para = Paragraph::new(lines).block(block).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

// ─── Presets ─────────────────────────────────────────────────────────────────

fn draw_presets(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let unit = app.temp_unit();
    let presets = &app.settings.presets;
    let selected = app.selected_preset_index;

    let min_horiz_width = (presets.len() * 15 + 4) as u16;
    let use_vertical = area.width < min_horiz_width;

    let mut lines = vec![Line::from(vec![])];

    if use_vertical {
        // Table layout
        for (i, preset) in presets.iter().enumerate() {
            let is_sel = selected == i as i64;
            let temp = format_temperature(preset.temperature, unit);
            let line = Line::from(vec![
                Span::styled(
                    if is_sel { "> " } else { "  " },
                    Style::default().fg(theme.primary),
                ),
                Span::styled(
                    format!("{:<8}", &preset.name),
                    Style::default()
                        .fg(if is_sel { theme.text } else { theme.dim_text })
                        .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                ),
                Span::styled(
                    format!("{:>7}", temp),
                    Style::default().fg(if is_sel { theme.primary } else { theme.text }),
                ),
                Span::styled(
                    format!("  [{}]", i + 1),
                    Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
                ),
            ]);
            lines.push(line);
        }
    } else {
        // Horizontal button layout
        let mut spans = vec![];
        for (i, preset) in presets.iter().enumerate() {
            let is_sel = selected == i as i64;
            let temp = format_temperature(preset.temperature, unit);
            spans.push(Span::styled("  ", Style::default()));
            spans.push(Span::styled(
                format!("[{}] {}: {}", i + 1, preset.name, temp),
                Style::default()
                    .fg(if is_sel { theme.primary } else { theme.dim_text })
                    .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
            ));
        }
        lines.push(Line::from(spans));
    }

    let block = panel_block("Temperature Presets", theme);
    let para = Paragraph::new(lines).block(block).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

// ─── Connection Status ────────────────────────────────────────────────────────

fn draw_connection_status(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    // Center a panel in the content area
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Min(12),
            Constraint::Percentage(20),
        ])
        .split(area);
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Min(50),
            Constraint::Percentage(15),
        ])
        .split(vert[1]);
    let panel_area = horiz[1];

    let (title, title_color) = if app.error.is_some() {
        ("[!] Error", Color::Red)
    } else if app.found_mug_name.is_some() {
        ("[~] Connecting", theme.primary)
    } else if app.is_scanning {
        ("[~] Scanning", theme.primary)
    } else {
        ("C[_] Welcome", theme.primary)
    };

    let block = Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(title_color))
        .border_type(BorderType::Plain);

    let mut lines: Vec<Line> = vec![Line::from(vec![])];

    if let Some(ref err) = app.error {
        let is_write_error = err.contains("does not allow write commands")
            || err.contains("write commands");

        lines.push(Line::from(Span::styled(
            if is_write_error { "Write Access Required" } else { err.as_str() },
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));

        if is_write_error {
            lines.push(Line::from(vec![]));
            lines.push(Line::from(Span::styled(
                "Your mug needs to be set up with the official Ember app first.",
                Style::default().fg(theme.dim_text),
            )));
            lines.push(Line::from(Span::styled(
                "1. Download the Ember app from the App Store",
                Style::default().fg(theme.dim_text),
            )));
            lines.push(Line::from(Span::styled(
                "2. Pair your mug and complete setup in the app",
                Style::default().fg(theme.dim_text),
            )));
            lines.push(Line::from(Span::styled(
                "3. Press [r] to reconnect here",
                Style::default().fg(theme.dim_text),
            )));
        }
        lines.push(Line::from(vec![]));
        lines.push(Line::from(vec![
            Span::styled("[r]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(" retry scanning", Style::default().fg(theme.dim_text)),
        ]));
    } else if app.is_scanning || app.found_mug_name.is_some() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", spinner_frame(app.tick_count)),
                Style::default().fg(theme.primary),
            ),
            Span::styled("Searching for Ember mug…", Style::default().fg(theme.text)),
        ]));
        if let Some(ref name) = app.found_mug_name {
            lines.push(Line::from(vec![]));
            lines.push(Line::from(Span::styled(
                format!("✓ Found: {name}"),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", spinner_frame(app.tick_count)),
                    Style::default().fg(theme.primary),
                ),
                Span::styled("Connecting…", Style::default().fg(theme.text)),
            ]));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No Ember mug connected",
            Style::default().fg(theme.text),
        )));
        lines.push(Line::from(vec![]));
        lines.push(Line::from(vec![
            Span::styled("[s]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" to start scanning", Style::default().fg(theme.dim_text)),
        ]));
    }

    let para = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });
    frame.render_widget(para, panel_area);
}

// ─── Settings view ────────────────────────────────────────────────────────────

fn draw_settings(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let (header_area, content_area, help_area) = outer_layout(area);
    draw_header(frame, header_area, app, theme);

    // Simplified help for settings
    let help_spans = vec![
        Span::raw("  "),
        Span::styled("[↑/↓]", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
        Span::styled(" navigate  ", Style::default().fg(theme.dim_text)),
        Span::styled("[←/→]", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
        Span::styled(" change  ", Style::default().fg(theme.dim_text)),
        Span::styled("[Esc]", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
        Span::styled(" close", Style::default().fg(theme.dim_text)),
    ];
    let help_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));
    let help_para = Paragraph::new(Line::from(help_spans))
        .block(help_block)
        .alignment(Alignment::Left);
    frame.render_widget(help_para, help_area);

    // Center the panel
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(5), Constraint::Min(0), Constraint::Percentage(5)])
        .split(content_area);
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(10), Constraint::Min(0), Constraint::Percentage(10)])
        .split(vert[1]);
    let panel_area = horiz[1];

    let selected = app.settings_selected;
    let unit = app.temp_unit();
    let presets = &app.settings.presets;

    let mut lines: Vec<Line> = vec![Line::from(vec![])];

    // Row 0: Temperature Unit
    {
        let unit_str = if unit == TemperatureUnit::Celsius {
            "Celsius (°C)"
        } else {
            "Fahrenheit (°F)"
        };
        lines.push(setting_row("Temperature Unit", unit_str, None, selected == 0, theme));
    }

    // Row 1: LED Color
    {
        let color = &app.desired_led_color;
        let color_name = find_led_preset_index(color)
            .map(|i| LED_PRESETS[i].name)
            .unwrap_or("Custom");
        let value_color = Color::Rgb(color.r, color.g, color.b);

        lines.push(setting_row_color(
            "LED Color",
            color_name,
            value_color,
            selected == 1,
            theme,
        ));

        if selected == 1 {
            let status = match app.led_loading {
                LoadingState::Setting => Some(format!("{} Setting…", spinner_frame(app.tick_count))),
                LoadingState::Retrying => Some(format!("{} Retrying…", spinner_frame(app.tick_count))),
                LoadingState::Error => Some("✗ Failed to set color".to_string()),
                LoadingState::Idle => None,
            };
            if let Some(s) = status {
                let col = if app.led_loading == LoadingState::Error {
                    Color::Red
                } else {
                    theme.primary
                };
                lines.push(Line::from(Span::styled(
                    format!("    {s}"),
                    Style::default().fg(col),
                )));
            }
        }
    }

    // Preset rows
    lines.push(Line::from(vec![]));
    lines.push(Line::from(Span::styled(
        "  Presets:",
        Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
    )));

    for (i, preset) in presets.iter().enumerate() {
        let row_idx = i + 2;
        let is_sel = selected == row_idx;
        let temp = format_temperature(preset.temperature, unit);
        let marker = if is_sel { "> " } else { "  " };
        let mut spans = vec![
            Span::styled(
                format!("    {marker}{}. {}: ", i + 1, preset.name),
                Style::default().fg(if is_sel { theme.primary } else { theme.dim_text }),
            ),
            Span::styled(
                temp,
                Style::default()
                    .fg(if is_sel { theme.text } else { theme.dim_text })
                    .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
            ),
        ];
        if is_sel {
            spans.push(Span::styled(
                "  (←/→ to edit)",
                Style::default().fg(theme.dim_text).add_modifier(Modifier::DIM),
            ));
        }
        lines.push(Line::from(spans));
    }

    let block = panel_block("[=] Settings", theme);
    let para = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Left);
    frame.render_widget(para, panel_area);
}

fn setting_row<'a>(label: &str, value: &str, hint: Option<&str>, selected: bool, theme: Theme) -> Line<'a> {
    let mut spans = vec![
        Span::styled(
            if selected { "> " } else { "  " },
            Style::default().fg(theme.primary),
        ),
        Span::styled(format!("{label}: "), Style::default().fg(theme.text)),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if selected {
        if let Some(h) = hint {
            spans.push(Span::styled(
                format!("  ({h})"),
                Style::default().fg(theme.dim_text).add_modifier(Modifier::DIM),
            ));
        }
    }
    Line::from(spans)
}

fn setting_row_color<'a>(label: &str, value: &str, value_color: Color, selected: bool, theme: Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            if selected { "> " } else { "  " },
            Style::default().fg(theme.primary),
        ),
        Span::styled(format!("{label}: "), Style::default().fg(theme.text)),
        Span::styled(
            "●  ".to_string(),
            Style::default().fg(value_color),
        ),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        ),
        if selected {
            Span::styled("  (←/→ to change)", Style::default().fg(theme.dim_text).add_modifier(Modifier::DIM))
        } else {
            Span::raw("")
        },
    ])
}

// ─── Repair instructions ──────────────────────────────────────────────────────

fn draw_repair(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let (header_area, content_area, _) = outer_layout(area);
    draw_header(frame, header_area, app, theme);

    // Center
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(10), Constraint::Min(0), Constraint::Percentage(10)])
        .split(content_area);
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(15), Constraint::Min(0), Constraint::Percentage(15)])
        .split(vert[1]);
    let panel_area = horiz[1];

    struct Step { title: &'static str, desc: &'static str, detail: Option<&'static str> }
    let steps = [
        Step { title: "Disconnect",     desc: "Mug has been disconnected",                         detail: None },
        Step { title: "Forget Device",  desc: "Open System Settings → Bluetooth",                  detail: Some("Click \"i\" next to your Ember mug → \"Forget This Device\"") },
        Step { title: "Reconnect",      desc: "Press [c] to scan for your mug again",              detail: Some("Press [Esc] to cancel") },
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            " Force Repair – Re-pairing with your Mug",
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![]),
    ];

    for (i, step) in steps.iter().enumerate() {
        let is_current = i == app.repair_step;
        let is_past = i < app.repair_step;
        let col = if is_past { Color::Green } else if is_current { theme.primary } else { Color::DarkGray };
        let marker = if is_past { "✓ " } else if is_current { "→ " } else { "○ " };

        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(col).add_modifier(Modifier::BOLD)),
            Span::styled(step.title, Style::default().fg(col).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("   {}", step.desc),
            Style::default().fg(if is_current { theme.text } else { Color::DarkGray }),
        )));
        if let Some(d) = step.detail {
            lines.push(Line::from(Span::styled(
                format!("   {d}"),
                Style::default().fg(if is_current { theme.text } else { Color::DarkGray }),
            )));
        }
        lines.push(Line::from(vec![]));
    }

    if app.repair_step == 2 {
        lines.push(Line::from(vec![
            Span::styled("Follow the steps above, then press ", Style::default().fg(theme.dim_text)),
            Span::styled("[c]", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            Span::styled(" to continue  ", Style::default().fg(theme.dim_text)),
            Span::styled("[Esc]", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            Span::styled(" to cancel", Style::default().fg(theme.dim_text)),
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .border_type(BorderType::Rounded);
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(para, panel_area);
}
