# The commands

- Every command, with what it does.
- Most commands ask the running daemon over its socket.
- `setup` and `config set` ask it too, and fetch the models or write the file
  themselves only when no daemon runs.
- `start`, `serve`, `tray` and `service` manage the daemon.
- `connect` and `bind` edit another tool's config directly.
- `tell` runs your coding agent itself, and that agent edits your config.
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
| `banshee tell "<text>"`         | Send a command to your coding agent, which changes your desktop |
| `banshee tell --undo`           | Put the watched folders back from the newest snapshot      |
| `banshee history`               | List all saved transcriptions                              |
| `banshee clear-history`         | Clear the saved transcriptions                             |

## Telling your agent to change the desktop

- `banshee tell "make the window gaps bigger"` runs your coding agent with
  those words.
- Banshee sends your words and nothing else. The agent's own skills carry the
  desktop knowledge.
- The agent edits the config, and speaks the result through Banshee.
- Banshee itself says nothing. `banshee tell` prints what the agent may edit
  before it starts.
- The tell key sounds no cue of its own. A tell recording ends on the same
  record-stop cue as a dictation.
- A failed run sounds the error cue. `banshee status` names the reason.
- The menu bar icon shows Busy for the length of the run, so you can see that
  the agent still works.
- `banshee tell --undo` puts the watched folders back from the newest snapshot.
- The folders, the agent and the timeouts are settings. See
  [configuration.md](configuration.md#telling-your-agent).
- The tell key runs the same command from any window. See
  [linux.md](linux.md#the-key-on-wayland).

Two phrases Banshee answers itself. It hands neither to the agent:

- `banshee tell "start over"` ends the thread, and starts no agent.
- `banshee tell "show me"` reopens the thread in a terminal, so you can read
  what the agent wrote.
- Each phrase must be the whole sentence. Case and a trailing `.` or `!` are
  ignored.
- A longer sentence goes to the agent, so `show me a list of themes` is a
  command.

## Following what the daemon is doing

- `banshee watch` prints one word per state change, and keeps running.

```
$ banshee watch
idle
recording
busy
idle
speaking
idle
```

- The first line is the state at connect.
- The daemon pushes the rest as they happen.
- The words are `idle`, `recording`, `busy`, `speaking` and `listening`.
- `busy` means Banshee transcribes what you said, or the agent a `banshee tell`
  started still runs. Neither one needs you.
- The command exits non-zero when the daemon stops, so a supervisor can
  restart it.
- For a single answer, not a stream, ask `banshee status`.
