#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
MAC_DECK_APP_VERSION=12.34.56 MAC_DECK_BUILD_NUMBER=789 sh "$project_root/scripts/build-macos-app.sh"
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
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist")" = 12.34.56
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$plist")" = 789
echo "macOS app bundle startup policy, name and version test passed."
