#!/usr/bin/env bash
# Build hi5.app.
#
# Not a nicety. Two things only work from a real bundle:
#
#   * Notifications. macOS attributes every banner to a bundle
#     identifier, and a bare binary has none — see platform/notify.rs.
#   * Launch at login. `auto-launch` records the path it is given, and
#     the previous implementation shipped a login item pointing at a
#     debug binary in `target/`, which would have broken the first time
#     that directory was cleaned.
#
# `LSUIElement` is set for completeness even though GPUI overrides the
# activation policy at launch (mac/platform.rs:1390, which is why
# `platform::panel::become_accessory` exists) — a future GPUI that stops
# doing that should find the plist already correct.
set -euo pipefail

# Usage: bundle.sh [profile] [binary]
#   profile  which target/<profile>/hi5 to wrap (default: release)
#   binary   an explicit binary instead — the release workflow hands in
#            the universal one it made with lipo. The bundle lands next
#            to whichever binary was used.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${1:-release}"
BIN="${2:-$ROOT/target/$PROFILE/hi5}"
APP="$(dirname "$BIN")/hi5.app"

# One version, from the workspace manifest, so a tagged release cannot
# ship a bundle that reports a different number than its crate.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
[ -n "$VERSION" ] || { echo "hi5: no workspace version in $ROOT/Cargo.toml" >&2; exit 1; }

[ -x "$BIN" ] || { echo "no binary at $BIN — run: cargo build --profile $PROFILE -p hi5-gpui" >&2; exit 1; }

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/hi5"
cp "$ROOT/assets/hi5.icns" "$APP/Contents/Resources/hi5.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>hi5</string>
  <key>CFBundleDisplayName</key><string>hi5</string>
  <key>CFBundleIdentifier</key><string>com.hi5.app</string>
  <key>CFBundleExecutable</key><string>hi5</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>hi5</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature: unsigned bundles are refused a notification
# authorisation record, so banners would stay silent even from the app.
codesign --force --sign - "$APP" >/dev/null 2>&1 || \
  echo "hi5: could not ad-hoc sign $APP — notifications may stay silent" >&2

echo "$APP"
