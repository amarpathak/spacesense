#!/bin/sh
# Builds dist/SpaceSense-<version>.zip and .dmg (universal, ad-hoc signed).
set -e
cd "$(dirname "$0")/.."
VERSION="$(cat VERSION)"
app/build.sh
rm -rf dist build-dmg && mkdir -p dist build-dmg
export COPYFILE_DISABLE=1
xattr -cr app/build/SpaceSense.app
ditto -c -k --keepParent app/build/SpaceSense.app "dist/SpaceSense-$VERSION.zip"
# Disk image with an Applications shortcut for drag-to-install.
cp -R app/build/SpaceSense.app build-dmg/
ln -s /Applications build-dmg/Applications
hdiutil create -quiet -volname "SpaceSense $VERSION" -srcfolder build-dmg -ov -format UDZO "dist/SpaceSense-$VERSION.dmg"
rm -rf build-dmg
# CLI as its own archive (for Homebrew later).
tar -czf "dist/spacesense-cli-$VERSION.tar.gz" -C app/build spacesense
(cd dist && shasum -a 256 * > SHA256SUMS)
ls -la dist && cat dist/SHA256SUMS
