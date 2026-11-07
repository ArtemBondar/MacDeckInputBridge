#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_root="$project_root/.build/release"
app_root="$project_root/.build/MacDeckInputBridge.app"
contents="$app_root/Contents"
resources="$contents/Resources"
icon_source="$project_root/assets/MacDeckInputBridgeIcon.jpg"

cd "$project_root"
swift build -c release --product mac-deck-menu-bar

rm -rf "$app_root"
mkdir -p "$contents/MacOS" "$resources/AppIcon.iconset"
cp "$build_root/mac-deck-menu-bar" "$contents/MacOS/MacDeckInputBridge"

for spec in \
  "16 icon_16x16.png" \
  "32 icon_16x16@2x.png" \
  "32 icon_32x32.png" \
  "64 icon_32x32@2x.png" \
  "128 icon_128x128.png" \
  "256 icon_128x128@2x.png" \
  "256 icon_256x256.png" \
  "512 icon_256x256@2x.png" \
  "512 icon_512x512.png" \
  "1024 icon_512x512@2x.png"
do
  set -- $spec
  sips -s format png -z "$1" "$1" "$icon_source" --out "$resources/AppIcon.iconset/$2" >/dev/null
done
iconutil -c icns "$resources/AppIcon.iconset" -o "$resources/AppIcon.icns"
rm -rf "$resources/AppIcon.iconset"

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
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleName</key>
  <string>MacDeckInputBridge</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.1</string>
  <key>CFBundleVersion</key>
  <string>2</string>
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