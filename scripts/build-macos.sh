#!/bin/bash
# Builds MyCCTV for macOS (Mac mini, MacBook, iMac...) as one "universal" app that runs on both
# Apple-silicon (M1/M2/M3/M4) and Intel Macs, and packs it into a .dmg.
#
# Run on a Mac, from the app folder:   bash scripts/build-macos.sh
# Needs: Xcode command line tools (xcode-select --install) and Rust (https://rustup.rs).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=release
APP="$OUT/MyCCTV.app"

echo "== MyCCTV $VERSION for macOS"
rustup target add aarch64-apple-darwin x86_64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=11.0
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

echo "== Making the app bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/mycctv" \
    target/aarch64-apple-darwin/release/mycctv \
    target/x86_64-apple-darwin/release/mycctv
sed "s/__VERSION__/$VERSION/g" macos/Info.plist > "$APP/Contents/Info.plist"
cp assets/app.icns "$APP/Contents/Resources/app.icns"

# Ad-hoc signature so macOS runs it on this Mac. For other Macs without warnings, sign with a
# "Developer ID Application" certificate and notarize (see DEV_GUIDE.md).
codesign --force --deep --sign - "$APP"

echo "== Making the disk image"
DMG="$OUT/MyCCTV-$VERSION-macos-universal.dmg"
rm -f "$DMG"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "MyCCTV" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

echo "== Done: $APP and $DMG"
