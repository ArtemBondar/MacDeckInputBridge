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
ui_binary="$build_root/mac-deck-receiver-ui"
receiver_binary="$build_root/mac-deck-receiver"
icon_source="$project_root/assets/MacDeckInputBridgeIcon.png"

if [ ! -x "$ui_binary" ] || [ ! -x "$receiver_binary" ]; then
  echo "Missing release binaries. Run: cargo build --release --locked --features desktop-ui --bins" >&2
  exit 1
fi

if [ ! -f "$icon_source" ]; then
  echo "Missing MacDeck icon at $icon_source." >&2
  exit 1
fi

mkdir -p "$bin_root" "$icons_root"
install -m 755 "$receiver_binary" "$bin_root/mac-deck-receiver"
install -m 755 "$ui_binary" "$bin_root/mac-deck-receiver-ui"
install -m 644 "$icon_source" \
  "$icons_root/mac-deck-input-bridge.png"

mkdir -p "$applications_root"
sed "s|@MAC_DECK_RECEIVER_UI@|$bin_root/mac-deck-receiver-ui|" \
  "$receiver_root/resources/mac-deck-input-bridge.desktop" \
  > "$applications_root/mac-deck-input-bridge.desktop"
chmod 644 "$applications_root/mac-deck-input-bridge.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$applications_root" >/dev/null 2>&1 || true
fi

if [ "$icons_root" = "$HOME/.local/share/icons/hicolor/1024x1024/apps" ] && \
  command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

if command -v kbuildsycoca6 >/dev/null 2>&1; then
  kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
  kbuildsycoca5 --noincremental >/dev/null 2>&1 || true
fi

echo "Installed MacDeck. Open it from the Steam Deck application launcher."
