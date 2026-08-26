#!/usr/bin/env bash
# Stop hook: a turn does not end until its status has been spoken aloud.
# Every exit path other than the last one lets the turn end. Blocking is the
# exception, so a broken assumption here costs a missed reminder, not a wedged
# session.
set -uo pipefail

payload=$(cat)
field() { printf '%s' "$payload" | jq -r "$1 // empty" 2>/dev/null; }

# Already sent back once for this turn. Blocking again never terminates.
[ "$(field '.stop_hook_active')" = "true" ] && exit 0

# PATH is not the login shell's inside a hook, so resolve the binary directly
banshee="${BANSHEE_BIN:-@BANSHEE_BIN@}"
[ -x "$banshee" ] || banshee=$(command -v banshee 2>/dev/null)
# Nothing to speak through: let the turn end rather than demand the impossible
[ -n "$banshee" ] && "$banshee" status --json 2>/dev/null | jq -e '.running == true' >/dev/null || exit 0

transcript=$(field '.transcript_path')
prompt=$(field '.prompt_id')
[ -n "$transcript" ] && [ -f "$transcript" ] && [ -n "$prompt" ] || exit 0

# Every line of a turn that carries a prompt id carries this one: the user's
# own line and each tool result after it. So the next turn is the next id that
# differs, not merely the next id -- most of them belong to this turn.
start=$(grep -n "\"promptId\":\"$prompt\"" "$transcript" | head -1 | cut -d: -f1)
[ -n "$start" ] || exit 0
end=$(grep -n '"promptId":"' "$transcript" | grep -v "\"promptId\":\"$prompt\"" |
  awk -F: -v s="$start" '$1 > s { print $1; exit }')

if [ -n "$end" ]; then
  turn=$(sed -n "${start},$((end - 1))p" "$transcript")
else
  turn=$(tail -n +"$start" "$transcript")
fi

# Matched on the tool_use block, not the raw name: the tool definitions carried
# in the transcript spell the same names and would match a plain grep
spoken=$(printf '%s\n' "$turn" | jq -r '
  select(.type == "assistant")
  | .message.content[]?
  | select(.type == "tool_use")
  | .name' 2>/dev/null |
  grep -cE '^mcp__banshee__(speak_status|ask_user)$')

[ "${spoken:-0}" -gt 0 ] && exit 0

jq -n '{
  decision: "block",
  reason: ("You are about to end this turn without saying anything aloud. "
    + "The user is working eyes-free and is not reading the screen. "
    + "Speak your status now with the banshee speak_status tool, or ask_user "
    + "if you need an answer from them, then finish. Keep written output for "
    + "what must be read: code, paths, commands, tables.")
}'
