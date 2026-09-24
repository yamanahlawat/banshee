# Claude Code

A Stop hook sends a turn that ended in silence back once, to speak its status.

```bash
banshee connect claude
```

It writes this entry into `settings.json` in `$CLAUDE_CONFIG_DIR` (by default
`~/.claude`), after it shows the change:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/bin/sh -c 'b=\"$0\"; [ -x \"$b\" ] || b=$(command -v banshee) || exit 0; \"$b\" turn-end claude || exit 0' /Applications/Banshee.app/Contents/MacOS/banshee",
            "timeout": 15,
            "statusMessage": "Checking you spoke"
          }
        ]
      }
    ]
  }
}
```

- The daemon counts each Claude Code session's `speak_status` and `ask_user` calls. The hook
  asks it whether the session spoke since its last turn ended.
- It sends a turn back once. A second silent stop in the same turn ends it.
- Any failure lets the turn end: Banshee removed, the daemon stopped, no answer in two seconds.

Where it falls short:

- A subagent's speech counts for the session that started it.
- An interrupted turn fires no Stop, so the next silent turn can pass.
