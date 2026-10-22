#!/usr/bin/env bash
# Assembles Banshee.app from binaries that are already built, then signs it.
set -euo pipefail

bindir="${1:?usage: bundle.sh <binary-dir> <output-app> <identity> <version>}"
app="${2:?missing <output-app>}"
identity="${3:?missing <identity>}"
version="${4:?missing <version>}"

security find-identity -p codesigning -v | grep -q "$identity" || {
    echo "no codesigning identity '$identity'; see CONTRIBUTING.md" >&2
    exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
staging="$app.new"
macos="$staging/Contents/MacOS"
resources="$staging/Contents/Resources"

# Builds into a staging path so a working install is only replaced once the
# new one is complete and verified.
trap 'rm -rf "$staging"' EXIT

rm -rf "$staging"
mkdir -p "$macos" "$resources"

# banshee-app is the bundle's executable. The other three run from inside it:
# launchd starts the daemon and the tray, and ~/.cargo/bin links the CLI.
helpers="banshee-tray banshee-mcp-shim"
binaries="banshee-app banshee $helpers"

for binary in $binaries; do
    cp "$bindir/$binary" "$macos/$binary"
done

cp "$root/assets/banshee.icns" "$resources/banshee.icns"
sed "s/__VERSION__/$version/g" "$root/packaging/Info.plist" > "$staging/Contents/Info.plist"

sign() {
    codesign --force --options runtime --sign "$identity" "$@"
}

# The bundle signature covers banshee-app as its executable, so it comes last.
# --deep is unreliable and Apple deprecated it. Only the daemon captures audio,
# so only it gets the entitlement.
sign --identifier "com.banshee.app" \
    --entitlements "$root/packaging/banshee.entitlements" "$macos/banshee"
for binary in $helpers; do
    sign --identifier "com.banshee.app" "$macos/$binary"
done
sign "$staging"

codesign --verify --strict --verbose=2 "$staging"

trap - EXIT
rm -rf "$app"
mv "$staging" "$app"
echo "built $app"
