#!/usr/bin/env bash
# Wrap hi5.app in a disk image: the app beside an Applications link, so
# installing is one drag. This is what the release publishes, next to a
# plain zip of the same bundle.
#
# Usage: dmg.sh <path/to/hi5.app> <out.dmg>
set -euo pipefail

APP="${1:?path to hi5.app}"
OUT="${2:?output .dmg}"
[ -d "$APP" ] || { echo "hi5: no bundle at $APP" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

rm -f "$OUT"
hdiutil create -quiet -volname "hi5" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
echo "$OUT"
