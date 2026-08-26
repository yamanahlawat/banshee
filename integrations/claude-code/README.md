# Claude Code

`banshee-speak-check.sh` is a Stop hook. It reads the turn that is about to end
and looks for a call to the `speak_status` or `ask_user` tool of the Banshee MCP
server. If the turn made neither call, the hook blocks the end of the turn and
tells the agent to speak. Every other path lets the turn end, so a broken
assumption costs a missed reminder, not a stuck session.

## Install

```bash
banshee connect claude
```

That writes the script into `$CLAUDE_CONFIG_DIR/hooks/` and registers it in
`settings.json`, after it shows you both changes.

## Install by hand

Copy `banshee-speak-check.sh` into `$CLAUDE_CONFIG_DIR/hooks/` (by default
`~/.claude/hooks/`) and make it executable. Replace `@BANSHEE_BIN@` in the script
with the absolute path of your `banshee` binary. Then add the hook to
`settings.json`:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bash '/Users/you/.claude/hooks/banshee-speak-check.sh'",
            "timeout": 15,
            "statusMessage": "Checking you spoke"
          }
        ]
      }
    ]
  }
}
```

## Requirement

The script reads the hook payload and the transcript with `jq`. Put `jq` on your
PATH, or the hook exits without a check.
