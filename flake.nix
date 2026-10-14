{
  description = "ember-mug – a native Rust TUI for the Ember Mug, no Node required";

  inputs = {
    nixpkgs.url     = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay    = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    # ── Per-system outputs (packages, devShells, apps) ────────────────────────
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

        # Shared build inputs (also re-used by the overlay package builder)
        nativeBuildInputs = with pkgs; [ rust pkg-config ];
        buildInputs       = with pkgs; [ dbus dbus.dev openssl openssl.dev ];
        PKG_CONFIG_PATH   = pkgs.lib.makeSearchPathOutput "dev" "lib/pkgconfig" buildInputs;

        package = pkgs.rustPlatform.buildRustPackage {
          pname   = "ember-mug";
          version = "0.1.0";
          src     = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          inherit nativeBuildInputs buildInputs PKG_CONFIG_PATH;

          meta = with pkgs.lib; {
            description = "Native Rust TUI for the Ember Mug – no Node required";
            homepage    = "https://github.com/Binary12Spy/ember-mug";
            license     = licenses.mit;
            platforms   = platforms.linux ++ platforms.darwin;
            mainProgram = "ember-mug";
          };
        };
      in {
        # ── `nix build` / `pkgs.ember-mug` ───────────────────────────────────
        packages.default   = package;
        packages.ember-mug = package;

        # ── `nix run` ────────────────────────────────────────────────────────
        apps.default = flake-utils.lib.mkApp { drv = package; };

        # ── `nix develop` ────────────────────────────────────────────────────
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs PKG_CONFIG_PATH;

          shellHook = ''
            echo "ember-mug dev shell"
            echo "  rustc $(rustc --version)"
            echo "  cargo $(cargo --version)"
            echo ""
            echo "  cargo run                    – run (real BLE)"
            echo "  EMBER_MOCK=1 cargo run       – run with simulated mug"
            echo "  RUST_LOG=debug cargo run     – run with BLE debug logging"
            echo "  cargo build --release        – release build"
          '';
        };
      }
    )

    # ── System-independent outputs ────────────────────────────────────────────
    // {
      # overlay: adds `pkgs.ember-mug` to any nixpkgs instance
      #
      # Usage in a flake:
      #   nixpkgs.overlays = [ inputs.ember-mug.overlays.default ];
      #   environment.systemPackages = [ pkgs.ember-mug ];
      overlays.default = final: _prev:
        let
          pkgs = import nixpkgs {
            system  = final.system;
            overlays = [ (import rust-overlay) ];
          };
        in {
          ember-mug = self.packages.${final.system}.default;
        };

      # NixOS module: programs.ember-mug.enable = true;
      nixosModules.default = { config, pkgs, lib, ... }: {
        options.programs.ember-mug = {
          enable = lib.mkEnableOption "ember-mug Ember Mug TUI";
        };

        config = lib.mkIf config.programs.ember-mug.enable {
          environment.systemPackages = [ self.packages.${pkgs.system}.default ];
        };
      };

      # home-manager module: programs.ember-mug.enable = true;
      homeManagerModules.default = { config, pkgs, lib, ... }: {
        options.programs.ember-mug = {
          enable = lib.mkEnableOption "ember-mug Ember Mug TUI";
        };

        config = lib.mkIf config.programs.ember-mug.enable {
          home.packages = [ self.packages.${pkgs.system}.default ];
        };
      };
    };
}
