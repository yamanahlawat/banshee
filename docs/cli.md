# The commands

- Every command, with what it does.
- Most commands ask the running daemon over its socket.
- `setup` and `config set` ask it too, and fetch the models or write the file
  themselves only when no daemon runs.
- `start`, `serve` and `tray` manage the daemon.
- `connect` and `bind` edit another tool's config directly.
- `banshee --help` prints this table in your terminal.

| Command                         | What it does                                               |
| ------------------------------- | ---------------------------------------------------------- |
| `banshee start`                 | Start the daemon, now and at every login (on macOS as a Login Item); it registers again if already on and names any model that is missing |
| `banshee stop`                  | Stop the running daemon                                    |
| `banshee setup`                 | Download the required models; a re-run fetches only what is missing         |
| `banshee setup --model <id>`    | Download one listening model by its id; then `banshee config set stt.model <id>` listens with it |
| `banshee setup --model <id> --accept-licence` | The same for a model whose licence asks for acceptance; `setup --model` alone shows its terms and link |
| `banshee setup --list`          | List every listening model: id, size, languages, whether it streams, and its licence |
| `banshee status`                | What Banshee is doing, and what stops it working           |
| `banshee status --json`         | The same as machine-readable state and blockers            |
| `banshee devices`               | List the microphones, and mark the one in use              |
| `banshee watch`                 | Follow what the daemon is doing, one line per change       |
| `banshee watch --waybar`        | The same, as Waybar custom-module JSON                     |
| `banshee watch --events cues,level,words` | Print each cue, level and words event as one JSON line |
| `banshee voices`                | List the speech voices on disk, and mark the one in use    |
| `banshee config set <key> <value>` | Change one setting in `config.toml`                     |
| `banshee config remote`         | Set up the remote listener and speaker in one run, after showing each value |
| `banshee connect [agent]`       | Connect a coding agent, after showing the change           |
| `banshee bind [hyprland]`       | Bind the record keys in your compositor's config, after showing the change. It asks for the key and offers `audio.hotkey`, or `F9` when that is a lone modifier; alone, print the snippet and name the file it would write |
| `banshee serve`                 | Run the daemon in the foreground                           |
| `banshee tray`                  | Show the tray icon, now and at every login; registers again if already on                 |
| `banshee record start` / `stop` / `toggle` | Push-to-talk without the hotkey; `toggle` is one key that starts, then stops |
| `banshee speak "<text>"`        | Speak some text aloud                                      |
| `banshee history`               | List all saved transcriptions                              |
| `banshee clear-history`         | Clear the saved transcriptions                             |
| `banshee uninstall [--data] [--yes]` | Stop Banshee and remove its login services (on macOS, its Login Items). It lists the files it would delete and asks first; for a Homebrew install it names the `brew` command that removes the rest. `--data` also deletes `~/.banshee` and, on macOS, `~/Library/Logs/com.banshee.app`, and `--yes` skips the question |

## Following what the daemon is doing

- `banshee watch` prints one word per state change, and keeps running.
- `banshee watch --events cues,level,words` prints each event as one JSON line.
  A words event carries the words heard so far while you speak, and an empty
  text when the recording ends. The one-word lines and `--waybar` stay as they
  are.

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
- `busy` means Banshee transcribes what you said. It needs nothing from you.
- The command exits non-zero when the daemon stops, so a supervisor can
  restart it.
- For a single answer, not a stream, ask `banshee status`.
