#!/usr/bin/env bash
# Expects the app that ci-smappservice.sh leaves behind: build A, with no registrations.
# GitHub's macOS runner has not reproduced macOS refusing the first launch, so
# this checks only that an upgraded build runs after one start.
set -euo pipefail

# It rewrites Cargo.toml, builds into the app path and registers login items,
# so it runs only on a disposable runner.
if [ "${CI:-}" != true ]; then
    echo "ci-upgrade.sh runs only on a CI runner" >&2
    exit 1
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app="${1:-/Applications/Banshee.app}"
macos="$app/Contents/MacOS"
banshee="$macos/banshee"
identity=banshee-ci

# launchd.plist(5), ThrottleInterval: a job is spawned at most once every 10
# seconds, so a launch that fails is retried after the throttle.
throttle=10
start_wait=$((2 * throttle))

pass() { echo "PASS $1"; }
fail() {
    echo "FAIL $1${2:+: $2}"
    exit 1
}

step() {
    local name="$1" code=0
    shift
    echo "--- $*"
    "$@" || code=$?
    [ "$code" -eq 0 ] || fail "$* exited $code"
    pass "$name"
}

status_version() {
    "$banshee" status --json 2> /dev/null | jq -r '.version // empty' 2> /dev/null || true
}

cdhash() {
    codesign -dv --verbose=3 "$macos/$1" 2>&1 | sed -n 's/^CDHash=//p'
}

wait_for_tray_since() {
    local since="$1" deadline=$((SECONDS + $2)) pid started
    while :; do
        for pid in $(pgrep -x banshee-tray || true); do
            started="$(LC_ALL=C date -j -f '%a %b %e %T %Y' "$(ps -o lstart= -p "$pid")" +%s 2> /dev/null)" || continue
            [ "$started" -lt "$since" ] || return 0
        done
        [ "$SECONDS" -lt "$deadline" ] || return 1
        sleep 1
    done
}

wait_for_version() {
    local deadline=$((SECONDS + $2))
    while :; do
        [ "$(status_version)" != "$1" ] || return 0
        [ "$SECONDS" -lt "$deadline" ] || return 1
        sleep 1
    done
}

# The workspace version is the first `version` line in Cargo.toml and every
# crate inherits it, so one edit changes every binary.
version_a="$(grep -m1 '^version' "$root/Cargo.toml" | cut -d'"' -f2)"
version_b="$version_a-upgrade"
trap 'git -C "$root" checkout -- Cargo.toml Cargo.lock' EXIT

echo "=== build A runs"
step "banshee start on build A" "$banshee" start
step "banshee tray on build A" "$banshee" tray
wait_for_version "$version_a" "$start_wait" ||
    fail "build A's daemon did not answer with version $version_a within ${start_wait}s"
wait_for_tray_since 0 "$start_wait" ||
    fail "no banshee-tray process within ${start_wait}s of banshee tray"
pass "build A runs (version $version_a)"
hash_a_daemon="$(cdhash banshee)"
hash_a_tray="$(cdhash banshee-tray)"
[ -n "$hash_a_daemon" ] && [ -n "$hash_a_tray" ] || fail "build A has no CDHash"

echo "=== build B over the running install"
awk -v version="$version_b" '!done && /^version/ { print "version = \"" version "\""; done = 1; next } { print }' \
    "$root/Cargo.toml" > "$root/Cargo.toml.new"
mv "$root/Cargo.toml.new" "$root/Cargo.toml"
cd "$root"
cargo build --release --workspace --exclude banshee-app
# Last, and never followed by a plain `cargo build`: see the Makefile.
(cd "$root/banshee-app" && cargo tauri build --no-bundle)
"$root/scripts/bundle.sh" "$root/target/release" "$app" "$identity" "$version_b"
[ "$(cdhash banshee)" != "$hash_a_daemon" ] &&
    [ "$(cdhash banshee-tray)" != "$hash_a_tray" ] ||
    fail "build B differs from build A"
pass "build B differs from build A"

echo "=== one banshee start and one banshee tray on build B"
start_sent=$SECONDS
step "banshee start on build B" "$banshee" start
wait_for_version "$version_b" "$start_wait" ||
    fail "the daemon did not report version $version_b within ${start_wait}s after banshee start returned" "it reports '$(status_version)'"
pass "the daemon reports version $version_b, $((SECONDS - start_sent))s after banshee start was sent"
tray_epoch="$(date +%s)"
step "banshee tray on build B" "$banshee" tray
wait_for_tray_since "$tray_epoch" "$start_wait" ||
    fail "no banshee-tray process started within ${start_wait}s after banshee tray returned"
pass "a banshee-tray process started after banshee tray"

if log show --last 5m --predicate 'process == "launchd" AND eventMessage CONTAINS "Launch Constraint Violation"' 2> /dev/null |
    grep -q com.banshee.app; then
    echo "the first launch hit the launch constraint: yes"
else
    echo "the first launch hit the launch constraint: no"
fi
