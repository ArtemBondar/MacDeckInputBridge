#!/bin/sh
# Remove the MacDeck Desktop Mode launcher and its per-user files. Pairing
# settings are intentionally retained so reinstalling does not require a new
# pairing; remove them separately only when a full reset is wanted.
set -eu

bin_root=${MAC_DECK_INSTALL_BIN_DIR:-"$HOME/.local/bin"}
applications_root=${MAC_DECK_INSTALL_APPLICATIONS_DIR:-"$HOME/.local/share/applications"}
icons_root=${MAC_DECK_INSTALL_ICON_DIR:-"$HOME/.local/share/icons/hicolor/1024x1024/apps"}
fallback_icons_root=${MAC_DECK_INSTALL_FALLBACK_ICON_DIR:-"$HOME/.local/share/icons/hicolor/scalable/apps"}
pixmaps_root=${MAC_DECK_INSTALL_PIXMAP_DIR:-"$HOME/.local/share/pixmaps"}

# The running receiver owns the virtual keyboard. Ask it to exit before its
# executable is removed; it releases any injected keys during shutdown.
if [ "${MAC_DECK_STOP_PROCESSES:-1}" = "1" ] && command -v pkill >/dev/null 2>&1; then
  pkill -x mac-deck-receiver-ui 2>/dev/null || true
  pkill -x mac-deck-receiver 2>/dev/null || true
fi

rm -f "$bin_root/mac-deck-receiver"
rm -f "$bin_root/mac-deck-receiver-ui"
rm -f "$applications_root/mac-deck-input-bridge.desktop"
rm -f "$icons_root/mac-deck-input-bridge.png"
rm -f "$fallback_icons_root/mac-deck-input-bridge.svg"
rm -f "$fallback_icons_root/mac-deck-input-bridge-tray-symbolic.svg"
rm -f "$fallback_icons_root/mac-deck-input-bridge-tray-active-symbolic.svg"
rm -f "$pixmaps_root/mac-deck-input-bridge.png"
rm -f "$pixmaps_root/mac-deck-input-bridge.svg"

if command -v xdg-icon-resource >/dev/null 2>&1; then
  if ! xdg-icon-resource uninstall --mode user --context apps --size 1024 \
    mac-deck-input-bridge; then
    echo "Warning: xdg-icon-resource could not remove the MacDeck icon." >&2
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

echo "Uninstalled MacDeck. Pairing settings were kept."
