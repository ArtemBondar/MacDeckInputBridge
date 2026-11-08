#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_root="$project_root/.build/release"
app_root="$project_root/.build/MacDeckInputBridge.app"
contents="$app_root/Contents"
resources="$contents/Resources"
icon_source="$project_root/assets/MacDeckInputBridgeIcon.png"
signing_identity="${MAC_DECK_CODESIGN_IDENTITY:-}"
app_version="${MAC_DECK_APP_VERSION:-$(tr -d '\n' < "$project_root/VERSION")}"
build_number="${MAC_DECK_BUILD_NUMBER:-1}"

if [ -z "$signing_identity" ]; then
  signing_identity=$(security find-identity -v -p codesigning 2>/dev/null | \
    sed -n 's/.*"\(Apple Development:.*\)"/\1/p' | head -n 1 || true)
fi

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
  <string>$app_version</string>
  <key>CFBundleVersion</key>
  <string>$build_number</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
</dict>
</plist>
PLIST

# Accessibility trust is associated with the app's designated requirement.
# An Apple Development certificate supplies a stable requirement across local
# builds; an ad-hoc signature does not. Use MAC_DECK_CODESIGN_IDENTITY to pick
# a specific identity when more than one is available.
if [ -n "$signing_identity" ]; then
  codesign --force --sign "$signing_identity" --identifier com.artembondar.MacDeckInputBridge "$app_root"
  echo "Signed with: $signing_identity"
else
  codesign --force --sign - --identifier com.artembondar.MacDeckInputBridge "$app_root"
  echo "WARNING: no Apple Development signing identity was found. Accessibility will need to be granted again after each changed build."
  echo "Add an Apple Development certificate in Xcode Settings > Accounts > Manage Certificates, then rebuild."
fi
codesign --verify --deep --strict "$app_root"

echo "Built $app_root"
echo "Launch it with: open '$app_root'"
