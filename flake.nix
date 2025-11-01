{
  description = "rust shell";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    esp-dev.url = "github:mirrexagon/nixpkgs-esp-dev";
  };

  outputs = { self, nixpkgs, flake-utils, fenix, esp-dev, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        # Import nixpkgs with overlays
        pkgs = import nixpkgs {
          overlays = [ fenix.overlays.default esp-dev.overlays.default];
          system = system;
        };

        # Construct LD_LIBRARY_PATH from buildInputs
        libPaths = with pkgs; [
          # library or etc
        ];

        # Create LD_LIBRARY_PATH
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath libPaths;

        # Rust toolchain
        rustToolchain = fenix.packages.${system}.stable.withComponents [
          "cargo"
          "clippy"
          "rustc"
          "rustfmt"
          "rust-src"
        ];

        # WASM target stdlib
        wasmTarget = fenix.packages.${system}.targets.wasm32-unknown-unknown.stable.rust-std;

        # Combine toolchain + target into one environment
        combinedToolchain = pkgs.symlinkJoin {
          name = "rust-toolchain-with-wasm";
          paths = [ rustToolchain wasmTarget ];
        };

      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            # === RUST ===
            combinedToolchain
            rust-analyzer
            pkg-config
            # === PYTHON ===
            python3
            # python313Packages.ar
            # === WASM2MPY ===
            # (wabt.overrideAttrs (oldAttrs: rec {
            #     version = "1.0.35";
            #     src = fetchurl {
            #       url = "https://github.com/WebAssembly/wabt/releases/download/1.0.35/wabt-1.0.35.tar.xz";
            #       sha256 = "04phxz2x5dx0hz9l0r36ihy19vdhs6sgbsc8ll0pq4s4a24hy8c7";
            #     };
            #   }))
            # esp-idf-xtensa
            # binaryen
            # # === MICROPYTHON ===
            mpremote
            esptool
            adafruit-ampy
            micropython
            # === C++ ===
            gcc
            gnumake
            # === TO IGNORE PLS ===
            nodejs_24
          ];

          buildInputs = libPaths;

          shellHook = ''
            export PATH=${combinedToolchain}/bin:$PATH
            export RUSTC="${combinedToolchain}/bin/rustc"
            export CARGO="${combinedToolchain}/bin/cargo"

            # Tell rustc where wasm32 stdlib actually lives
            export RUSTFLAGS="-L ${wasmTarget}/lib/rustlib/wasm32-unknown-unknown/lib"

            echo "Available targets:"
            rustc --print target-list | grep wasm32 || true

            echo "Using wasm stdlib path:"
            echo "${wasmTarget}/lib/rustlib/wasm32-unknown-unknown/lib"

            export LD_LIBRARY_PATH=${LD_LIBRARY_PATH}
            onefetch
            fish
          '';
        };
      }
    );
}
