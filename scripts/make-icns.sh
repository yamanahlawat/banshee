#!/usr/bin/env bash
# Builds an .icns from the committed 1024 asset with macOS built-ins only.
# The source is the full-bleed app artwork; macOS masks the tile itself.
set -euo pipefail

out="${1:?usage: make-icns.sh <output.icns>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/assets/banshee-icon-app-1024.png"

[ -f "$src" ] || { echo "missing $src; see scripts/make-icns.sh header" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
set="$work/banshee.iconset"
mkdir -p "$set"

# iconutil requires exactly these names.
for pair in "16 icon_16x16" "32 icon_16x16@2x" "32 icon_32x32" "64 icon_32x32@2x" \
            "128 icon_128x128" "256 icon_128x128@2x" "256 icon_256x256" \
            "512 icon_256x256@2x" "512 icon_512x512" "1024 icon_512x512@2x"; do
    size="${pair%% *}"
    name="${pair##* }"
    sips -z "$size" "$size" "$src" --out "$set/$name.png" >/dev/null
done

iconutil --convert icns --output "$out" "$set"
echo "wrote $out"
