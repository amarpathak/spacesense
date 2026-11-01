#!/bin/sh
# Builds app/build/SpaceSense.app for this Mac's architecture (universal comes in M4).
# Needs only Command Line Tools (Swift 5.8+) and Rust.
set -e
cd "$(dirname "$0")/.."
VERSION="$(cat VERSION)"
ID=io.github.amarpathak.spacesense
ARCH="$(uname -m)"
export PATH="$HOME/.cargo/bin:$PATH"

cargo build --release -p spacesense-ffi
APP=app/build/SpaceSense.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
swiftc -O -target "$ARCH-apple-macos13" -import-objc-header app/bridge.h \
    -o "$APP/Contents/MacOS/SpaceSense" app/Sources/*.swift \
    -L target/release -lspacesense_ffi
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleName</key><string>SpaceSense</string>
  <key>CFBundleDisplayName</key><string>SpaceSense</string>
  <key>CFBundleExecutable</key><string>SpaceSense</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# Ad-hoc signature (Apple Silicon refuses unsigned code). macOS privacy grants
# (Full Disk Access) reset on every rebuild until real signing in M4.
codesign --force -s - "$APP"
echo "built $APP ($VERSION, $ARCH)"
