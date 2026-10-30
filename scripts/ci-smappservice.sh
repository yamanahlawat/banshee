#!/usr/bin/env bash
# Proves the login-item path against an installed, signed Banshee.app: the
# migration from legacy launchd jobs, registration, restart, stop, start and
# uninstall. Each check prints PASS or FAIL with its name, and the first FAIL
# ends the run.
set -euo pipefail

# It bootstraps launchd jobs and `banshee uninstall` deletes the app, so it runs
# only on a disposable runner.
if [ "${CI:-}" != true ]; then
    echo "ci-smappservice.sh runs only on a CI runner" >&2
    exit 1
fi

app="${1:-/Applications/Banshee.app}"
macos="$app/Contents/MacOS"
banshee="$macos/banshee"
domain="gui/$(id -u)"
daemon_label=com.banshee.app.daemon
tray_label=com.banshee.app.tray
legacy_labels=(com.banshee.daemon com.banshee.tray)
launch_agents="$HOME/Library/LaunchAgents"
socket="$HOME/.banshee/banshee.sock"
daemon_log="$HOME/Library/Logs/com.banshee.app/daemon.log"
kept="${RUNNER_TEMP:?}/Banshee.app.kept"

# launchd.plist(5), ThrottleInterval: "by default, jobs will not be spawned more
# than once every 10 seconds". A job that exits sooner than that after it
# started is relaunched only once the 10 seconds have passed.
throttle=10
# A daemon that starts while the one leaving still holds the socket exits, and
# launchd relaunches it once the throttle has passed.
start_wait=$((2 * throttle))

pass() { echo "PASS $1"; }
fail() {
    echo "FAIL $1${2:+: $2}"
    exit 1
}

# Runs a command with its output shown, and passes or fails on its exit code.
step() {
    local name="$1" code=0
    shift
    echo "--- $*"
    "$@" || code=$?
    [ "$code" -eq 0 ] || fail "$* exited $code"
    pass "$name"
}

login_items() {
    "$macos/banshee-app" --login-items "$@"
}

loaded() {
    launchctl print "$domain/$1" > /dev/null 2>&1
}

daemon_print() {
    launchctl print "$domain/$daemon_label" 2> /dev/null || true
}

# The pid launchd holds for the daemon job, or nothing when it is not running.
daemon_pid() {
    daemon_print |
        awk '$1 == "pid" && $2 == "=" && !found { print $3; found = 1 }'
}

# The status the daemon job's last process exited with, as launchd records it.
daemon_last_exit() {
    daemon_print |
        awk '$1 == "last" && $2 == "exit" && $3 == "code" && $4 == "=" && !found {
            sub(":.*", "", $5); print $5; found = 1
        }'
}

expect_last_exit() {
    local after="$1" want="$2" got
    got="$(daemon_last_exit)"
    [ "$got" = "$want" ] ||
        fail "the daemon's last exit code after $after is '$got'" "expected $want"
    pass "the daemon's last exit code after $after is $want"
}

# `banshee status --json` prints {"running":false} when no daemon answers
# banshee.status, and the daemon's reply when one does.
answers() {
    local reply
    reply="$("$banshee" status --json 2> /dev/null)" || true
    [ -n "$reply" ] && [ "$reply" != '{"running":false}' ]
}

seconds_running() {
    ps -o etime= -p "$1" | awk -F '[-:]' '{
        n = NF; s = $n + 60 * $(n - 1)
        if (n >= 3) s += 3600 * $(n - 2)
        if (n >= 4) s += 86400 * $(n - 3)
        print s
    }'
}

# Returns 0 once the registered daemon answers, within `$1` seconds.
wait_for_answer() {
    local deadline=$((SECONDS + $1))
    while [ "$SECONDS" -le "$deadline" ]; do
        if [ -n "$(daemon_pid)" ] && answers; then
            return 0
        fi
        sleep 1
    done
    return 1
}

# Prints the pid of a daemon other than `$1` that answers, within `$2` seconds.
# Returns 1 when none does.
wait_for_new_daemon() {
    local old="$1" deadline=$((SECONDS + $2)) pid
    while [ "$SECONDS" -le "$deadline" ]; do
        pid="$(daemon_pid)"
        if [ -n "$pid" ] && [ "$pid" != "$old" ] && answers; then
            echo "$pid"
            return 0
        fi
        sleep 1
    done
    return 1
}

# The 2 seconds are what `banshee status` waits for a reply.
send_restart() {
    python3 - "$socket" << 'PY'
import json, socket, sys
with socket.socket(socket.AF_UNIX) as connection:
    connection.settimeout(2)
    connection.connect(sys.argv[1])
    connection.sendall(b'{"jsonrpc":"2.0","id":1,"method":"banshee.restart"}\n')
    line = connection.makefile().readline()
print(line.strip())
sys.exit(0 if json.loads(line).get("result", {}).get("ok") is True else 1)
PY
}

expect_states() {
    local name="$1" states="$2"
    shift 2
    local label_state
    for label_state in "$@"; do
        grep -qx "$label_state" <<< "$states" ||
            fail "no registration line '$label_state'" "status printed: $states"
    done
    pass "$name"
}

# `banshee uninstall` deletes the app, so a copy reads the registrations after it.
rm -rf "$kept"
ditto "$app" "$kept"

echo "=== banshee-app --login-items with no window"
errors="$RUNNER_TEMP/login-items.err"
code=0
states="$(login_items status all 2> "$errors")" || code=$?
echo "$states"
if [ "$code" -ne 0 ]; then
    echo "banshee-app --login-items status all exited $code; its stderr, verbatim:"
    cat "$errors"
    fail "banshee-app --login-items status all exited $code"
fi
pass "banshee-app --login-items status runs from a terminal"

echo "=== legacy jobs, written the way earlier releases wrote them"
mkdir -p "$launch_agents" "$HOME/.banshee"
for label in "${legacy_labels[@]}"; do
    case "$label" in
        com.banshee.daemon)
            arguments="<string>$banshee</string><string>serve</string>"
            log="$HOME/.banshee/daemon.log"
            ;;
        com.banshee.tray)
            arguments="<string>$macos/banshee-tray</string>"
            log="$HOME/.banshee/tray.log"
            ;;
    esac
    plist="$launch_agents/$label.plist"
    cat > "$plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$label</string>
    <key>ProgramArguments</key>
    <array>$arguments</array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>$log</string>
    <key>StandardErrorPath</key>
    <string>$log</string>
</dict>
</plist>
EOF
    step "legacy job $label bootstraps" launchctl bootstrap "$domain" "$plist"
done

echo "=== banshee start, then banshee tray"
step "banshee start migrates and registers" "$banshee" start
step "banshee tray registers" "$banshee" tray
deadline=$((SECONDS + throttle))
until pgrep -x banshee-tray > /dev/null || [ "$SECONDS" -gt "$deadline" ]; do
    sleep 1
done
pgrep -x banshee-tray > /dev/null ||
    fail "no banshee-tray process within ${throttle}s of banshee tray"
pass "the tray runs (pid $(pgrep -x banshee-tray | tr '\n' ' '))"
expect_states "both agents are enabled" "$(login_items status all)" \
    "$daemon_label enabled" "$tray_label enabled"
for label in "${legacy_labels[@]}"; do
    [ ! -e "$launch_agents/$label.plist" ] || fail "legacy plist $label.plist remains"
    pass "legacy plist $label.plist is gone"
    ! loaded "$label" ||
        fail "legacy job $label is still loaded" "$(launchctl print "$domain/$label" 2>&1 | head -20)"
    pass "legacy job $label is not loaded"
done
wait_for_answer "$start_wait" ||
    fail "no registered daemon answered banshee.status within ${start_wait}s"
pass "the registered daemon answers banshee.status (pid $(daemon_pid))"
grep -q "Listening on $socket" "$daemon_log" ||
    fail "$daemon_log lacks the daemon's start line"
pass "$daemon_log holds the daemon's start line"

echo "=== banshee start, registered and running"
running_pid="$(daemon_pid)"
[ -n "$running_pid" ] || fail "no daemon pid before banshee start while running"
start_sent=$SECONDS
step "banshee start while running" "$banshee" start
replaced_pid="$(wait_for_new_daemon "$running_pid" $((start_sent + start_wait - SECONDS)))" || true
elapsed=$((SECONDS - start_sent))
[ -n "$replaced_pid" ] ||
    fail "no new daemon answered within ${start_wait}s of banshee start while running" "pid $running_pid, ${elapsed}s elapsed"
pass "a new daemon answered ${elapsed}s after banshee start while running (pid $running_pid, then $replaced_pid)"
for label in "${legacy_labels[@]}"; do
    [ ! -e "$launch_agents/$label.plist" ] ||
        fail "legacy plist $label.plist is back after banshee start while running"
    pass "legacy plist $label.plist stays gone"
done

echo "=== banshee.restart"
old_pid="$(daemon_pid)"
[ -n "$old_pid" ] || fail "no daemon pid before banshee.restart"
while [ "$(seconds_running "$old_pid")" -lt "$throttle" ]; do
    sleep 1
done
restart_sent=$SECONDS
send_restart || fail "banshee.restart got no ok reply"
pass "banshee.restart is answered"
new_pid="$(wait_for_new_daemon "$old_pid" $((restart_sent + throttle - SECONDS)))" || true
elapsed=$((SECONDS - restart_sent))
[ -n "$new_pid" ] ||
    fail "no new daemon answered within ${throttle}s of banshee.restart" "pid $old_pid, ${elapsed}s elapsed"
pass "a new daemon answered ${elapsed}s after banshee.restart (pid $old_pid, then $new_pid)"
expect_last_exit banshee.restart 75

echo "=== banshee stop"
step "banshee stop" "$banshee" stop
deadline=$((SECONDS + throttle))
while [ -n "$(daemon_pid)" ] && [ "$SECONDS" -le "$deadline" ]; do
    sleep 1
done
[ -z "$(daemon_pid)" ] ||
    fail "the daemon still runs ${throttle}s after banshee stop" "pid $(daemon_pid)"
pass "the daemon leaves after banshee stop"
expect_last_exit "banshee stop" 0
# A relaunch, were launchd to make one, comes within the throttle.
deadline=$((SECONDS + throttle))
while [ "$SECONDS" -le "$deadline" ]; do
    pid="$(daemon_pid)"
    [ -z "$pid" ] || fail "launchd relaunched the daemon after banshee stop" "pid $pid"
    ! answers || fail "a daemon answered after banshee stop"
    sleep 1
done
pass "the daemon stays down for ${throttle}s"

echo "=== banshee start, registered but stopped"
step "banshee start again" "$banshee" start
wait_for_answer "$start_wait" ||
    fail "no daemon answered within ${start_wait}s of banshee start again"
pass "the daemon answers after banshee start again (pid $(daemon_pid))"

echo "=== banshee uninstall"
step "banshee uninstall --yes" "$banshee" uninstall --yes
[ -e "$app" ] || ditto "$kept" "$app"
expect_states "both agents are not registered" "$(login_items status all)" \
    "$daemon_label not-registered" "$tray_label not-registered"
for label in "$daemon_label" "$tray_label"; do
    ! loaded "$label" || fail "job $label is still loaded after uninstall"
    pass "job $label is not loaded after uninstall"
done
! answers || fail "a daemon answered after uninstall"
pass "no daemon answers after uninstall"
