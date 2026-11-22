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
resources="$staging/Contents/Resources"
agents="$staging/Contents/Library/LaunchAgents"
helper="$staging/Contents/Helpers/BansheeMenuBar.app"

# Builds into a staging path so a working install is only replaced once the
# new one is complete and verified.
trap 'rm -rf "$staging"' EXIT

# Columns: executable, its directory in the bundle, its signing identifier.
# The order is signing order. banshee-app is the bundle's executable, so the
# bundle signature covers it and it signs last.
executables="banshee Contents/MacOS com.banshee.app.daemon
banshee-mcp-shim Contents/MacOS com.banshee.app.mcp-shim
banshee-tray Contents/Helpers/BansheeMenuBar.app/Contents/MacOS com.banshee.app.tray
banshee-app Contents/MacOS com.banshee.app"

rm -rf "$staging"
mkdir -p "$resources" "$agents"

while read -r name dir _; do
    mkdir -p "$staging/$dir"
    cp "$bindir/$name" "$staging/$dir/$name"
done <<< "$executables"

cp "$root/packaging/LaunchAgents/"*.plist "$agents/"
cp "$root/assets/banshee.icns" "$resources/banshee.icns"
sed "s/__VERSION__/$version/g" "$root/packaging/Info.plist" > "$staging/Contents/Info.plist"
sed "s/__VERSION__/$version/g" "$root/packaging/MenuBar.Info.plist" > "$helper/Contents/Info.plist"

sign() {
    codesign --force --options runtime --sign "$identity" "$@"
}

# --deep is unreliable and Apple deprecated it. Only the daemon captures audio,
# so only it gets the entitlement.
while read -r name dir id; do
    entitlements=()
    if [ "$name" = banshee ]; then
        entitlements=(--entitlements "$root/packaging/banshee.entitlements")
    fi
    sign --identifier "$id" ${entitlements[@]+"${entitlements[@]}"} "$staging/$dir/$name"
    if [ "$name" = banshee-tray ]; then
        sign --identifier "$id" "$helper"
    fi
done <<< "$executables"
sign "$staging"

codesign --verify --strict --verbose=2 "$helper"
codesign --verify --strict --verbose=2 "$staging"

expect_identifier() {
    local id
    id="$(codesign -dv "$1" 2>&1 | sed -n 's/^Identifier=//p')"
    [ "$id" = "$2" ] || {
        echo "$1 is not signed as $2 (codesign printed '$id')" >&2
        exit 1
    }
}
while read -r name dir id; do
    expect_identifier "$staging/$dir/$name" "$id"
done <<< "$executables"
for agent in daemon tray; do
    test -f "$agents/com.banshee.app.$agent.plist" || {
        echo "missing launch agent plist com.banshee.app.$agent.plist" >&2
        exit 1
    }
done
for plist in "$agents"/*.plist; do
    program="$(/usr/libexec/PlistBuddy -c 'Print :BundleProgram' "$plist" 2> /dev/null)" || {
        echo "$(basename "$plist") has no BundleProgram" >&2
        exit 1
    }
    test -f "$staging/$program" && test -x "$staging/$program" || {
        echo "$(basename "$plist") names BundleProgram '$program', which is missing or not an executable file" >&2
        exit 1
    }
done

trap - EXIT
rm -rf "$app"
mv "$staging" "$app"
echo "built $app"
