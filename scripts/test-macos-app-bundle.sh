#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sh "$project_root/scripts/build-macos-app.sh"
plist="$project_root/.build/MacDeckInputBridge.app/Contents/Info.plist"
value=$(/usr/libexec/PlistBuddy -c 'Print :LSUIElement' "$plist")
if [ "$value" != true ]; then
  echo "Expected the built app to start as a menu bar agent" >&2
  exit 1
fi
echo "macOS app bundle startup policy test passed."
