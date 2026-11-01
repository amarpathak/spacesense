#!/bin/sh
# Builds app/build/SpaceSense.app (universal: Intel + Apple Silicon) and
# app/build/spacesense (CLI, universal).
# Needs only Command Line Tools (Swift 5.8+) and Rust with both macOS targets:
#   rustup target add x86_64-apple-darwin aarch64-apple-darwin
# ARCHS="x86_64" app/build.sh builds one architecture (faster while developing).
set -e
cd "$(dirname "$0")/.."
VERSION="$(cat VERSION)"
ID=io.github.amarpathak.spacesense
ARCHS="${ARCHS:-x86_64 arm64}"
export PATH="$HOME/.cargo/bin:$PATH"

APP=app/build/SpaceSense.app
rm -rf "$APP" app/build/obj
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" app/build/obj

BINS=""
CLIS=""
for arch in $ARCHS; do
    rust_arch=$arch
    [ "$arch" = arm64 ] && rust_arch=aarch64
    triple="$rust_arch-apple-darwin"
    cargo build --release --target "$triple" -p spacesense-ffi -p spacesense
    swiftc -O -target "$arch-apple-macos13" -import-objc-header app/bridge.h \
        -o "app/build/obj/SpaceSense-$arch" app/Sources/*.swift \
        -L "target/$triple/release" -lspacesense_ffi
    BINS="$BINS app/build/obj/SpaceSense-$arch"
    CLIS="$CLIS target/$triple/release/spacesense"
done
lipo -create $BINS -output "$APP/Contents/MacOS/SpaceSense"
lipo -create $CLIS -output app/build/spacesense
cp packaging/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleName</key><string>SpaceSense</string>
  <key>CFBundleDisplayName</key><string>SpaceSense</string>
  <key>CFBundleExecutable</key><string>SpaceSense</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT License. github.com/amarpathak/spacesense</string>
</dict></plist>
PLIST
# Ad-hoc signature (Apple Silicon refuses unsigned code). macOS privacy grants
# (Full Disk Access) reset on every rebuild until a Developer ID signature.
codesign --force -s - app/build/spacesense
codesign --force -s - "$APP"
echo "built $APP and app/build/spacesense ($VERSION: $(lipo -archs "$APP/Contents/MacOS/SpaceSense"))"
