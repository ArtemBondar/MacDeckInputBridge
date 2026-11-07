#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_app="$project_root/.build/MacDeckInputBridge.app"
install_root="${MAC_DECK_INSTALL_DIR:-$HOME/Applications}"
destination="$install_root/MacDeckInputBridge.app"

sh "$project_root/scripts/build-macos-app.sh"
mkdir -p "$install_root"
ditto "$source_app" "$destination"

echo "Installed $destination"
echo "Open it from Applications or run: open '$destination'"
