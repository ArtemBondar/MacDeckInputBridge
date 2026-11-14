#!/bin/sh
# Install the Steam Deck app into the current user's account.  A desktop
# launcher is started by Plasma with a restricted environment, so it must use
# an absolute path instead of relying on ~/.local/bin being in PATH.
set -eu

receiver_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
project_root=$(CDPATH= cd -- "$receiver_root/.." && pwd)
build_root=${MAC_DECK_BUILD_DIR:-"$receiver_root/target/release"}
bin_root=${MAC_DECK_INSTALL_BIN_DIR:-"$HOME/.local/bin"}
applications_root=${MAC_DECK_INSTALL_APPLICATIONS_DIR:-"$HOME/.local/share/applications"}
icons_root=${MAC_DECK_INSTALL_ICON_DIR:-"$HOME/.local/share/icons/hicolor/1024x1024/apps"}
fallback_icons_root=${MAC_DECK_INSTALL_FALLBACK_ICON_DIR:-"$HOME/.local/share/icons/hicolor/scalable/apps"}
pixmaps_root=${MAC_DECK_INSTALL_PIXMAP_DIR:-"$HOME/.local/share/pixmaps"}
ui_binary="$build_root/mac-deck-receiver-ui"
receiver_binary="$build_root/mac-deck-receiver"
bundled_icon="$receiver_root/resources/mac-deck-input-bridge.png"
source_checkout_icon="$project_root/assets/MacDeckInputBridgeIcon.png"
icon_source=${MAC_DECK_ICON_SOURCE:-"$bundled_icon"}
fallback_icon="$receiver_root/resources/mac-deck-input-bridge.svg"
tray_icon="$receiver_root/resources/mac-deck-input-bridge-tray-symbolic.svg"

# A release archive intentionally ships prebuilt binaries and has no Cargo
# manifest. A source checkout does have one, so compile it before installing:
# otherwise `git pull` followed by this script silently reinstalls an old
# target/release binary.
if [ -f "$receiver_root/Cargo.toml" ] && [ -z "${MAC_DECK_BUILD_DIR:-}" ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required to install from a source checkout." >&2
    exit 1
  fi
  echo "Building MacDeck from the current source checkout..."
  (cd "$receiver_root" && cargo build --release --locked --features desktop-ui --bins)
fi

# Release archives carry a single app icon beside the launcher resource. A
# source checkout can still install its asset without requiring a generated
# copy in receiver/resources.
if [ ! -f "$icon_source" ] && [ -f "$source_checkout_icon" ]; then
  icon_source="$source_checkout_icon"
fi

if [ ! -x "$ui_binary" ] || [ ! -x "$receiver_binary" ]; then
  echo "Missing release binaries. Run: cargo build --release --locked --features desktop-ui --bins" >&2
  exit 1
fi

mkdir -p "$bin_root" "$icons_root" "$fallback_icons_root" "$pixmaps_root"
install -m 755 "$receiver_binary" "$bin_root/mac-deck-receiver"
install -m 755 "$ui_binary" "$bin_root/mac-deck-receiver-ui"
if [ -f "$icon_source" ]; then
  # The full repository includes the exact macOS app icon.
  install -m 644 "$icon_source" "$icons_root/mac-deck-input-bridge.png"
  # ~/.local/share/pixmaps is supported by older Plasma releases too.
  installed_icon="$pixmaps_root/mac-deck-input-bridge.png"
  install -m 644 "$icon_source" "$installed_icon"
else
  # A source archive may exclude large generated assets. The launcher must
  # still work; use the matching, source-controlled menu-bar silhouette.
  install -m 644 "$fallback_icon" "$fallback_icons_root/mac-deck-input-bridge.svg"
  installed_icon="$pixmaps_root/mac-deck-input-bridge.svg"
  install -m 644 "$fallback_icon" "$installed_icon"
  echo "MacDeck PNG icon was not found; installed the matching vector icon instead." >&2
fi

mkdir -p "$applications_root"
mkdir -p "$fallback_icons_root"
install -m 644 "$tray_icon" "$fallback_icons_root/mac-deck-input-bridge-tray-symbolic.svg"
install -m 644 "$receiver_root/resources/mac-deck-input-bridge-tray-active-symbolic.svg" "$fallback_icons_root/mac-deck-input-bridge-tray-active-symbolic.svg"
sed -e "s|@MAC_DECK_RECEIVER_UI@|$bin_root/mac-deck-receiver-ui|" \
  "$receiver_root/resources/mac-deck-input-bridge.desktop" \
  > "$applications_root/mac-deck-input-bridge.desktop"
chmod 644 "$applications_root/mac-deck-input-bridge.desktop"

# Plasma 5 on SteamOS can ignore icons copied into a user theme until they are
# registered as an application resource. xdg-icon-resource handles the correct
# user data directory and cache format for both older Plasma and current KDE.
if command -v xdg-icon-resource >/dev/null 2>&1; then
  if ! xdg-icon-resource install --mode user --context apps --size 1024 \
    "$installed_icon" mac-deck-input-bridge; then
    echo "Warning: xdg-icon-resource could not register the MacDeck icon." >&2
  fi
  if ! xdg-icon-resource forceupdate --mode user; then
    echo "Warning: xdg-icon-resource could not refresh its icon cache." >&2
  fi
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  if ! update-desktop-database "$applications_root"; then
    echo "Warning: could not refresh the desktop-entry database." >&2
  fi
fi

if { [ "$icons_root" = "$HOME/.local/share/icons/hicolor/1024x1024/apps" ] || \
     [ "$fallback_icons_root" = "$HOME/.local/share/icons/hicolor/scalable/apps" ]; } && \
  command -v gtk-update-icon-cache >/dev/null 2>&1; then
  if ! gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor"; then
    echo "Warning: could not refresh the GTK icon cache." >&2
  fi
fi

if command -v kbuildsycoca6 >/dev/null 2>&1; then
  if ! kbuildsycoca6 --noincremental; then
    echo "Warning: could not refresh the Plasma application cache." >&2
  fi
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
  if ! kbuildsycoca5 --noincremental; then
    echo "Warning: could not refresh the Plasma application cache." >&2
  fi
fi

echo "Installed MacDeck. Open it from the Steam Deck application launcher."
