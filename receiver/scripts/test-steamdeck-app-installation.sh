#!/bin/sh
# Exercise install and uninstall without touching the developer's real Plasma
# profile. This is deliberately POSIX shell so CI and SteamOS use the same path.
set -eu

receiver_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/macdeck-install-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

build_root="$test_root/build"
bin_root="$test_root/user/.local/bin"
applications_root="$test_root/user/.local/share/applications"
icons_root="$test_root/user/.local/share/icons/hicolor/1024x1024/apps"
fallback_icons_root="$test_root/user/.local/share/icons/hicolor/scalable/apps"
pixmaps_root="$test_root/user/.local/share/pixmaps"
config_root="$test_root/user/.config/mac-deck-input-bridge"
fake_bin="$test_root/fake-bin"
log_file="$test_root/commands.log"
icon_source="$test_root/mac-deck-input-bridge.png"

mkdir -p "$build_root" "$fake_bin" "$config_root"
printf '#!/bin/sh\nexit 0\n' > "$build_root/mac-deck-receiver"
printf '#!/bin/sh\nexit 0\n' > "$build_root/mac-deck-receiver-ui"
chmod 755 "$build_root/mac-deck-receiver" "$build_root/mac-deck-receiver-ui"
printf 'retain this pairing configuration\n' > "$config_root/receiver.toml"
printf 'test icon\n' > "$icon_source"

for command in xdg-icon-resource update-desktop-database gtk-update-icon-cache kbuildsycoca6; do
  cat > "$fake_bin/$command" <<'EOF'
#!/bin/sh
printf '%s\n' "$(basename "$0") $*" >> "$MAC_DECK_TEST_LOG"
EOF
  chmod 755 "$fake_bin/$command"
done

run_with_test_paths() {
  HOME="$test_root/user" \
  PATH="$fake_bin:$PATH" \
  MAC_DECK_TEST_LOG="$log_file" \
  MAC_DECK_BUILD_DIR="$build_root" \
  MAC_DECK_INSTALL_BIN_DIR="$bin_root" \
  MAC_DECK_INSTALL_APPLICATIONS_DIR="$applications_root" \
  MAC_DECK_INSTALL_ICON_DIR="$icons_root" \
  MAC_DECK_INSTALL_FALLBACK_ICON_DIR="$fallback_icons_root" \
  MAC_DECK_INSTALL_PIXMAP_DIR="$pixmaps_root" \
  MAC_DECK_ICON_SOURCE="${MAC_DECK_TEST_ICON_SOURCE:-$icon_source}" \
  "$@"
}

run_with_test_paths sh "$receiver_root/scripts/install-steamdeck-app.sh"
test -x "$bin_root/mac-deck-receiver"
test -x "$bin_root/mac-deck-receiver-ui"
test -f "$applications_root/mac-deck-input-bridge.desktop"
test -f "$pixmaps_root/mac-deck-input-bridge.png"
grep -F "Exec=$bin_root/mac-deck-receiver-ui" \
  "$applications_root/mac-deck-input-bridge.desktop" >/dev/null

MAC_DECK_STOP_PROCESSES=0 run_with_test_paths sh "$receiver_root/scripts/uninstall-steamdeck-app.sh"
test ! -e "$bin_root/mac-deck-receiver"
test ! -e "$bin_root/mac-deck-receiver-ui"
test ! -e "$applications_root/mac-deck-input-bridge.desktop"
test ! -e "$pixmaps_root/mac-deck-input-bridge.png"
test -f "$config_root/receiver.toml"

grep -F "xdg-icon-resource uninstall" "$log_file" >/dev/null
grep -F "kbuildsycoca6 --noincremental" "$log_file" >/dev/null

echo "Steam Deck install/uninstall test passed."
