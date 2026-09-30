#!/usr/bin/env bash
# Builds Portal Workspaces in release mode and installs it for the current user.
#   Linux: binary to ~/.local/bin, launcher + icon to ~/.local/share
#   macOS: Portal Workspaces.app to ~/Applications (needs `cargo install cargo-bundle`)
set -euo pipefail
cd "$(dirname "$0")/.."

command -v cargo >/dev/null || { echo "cargo not found: install Rust from https://rustup.rs" >&2; exit 1; }

case "$(uname -s)" in
  Linux)
    cargo build --release -p pw-app
    install -Dm755 target/release/portal-workspaces "$HOME/.local/bin/portal-workspaces"
    install -Dm644 packaging/linux/portal-workspaces.desktop "$HOME/.local/share/applications/portal-workspaces.desktop"
    install -Dm644 assets/icons/portal-workspaces.svg "$HOME/.local/share/icons/hicolor/scalable/apps/portal-workspaces.svg"
    install -Dm644 assets/icons/portal-workspaces.png "$HOME/.local/share/icons/hicolor/512x512/apps/portal-workspaces.png"
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$HOME/.local/share/applications" || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" || true
    echo "Installed. Launch \"Portal Workspaces\" from your app menu, or run: portal-workspaces"
    case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Note: ~/.local/bin is not on your PATH." ;; esac
    ;;
  Darwin)
    command -v cargo-bundle >/dev/null || cargo install cargo-bundle
    cargo bundle --release -p pw-app
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/Portal Workspaces.app"
    cp -R "target/release/bundle/osx/Portal Workspaces.app" "$HOME/Applications/"
    echo "Installed to ~/Applications/Portal Workspaces.app"
    ;;
  *) echo "Unsupported platform: $(uname -s)" >&2; exit 1 ;;
esac
