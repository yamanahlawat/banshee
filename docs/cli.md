# The commands

Most commands ask the running daemon over its socket. The ones that manage it,
such as `start`, `serve` and `tray`, do not. Nor do the ones that stand in for
it: `setup` fetches the models itself, `config set` writes the file, and
`connect` edits an agent's config directly. `banshee <command> --help` prints
this table in your terminal.

| Command                         | What it does                                               |
| ------------------------------- | ---------------------------------------------------------- |
| `banshee start`                 | Start the daemon, now and at every login                   |
| `banshee stop`                  | Stop the running daemon                                    |
| `banshee setup`                 | Download the required models                               |
| `banshee status`                | What Banshee is doing, and what stops it working           |
| `banshee status --json`         | The same as machine-readable state and blockers            |
| `banshee devices`               | List the microphones, and mark the one in use              |
| `banshee watch`                 | Follow what the daemon is doing, one line per change       |
| `banshee watch --waybar`        | The same, as Waybar custom-module JSON                     |
| `banshee voices`                | List the speech voices on disk, and mark the one in use    |
| `banshee config set <key> <value>` | Change one setting in `config.toml`                     |
| `banshee connect [agent]`       | Connect a coding agent, after showing the change           |
| `banshee serve`                 | Run the daemon in the foreground                           |
| `banshee tray`                  | Show the menu bar icon, now and at every login (macOS)     |
| `banshee tray --uninstall`      | Stop the menu bar icon and remove its launch agent         |
| `banshee service uninstall`     | Remove the start-at-login launch agents                    |
| `banshee listen`                | Print recent transcriptions                                |
| `banshee record start` / `stop` | Push-to-talk without the hotkey (for keybinds and scripts) |
| `banshee speak "<text>"`        | Speak some text aloud                                      |
| `banshee history`               | List all saved transcriptions                              |
| `banshee clear-history`         | Clear the saved transcriptions                             |

## Following what the daemon is doing

`banshee watch` prints one word per state change and keeps running:

```
$ banshee watch
idle
recording
idle
speaking
idle
```

The first line is the state at connect; the daemon pushes the rest as they
happen. The command exits non-zero when the daemon stops, so a supervisor can
restart it. For a single answer rather than a stream, ask `banshee status`.

