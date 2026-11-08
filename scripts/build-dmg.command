#!/bin/zsh
# Build the universal app once, then package it without Finder automation.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != "Darwin" ]]; then
  print -u2 "ERROR: This script must run on macOS."
  exit 1
fi
for command in npm node cargo rustup xcode-select hdiutil ditto codesign lipo; do
  if ! command -v "$command" >/dev/null 2>&1; then
    print -u2 "ERROR: Missing $command."
    exit 1
  fi
done
if ! xcode-select -p >/dev/null 2>&1; then
  print -u2 "ERROR: Xcode Command Line Tools missing. Run: xcode-select --install"
  exit 1
fi

VERSION="$(node -p "require('./package.json').version")"
OUTPUT="$(pwd)/dist/NorthDriveDownloader-${VERSION}-universal-adhoc.dmg"
APP_ZIP="$(pwd)/dist/NorthDriveDownloader-${VERSION}-universal-adhoc-app.zip"
APP_BUNDLE="$(pwd)/src-tauri/target/universal-apple-darwin/release/bundle/macos/North Drive Downloader.app"
print "Building North Drive Downloader $VERSION — macOS Universal..."
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm install --no-audit --no-fund
# Ad-hoc signing is not Developer ID signing or notarization.
export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
npm run build -- --bundles app --target universal-apple-darwin --ci

BINARY="$APP_BUNDLE/Contents/MacOS/north-drive-downloader"
[[ -x "$BINARY" ]] || { print -u2 "ERROR: Compiled app binary missing."; exit 1; }
lipo -verify_arch arm64 x86_64 "$BINARY"
codesign --verify --deep --strict "$APP_BUNDLE"
mkdir -p dist

# Keep a usable app even if macOS disk-image creation fails.
ditto -c -k --sequesterRsrc --keepParent "$APP_BUNDLE" "$APP_ZIP"
print "App ZIP ready: $APP_ZIP"

# Unique staging and image paths avoid collisions with stale packaging resources.
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp/}north-dmg.XXXXXX")"
STAGE_DIR="$WORK_DIR/stage"
MOUNT_DIR="$WORK_DIR/mount"
MOUNTED=0
cleanup() {
  if [[ "$MOUNTED" -eq 1 ]]; then
    if ! hdiutil detach -quiet "$MOUNT_DIR"; then
      print -u2 "WARNING: Could not detach $MOUNT_DIR; leaving staging directory intact."
      return
    fi
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT
mkdir -p "$STAGE_DIR" "$MOUNT_DIR"
ditto "$APP_BUNDLE" "$STAGE_DIR/North Drive Downloader.app"
ln -s /Applications "$STAGE_DIR/Applications"

# Retry only image creation; never rebuild the Rust application for a busy disk image.
IMAGE_READY=0
for attempt in 1 2 3; do
  TEMP_DMG="$WORK_DIR/installer-$attempt.dmg"
  if hdiutil create -volname "North Drive Downloader" -srcfolder "$STAGE_DIR" \
      -fs HFS+ -format UDZO "$TEMP_DMG"; then
    IMAGE_READY=1
    break
  fi
  if [[ "$attempt" -lt 3 ]]; then
    print -u2 "Disk image attempt $attempt failed; retrying after $((attempt * 5)) seconds."
    sleep "$((attempt * 5))"
  fi
done
if [[ "$IMAGE_READY" -ne 1 ]]; then
  print -u2 "ERROR: DMG creation failed after 3 attempts. The verified app ZIP is available: $APP_ZIP"
  exit 1
fi

hdiutil verify "$TEMP_DMG"
hdiutil attach -quiet -readonly -nobrowse -mountpoint "$MOUNT_DIR" "$TEMP_DMG"
MOUNTED=1
if [[ ! -x "$MOUNT_DIR/North Drive Downloader.app/Contents/MacOS/north-drive-downloader" \
      || ! -L "$MOUNT_DIR/Applications" \
      || "$(readlink "$MOUNT_DIR/Applications")" != "/Applications" ]]; then
  print -u2 "ERROR: DMG missing the compiled app or Applications shortcut."
  exit 1
fi
hdiutil detach -quiet "$MOUNT_DIR"
MOUNTED=0
mv -f "$TEMP_DMG" "$OUTPUT"
print "SUCCESS: $OUTPUT"
print "Open the DMG, then drag North Drive Downloader into Applications."
if [[ -z "${CI:-}" ]]; then open dist; fi
