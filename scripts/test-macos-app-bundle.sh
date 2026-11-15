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
for key in CFBundleName CFBundleDisplayName; do
  value=$(/usr/libexec/PlistBuddy -c "Print :$key" "$plist")
  if [ "$value" != MacDeck ]; then
    echo "Expected $key to be MacDeck" >&2
    exit 1
  fi
done
echo "macOS app bundle startup policy and name test passed."
