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
ui_binary="$build_root/mac-deck-receiver-ui"
receiver_binary="$build_root/mac-deck-receiver"
icon_source=${MAC_DECK_ICON_SOURCE:-"$project_root/assets/MacDeckInputBridgeIcon.png"}
fallback_icon="$receiver_root/resources/mac-deck-input-bridge.svg"

if [ ! -x "$ui_binary" ] || [ ! -x "$receiver_binary" ]; then
  echo "Missing release binaries. Run: cargo build --release --locked --features desktop-ui --bins" >&2
  exit 1
fi

mkdir -p "$bin_root" "$icons_root" "$fallback_icons_root"
install -m 755 "$receiver_binary" "$bin_root/mac-deck-receiver"
install -m 755 "$ui_binary" "$bin_root/mac-deck-receiver-ui"
if [ -f "$icon_source" ]; then
  # The full repository includes the exact macOS app icon.
  install -m 644 "$icon_source" "$icons_root/mac-deck-input-bridge.png"
else
  # A source archive may exclude large generated assets. The launcher must
  # still work; use the matching, source-controlled menu-bar silhouette.
  install -m 644 "$fallback_icon" "$fallback_icons_root/mac-deck-input-bridge.svg"
  echo "MacDeck PNG icon was not found; installed the matching vector icon instead." >&2
fi

mkdir -p "$applications_root"
sed "s|@MAC_DECK_RECEIVER_UI@|$bin_root/mac-deck-receiver-ui|" \
  "$receiver_root/resources/mac-deck-input-bridge.desktop" \
  > "$applications_root/mac-deck-input-bridge.desktop"
chmod 644 "$applications_root/mac-deck-input-bridge.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$applications_root" >/dev/null 2>&1 || true
fi

if { [ "$icons_root" = "$HOME/.local/share/icons/hicolor/1024x1024/apps" ] || \
     [ "$fallback_icons_root" = "$HOME/.local/share/icons/hicolor/scalable/apps" ]; } && \
  command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

if command -v kbuildsycoca6 >/dev/null 2>&1; then
  kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
  kbuildsycoca5 --noincremental >/dev/null 2>&1 || true
fi

echo "Installed MacDeck. Open it from the Steam Deck application launcher."
