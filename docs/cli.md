# The commands

- Every command, with what it does.
- Most commands ask the running daemon over its socket.
- `setup` and `config set` ask it too, and fetch the models or write the file
  themselves only when no daemon runs.
- `start`, `serve`, `tray` and `service` manage the daemon.
- `connect` and `bind` edit another tool's config directly.
- `banshee <command> --help` prints this table in your terminal.

| Command                         | What it does                                               |
| ------------------------------- | ---------------------------------------------------------- |
| `banshee start`                 | Start the daemon, now and at every login, and download the models it is missing |
| `banshee stop`                  | Stop the running daemon                                    |
| `banshee setup`                 | Download the required models; a re-run fetches only what is missing         |
| `banshee status`                | What Banshee is doing, and what stops it working           |
| `banshee status --json`         | The same as machine-readable state and blockers            |
| `banshee devices`               | List the microphones, and mark the one in use              |
| `banshee watch`                 | Follow what the daemon is doing, one line per change       |
| `banshee watch --waybar`        | The same, as Waybar custom-module JSON                     |
| `banshee voices`                | List the speech voices on disk, and mark the one in use    |
| `banshee config set <key> <value>` | Change one setting in `config.toml`                     |
| `banshee config remote`         | Set up the remote listener and speaker in one run, after showing each value |
| `banshee connect [agent]`       | Connect a coding agent, after showing the change           |
| `banshee bind [hyprland]`       | Bind F9 in your compositor's config, after showing the change; alone, print the snippet and name the file it would write |
| `banshee serve`                 | Run the daemon in the foreground                           |
| `banshee tray`                  | Show the menu bar icon, now and at every login (macOS)     |
| `banshee tray --uninstall`      | Stop the menu bar icon and remove its launch agent         |
| `banshee service uninstall`     | Remove the start-at-login launch agents                    |
| `banshee listen`                | Print recent transcriptions                                |
| `banshee record start` / `stop` / `toggle` | Push-to-talk without the hotkey; `toggle` is one key that starts, then stops |
| `banshee speak "<text>"`        | Speak some text aloud                                      |
| `banshee history`               | List all saved transcriptions                              |
| `banshee clear-history`         | Clear the saved transcriptions                             |

## Following what the daemon is doing

- `banshee watch` prints one word per state change, and keeps running.

```
$ banshee watch
idle
recording
idle
speaking
idle
```

- The first line is the state at connect.
- The daemon pushes the rest as they happen.
- The command exits non-zero when the daemon stops, so a supervisor can
  restart it.
- For a single answer, not a stream, ask `banshee status`.
