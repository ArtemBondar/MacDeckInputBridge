#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_root="$project_root/.build/release"
app_root="$project_root/.build/MacDeckInputBridge.app"
contents="$app_root/Contents"

cd "$project_root"
swift build -c release --product mac-deck-menu-bar

rm -rf "$app_root"
mkdir -p "$contents/MacOS"
cp "$build_root/mac-deck-menu-bar" "$contents/MacOS/MacDeckInputBridge"

cat > "$contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>MacDeckInputBridge</string>
  <key>CFBundleExecutable</key>
  <string>MacDeckInputBridge</string>
  <key>CFBundleIdentifier</key>
  <string>com.artembondar.MacDeckInputBridge</string>
  <key>CFBundleName</key>
  <string>MacDeckInputBridge</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
</dict>
</plist>
PLIST

# The SwiftPM executable is linker-signed with its target name. Re-sign the
# completed bundle with its final bundle identifier so macOS TCC evaluates the
# same identity shown in Accessibility settings.
codesign --force --sign - --identifier com.artembondar.MacDeckInputBridge "$app_root"
codesign --verify --deep --strict "$app_root"

echo "Built $app_root"
echo "Launch it with: open '$app_root'"
