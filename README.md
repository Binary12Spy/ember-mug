# C[_] ember-mug-rs

A native Rust TUI for controlling your [Ember Mug](https://ember.com) — **no Node.js required**.

A port of [singerbj/ember-mug](https://github.com/singerbj/ember-mug) from TypeScript / Ink / React to pure Rust, producing a single self-contained binary.

## Features

- 🔵 BLE auto-scan & connect (BlueZ/DBus on Linux, CoreBluetooth on macOS)
- 🌡️ Live temperature display with heating / cooling / stable state
- 🎯 Target temperature control with ←/→ arrow keys (debounced, with retry)
- 🔋 Battery level with estimated charge / discharge time
- 🎨 LED ring colour (8 presets, editable in settings)
- 🔖 Up to 9 named temperature presets (editable in settings, persisted)
- °C / °F toggle
- Animated ASCII coffee mug & steam
- Responsive layout — adapts to narrow (<90 cols) or wide terminals
- Settings persisted to `~/.config/ember-mug-rs/settings.json`
- `RUST_LOG=debug` for detailed BLE trace output
- `EMBER_MOCK=1` mock mode — full UI with simulated mug, no hardware needed

## Installation

### With Nix (recommended)

```bash
nix run github:Binary12Spy/ember-mug
```

Or clone and use the dev shell:

```bash
git clone https://github.com/Binary12Spy/ember-mug
cd ember-mug-rs
nix develop          # drops you into a shell with Rust + dbus headers
cargo build --release
./target/release/ember-mug
```

### Without Nix

Install system dependencies first:

```bash
# Ubuntu / Debian
sudo apt install pkg-config libdbus-1-dev

# Fedora
sudo dnf install pkgconf-pkg-config dbus-devel

# Arch
sudo pacman -S pkgconf dbus

# macOS — no extra deps needed (CoreBluetooth is built in)
```

Then:

```bash
cargo install --git https://github.com/Binary12Spy/ember-mug
```

## Using in a Nix configuration

Add the repo as a flake input, then choose whichever approach fits your setup.

### Flake input

```nix
# flake.nix
inputs.ember-mug.url = "github:Binary12Spy/ember-mug";
```

### NixOS module (recommended for system-wide install)

```nix
# flake.nix
outputs = { nixpkgs, ember-mug, ... }: {
  nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
    modules = [
      ember-mug.nixosModules.default
      {
        programs.ember-mug.enable = true;
      }
    ];
  };
};
```

### home-manager module (per-user install)

```nix
# flake.nix
outputs = { home-manager, ember-mug, ... }: {
  homeConfigurations.yourname = home-manager.lib.homeManagerConfiguration {
    modules = [
      ember-mug.homeManagerModules.default
      {
        programs.ember-mug.enable = true;
      }
    ];
  };
};
```

### Overlay (add `pkgs.ember-mug` to nixpkgs)

Useful if you prefer to manage packages manually or already have a central overlay list:

```nix
nixpkgs.overlays = [ inputs.ember-mug.overlays.default ];

# Then anywhere in your config:
environment.systemPackages = [ pkgs.ember-mug ];
# or
home.packages = [ pkgs.ember-mug ];
```

### One-off (no config changes needed)

```bash
# Run directly without installing
nix run github:Binary12Spy/ember-mug

# Install into your personal profile
nix profile install github:Binary12Spy/ember-mug
```

## Requirements

- Bluetooth 4.0+ adapter with BLE support
- Ember Mug 2 (other models may work but are untested)
- **Linux:** BlueZ ≥ 5, user in the `bluetooth` group (or run with `sudo`)
- **macOS:** grant Bluetooth permission to your terminal app in  
  System Settings → Privacy & Security → Bluetooth

### Important first-time setup

**Your mug must be set up with the official Ember app at least once before this tool can write to it.** The mug uses a UDSK provisioning handshake that only the official app performs. Without it you can read temperature and battery, but cannot change the target temperature or LED colour. After completing setup in the official app, press `r` here to reconnect.

## Controls

### When disconnected

| Key | Action |
|-----|--------|
| `s` | Start scanning |
| `r` | Retry after error |
| `q` / `Ctrl-C` | Quit |

### When connected

| Key | Action |
|-----|--------|
| `←` / `→` | Adjust target temperature (1 °F or 0.5 °C) |
| `1`–`9` | Apply temperature preset |
| `u` | Toggle °C / °F |
| `o` | Open settings |
| `r` | Force re-pair (forget & reconnect) |
| `q` / `Ctrl-C` | Quit |

### In settings (`o`)

| Key | Action |
|-----|--------|
| `↑` / `↓` or `k` / `j` | Navigate rows |
| `←` / `→` or `h` / `l` | Change value / edit preset temperature |
| `Esc` / `q` | Close settings |

## Debug logging

```bash
RUST_LOG=debug ./ember-mug 2>ble.log
```

Logs all BLE scan/connect/characteristic read-write events to stderr.

## Mock mode

Run the full TUI with a simulated mug — no Bluetooth hardware required:

```bash
EMBER_MOCK=1 ./ember-mug
```

The mock task simulates:
- Scan delay (~1.5 s) then auto-connect (~0.8 s)
- Temperature heating / cooling toward the target at 0.5 °C/s
- Liquid state transitions (Cooling → Heating → Stable)
- Battery drain (varies by state) and charging
- Full response to all commands: target temp, temperature unit, LED colour

The `[MOCK]` label appears in the header so it's always clear you're not on real hardware.

## Architecture

```
src/
  main.rs        Tokio entry point; async select! loop over tick / keyboard / BLE events
  bluetooth.rs   BLE task (btleplug); emits BleEvent, accepts BleCommand over mpsc channels
  app.rs         Application state; handles events, keyboard input, debounce & retry logic
  ui.rs          All ratatui rendering — pure, no state mutation
  types.rs       Shared types: MugState, LiquidState, Preset, RgbColor, BLE UUID constants
  utils.rs       Pure helpers: temperature conversion, rate calculation, time estimation
  settings.rs    JSON settings persistence (~/.config/ember-mug-rs/settings.json)
  theme.rs       Per-state colour themes + LED colour presets
```

The UI and BLE layers never share memory — they communicate only through typed
`tokio::sync::mpsc` channels:

```
┌─────────────────┐  BleCommand  ┌──────────────────────┐
│  App / UI       │ ──────────▶  │  BLE task            │
│  (main thread)  │ ◀──────────  │  (tokio::spawn)      │
└─────────────────┘  BleEvent    └──────────────────────┘
```

## Configuration

Settings are stored at `~/.config/ember-mug-rs/settings.json`
(respects `$XDG_CONFIG_HOME` on Linux). The file is created automatically on first use.

```json
{
  "presets": [
    { "id": "1", "name": "Latte",  "temperature": 52.0  },
    { "id": "2", "name": "Coffee", "temperature": 55.56 },
    { "id": "3", "name": "Tea",    "temperature": 60.0  }
  ],
  "temperature_unit": "Fahrenheit",
  "last_target_temp": 55.56,
  "led_color": { "r": 255, "g": 147, "b": 41, "a": 255 }
}
```

## Credits

- **[singerbj/ember-mug](https://github.com/singerbj/ember-mug)** — the TypeScript / Ink original this project ports
- **[orlopau/ember-mug](https://github.com/orlopau/ember-mug)** — reverse-engineering the Ember BLE protocol
- **[EmberMate](https://github.com/matthewnitschke/EmberMate)** — prior art and inspiration
- **[python-ember-mug](https://github.com/sopelj/python-ember-mug)** — UDSK write-capability documentation
- Coffee mug ASCII art by [Felix Lee](https://www.ascii-art.de/ascii/c/coffee.txt)

## License

MIT — see [LICENSE](LICENSE).
