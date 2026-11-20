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
agents="$staging/Contents/Library/LaunchAgents"
mkdir -p "$macos" "$resources" "$agents"

# banshee-app is the bundle's executable. The other three run from inside it:
# launchd starts the daemon and the tray, and ~/.cargo/bin links the CLI.
# Each executable signs with its own identifier, the name of its launchd job.
# The bundle signature covers banshee-app as its executable, so it signs last.
executables="banshee:com.banshee.app.daemon banshee-tray:com.banshee.app.tray
banshee-mcp-shim:com.banshee.app.mcp-shim banshee-app:com.banshee.app"

for pair in $executables; do
    cp "$bindir/${pair%%:*}" "$macos/${pair%%:*}"
done

cp "$root/packaging/LaunchAgents/"*.plist "$agents/"
cp "$root/assets/banshee.icns" "$resources/banshee.icns"
sed "s/__VERSION__/$version/g" "$root/packaging/Info.plist" > "$staging/Contents/Info.plist"

sign() {
    codesign --force --options runtime --sign "$identity" "$@"
}

# --deep is unreliable and Apple deprecated it. Only the daemon captures audio,
# so only it gets the entitlement.
for pair in $executables; do
    name="${pair%%:*}"
    entitlements=()
    if [ "$name" = banshee ]; then
        entitlements=(--entitlements "$root/packaging/banshee.entitlements")
    fi
    sign --identifier "${pair#*:}" ${entitlements[@]+"${entitlements[@]}"} "$macos/$name"
done
sign "$staging"

codesign --verify --strict --verbose=2 "$staging"

expect_identifier() {
    local id
    id="$(codesign -dv "$macos/$1" 2>&1 | sed -n 's/^Identifier=//p')"
    [ "$id" = "$2" ] || {
        echo "$1 is not signed as $2 (codesign printed '$id')" >&2
        exit 1
    }
}
for pair in $executables; do
    expect_identifier "${pair%%:*}" "${pair#*:}"
done
for agent in daemon tray; do
    test -f "$agents/com.banshee.app.$agent.plist" || {
        echo "missing launch agent plist com.banshee.app.$agent.plist" >&2
        exit 1
    }
done

trap - EXIT
rm -rf "$app"
mv "$staging" "$app"
echo "built $app"
