<p align="center">
  <img src="assets/banshee-icon.png" alt="banshee" width="104">
</p>

# Banshee

Banshee gives your AI coding agent a voice. Your agent - Claude Code, Copilot,
Cursor, or any MCP host - speaks its decisions and questions out loud, and you answer
back by talking, hands-free, while it works. It runs entirely on your machine:
local Whisper for listening, a local neural voice for speaking, no cloud, no
API keys, no audio ever leaving your laptop.

It's also a straight-up local dictation tool: hold a hotkey, speak, and the
text lands in whatever app you're focused on. Pure Rust, offline, always on.

## Demo

<https://github.com/user-attachments/assets/006132bd-9710-4322-a35a-4a5e5004371c>

The daemon running with the Pi coding agent. It asks which language to use,
hears "let's go with python", and writes the file. Nothing was typed.

## Why Banshee

Plenty of tools will transcribe your voice into an editor. Banshee is built for
the other half of the conversation.

- **Your agent asks, you answer.** `ask_user` speaks a question, waits for
  playback to finish, opens the microphone, and returns what you said, all in
  one call. Most voice tooling is one-directional dictation; this is a loop.
- **It never hears itself.** The microphone opens only after the question has
  finished playing, so the daemon can't transcribe its own voice. That's why
  Banshee works on laptop speakers without a headset.
- **Nothing leaves your machine.** Whisper, Silero VAD, and Kokoro all run
  locally. No API keys, no cloud tier, no audio uploaded, works on a plane.
- **It waits while you think.** Answers end on 2.5s of silence rather than the
  usual few hundred milliseconds, so pausing mid-sentence to think doesn't cut
  you off.
- **It handles your jargon in both directions.** `vocabulary` biases Whisper
  toward project words it would otherwise mangle, and the espeak-ng fallback
  pronounces unfamiliar terms instead of spelling them out letter by letter.
- **Not tied to one vendor.** It's an MCP server, so Claude Code, Copilot,
  Cursor, OpenCode, and anything else that speaks MCP all work.
- **One daemon, both jobs.** Agent voice and system-wide dictation share the
  same process, models, and microphone.

## Install

Runs on **macOS (Apple Silicon)** and **Linux** (x86_64 / aarch64); Intel Macs
aren't supported. Needs ~1 GB of disk for the models.

On Wayland the global hotkey needs your compositor's help and typing needs
`wtype` or `ydotool` — see [Wayland](#wayland). The agent voice works
everywhere, and `banshee status` reports what your session supports.

### Homebrew

```bash
brew install yamanahlawat/banshee/banshee
```

### Shell installer

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/yamanahlawat/banshee/releases/latest/download/banshee-installer.sh | sh
```

### From source

Needs Rust (stable, edition 2024):

```bash
git clone https://github.com/yamanahlawat/banshee.git
cd banshee
cargo install --path bansheed  # the `banshee` command and `banshee-mcp-shim`
```

## Setup

**1. Download the models** (~860 MB: Whisper, Silero VAD, Kokoro) into
`~/.banshee/models/`:

```bash
banshee setup
```

An interrupted download resumes where it stopped, and a re-run fetches only
what's missing — so after changing the STT preset or the TTS voice, run it
again.

**Optional: better pronunciation.** Install `espeak-ng` and Banshee pronounces
unfamiliar words (tech jargon, proper nouns) instead of spelling them out. On
macOS it's `brew install espeak-ng`; `banshee status` prints the command for
your system.

**2. Grant macOS permissions.** Banshee needs three, or it quietly fails to
record or type: **Microphone** (capture), **Input Monitoring** (the global
hotkey), and **Accessibility** (typing the transcription). macOS prompts for
each the first time Banshee needs it; approve, then restart the daemon —
permissions don't apply to an already-running process.

**3. Start Banshee, then check your setup:**

```bash
banshee start
banshee status
```

It reports on models, config, microphone, permissions, and daemon health, and
prints a fix for anything that's off. It never changes anything itself. Check
after starting, not before: a daemon that isn't running is one of the problems
it reports.

## Usage

The daemon runs at every login, restarts if it crashes, and logs to
`~/.banshee/daemon.log`. `banshee stop` pauses it until the next login;
`banshee serve` runs it in the foreground for debugging.

With the daemon running, the global hotkeys are:

- **Hold the hotkey** (Right Option by default) to record. On release, the
  transcription is typed straight into the app you're focused on (this is
  dictation mode).
- **Hold `Shift` + the hotkey** to record. On release, the transcription is
  saved, and you can grab it later with `banshee listen`.

To tap once to start and once to stop instead of holding, set
`hotkey_mode = "toggle"` under `[audio]`. Long dictations are easier that way,
and a session you forget about is still released by the push-to-talk watchdog.

The key is rebindable: `banshee config set audio.hotkey F6`, then
`banshee start`. Legal values are an F-key (`F1`–`F12`), a modifier alone
(`RightOption`, `LeftOption`, `LeftControl`, `LeftCommand`, plus
`RightCommand` and `Fn` on macOS), or modifiers and a key, as in `Ctrl+Alt+D`.
A modifier bound alone still works as a modifier: `RightOption+E` types é, and
banshee discards the accidental recording instead of transcribing it.

#### Binding an F-key on a Mac

macOS ships the top row as media keys, so a plain `F5` press starts Apple's
own Dictation and never reaches the daemon; hold `Fn` to send the real key. To
make F-keys single presses, turn on _Settings → Keyboard → "Use F1, F2, etc.
keys as standard function keys"_.

### Wayland

The global hotkey needs X11, so on a Wayland session (Hyprland, Sway, GNOME)
bind the record commands in your compositor instead. For push-to-talk on `F5`,
put these in `~/.config/hypr/hyprland.conf` (or `bindings.conf` on Omarchy),
then run `hyprctl reload`:

```conf
bind  = , F5, exec, banshee record start --dictate
bindr = , F5, exec, banshee record stop           # bindr fires on release
bind  = SHIFT, F5, exec, banshee record start
bindr = SHIFT, F5, exec, banshee record stop
```

Both release binds are there on purpose: Hyprland matches modifiers exactly,
and `Shift` may be released before `F5`.

Typing into the focused app needs **`wtype`** (wlroots compositors) or
**`ydotool`** (anywhere, with its own daemon and uinput access). Without one,
dictation reports an error and the transcription is kept in `banshee history`.
`banshee status` tells you which one it found.

The CLI commands all talk to the running daemon over its socket:

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

## Connect your coding agent

```bash
banshee connect            # which agents are installed, and which are connected
banshee connect antigravity # Antigravity IDE, agy CLI and SDK: the MCP server in ~/.gemini/config/mcp_config.json
banshee connect claude      # Claude Code: the MCP server and a stop hook that refuses to end a turn with no spoken status
banshee connect codex       # Codex CLI: the MCP server in ~/.codex/config.toml
banshee connect copilot     # GitHub Copilot CLI: the MCP server in ~/.copilot/mcp-config.json
banshee connect cursor      # Cursor: the MCP server in ~/.cursor/mcp.json
banshee connect opencode    # OpenCode: the MCP server
banshee connect pi          # Pi: the native extension
```

Each command shows the exact change to that tool's config and asks before it writes.
The Claude Code hook needs `jq` on your PATH. Antigravity, Claude Code, OpenCode
and Pi are verified on a real install; Codex, Copilot, and Cursor follow their
published config formats and wait for a report from a machine that has them.
Restart the tool afterwards.

`banshee-mcp-shim` is the MCP stdio server behind this. It exposes three tools:

| Tool                | What the agent does with it                                       |
| ------------------- | ----------------------------------------------------------------- |
| `speak_status`      | Say something aloud, for decisions made and work finished         |
| `ask_user`          | Ask a question aloud, then wait for and return your spoken answer |
| `listen_for_prompt` | Pick up anything you've said since it last checked                |

Any other MCP host takes the same `mcpServers` shape.

```json
{
  "mcpServers": {
    "banshee": {
      "command": "banshee-mcp-shim"
    }
  }
}
```

If the binary is not found, use its full path.

### Pi coding agent

Pi has its own extension API instead of MCP, so `banshee connect pi` installs a native
extension that talks to the daemon directly. See [integrations/pi](integrations/pi).

---

## Configuration

Configuration is optional. If you want to override the defaults, create
`~/.banshee/config.toml` (defaults shown here):

```toml
[daemon]
save_history = true    # keep transcriptions in ~/.banshee/banshee.db

[stt]
preset = "balanced"      # fast | balanced | quality (see below)
vad_threshold = 0.5      # 0.0 to 1.0; higher means stricter speech detection
vocabulary = ["banshee"] # words Whisper keeps mangling, e.g. ["clippy", "tokio"]
endpoint_silence_ms = 2500  # trailing silence that ends a spoken answer

[tts]
voice = "af_sky"       # any voice from the Kokoro voices directory
speed = 1.2            # playback speed multiplier
fallback = "system"    # system = use `say` when Kokoro is unavailable | none

[audio]
input_device = "default"  # "default" = follow the OS; otherwise match a device name
hotkey = "RightOption" # F1-F12, a lone modifier, or a chord like "Ctrl+Alt+D"
hotkey_mode = "hold"   # hold = record while the hotkey is down | toggle = tap to start, tap to stop
barge_in = "stop"      # stop = the record hotkey cuts off whatever Banshee is saying | none

[audio.cues]
enabled = true         # tones on record start/stop, success, and errors
```

`input_device` is a case-insensitive substring of the microphone name, so
`"yeti"` matches `Blue Yeti Stereo Microphone`. An exact name wins over a longer
name that contains it, so a `Yeti` next to a `Blue Yeti Pro` opens its own
device. `banshee devices` shows the names to choose from:

```
$ banshee devices
  Blue Yeti               system default, in use
  BlackHole 2ch
  MacBook Pro Microphone
```

**`"default"` follows the OS while Banshee runs.** Connect a headset, macOS makes
it the system default, and Banshee moves capture to it within about five seconds.
A device you name is not treated this way: Banshee opens the device you named,
and the system default never takes its place while it is present.

**A microphone that disappears does not stop dictation.** Unplug the headset you
named and Banshee records from the system default instead, so a press still
works. It says which microphone it moved to, and it says which one it is still
waiting for: the tray, `banshee status` and `banshee watch --waybar` all show
`MacBook Pro Microphone (waiting for "yeti")`. Reconnect the headset and Banshee
takes it back within about five seconds. Nothing needs a restart.

Banshee never picks a different microphone in silence. If it cannot open any
device at all, it says so and refuses to record rather than returning silence.

### Choosing a voice

`banshee voices` lists the voices on disk and marks the one the daemon loaded:

```
$ banshee voices
  af_heart
  af_sky    in use
  am_adam
  am_santa

Speak with one by: banshee config set tts.voice "<name>"
```

It lists only what is downloaded, so every name it prints works today. Nothing
is marked in use when Kokoro did not load: the system fallback speaks in
whatever voice macOS is set to, which Banshee did not choose.

### Following what the daemon is doing

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

### Showing the state in the macOS menu bar

```bash
banshee tray
```

An icon in the menu bar answers one question: can I speak right now. It comes
back at every login, and its menu names the state in words and the microphone
in use. Quit it from that menu, or remove it with `banshee tray --uninstall`.

| Idle | Recording | Speaking | Not running |
|:----:|:---------:|:--------:|:-----------:|
| <img src="assets/states/idle.png" width="52"> | <img src="assets/states/recording.png" width="52"> | <img src="assets/states/speaking.png" width="52"> | <img src="assets/states/notrunning.png" width="52"> |

The states differ by shape, never by colour alone, and the icon is a template
image, so macOS tints it to match the menu bar in light and dark.

### Showing the state in a Waybar module

`banshee watch --waybar` emits one Waybar custom-module object per line:

```json
{"text":"recording","alt":"recording","class":"recording","tooltip":"Banshee is recording. Microphone: Blue Yeti"}
```

`text` shows, `alt` picks a `format-icons` entry, and `class` is the CSS hook.
Put this in your Waybar config:

```jsonc
"custom/banshee": {
    "exec": "banshee watch --waybar",
    "return-type": "json",
    "restart-interval": 5,
    "format": "{icon}",
    "format-icons": { "idle": "mic", "recording": "REC", "speaking": "spk" }
}
```

and style it in your own CSS:

```css
#custom-banshee.recording { color: #e06c75; }
#custom-banshee.speaking  { color: #61afef; }
```

`restart-interval` matters: the command exits when the daemon stops, and that
is how the module reconnects once it comes back. The same channel is open to
any client over `banshee.subscribe`.

### Changing a setting without an editor

`banshee config set` writes one key and keeps your comments and layout:

```bash
banshee config set audio.hotkey RightOption
banshee config set stt.vad_threshold 0.7
banshee config set stt.vocabulary '["tokio", "clippy"]'
banshee config set audio.cues.enabled false
```

The key is the section and the field, as they appear in the file. A number, a
`true`, or a `[list]` is read as that type; anything else is read as text.
Quote twice to force text, as in `banshee config set audio.input_device '"12"'`.
A value the field does not accept is refused, and the message lists the legal
ones. `vad_threshold` and `audio.input_device` take effect immediately;
everything else is read once at startup, so the command tells you to restart.
This works whether or not the daemon is running.

`endpoint_silence_ms` is how long you can go quiet mid-answer before Banshee
decides you're done. Lower it if replies feel sluggish, raise it if you keep
getting cut off.

The `preset` picks which Whisper model Banshee uses:

| Preset     | Model                          | Trade-off                                  |
| ---------- | ------------------------------ | ------------------------------------------ |
| `fast`     | `ggml-base.en.bin`             | Fastest and lightest, English only         |
| `balanced` | `ggml-large-v3-turbo-q5_0.bin` | The default; accurate and reasonably quick |
| `quality`  | `ggml-large-v3-q5_0.bin`       | Most accurate, heaviest                    |

For `voice`, any file in the
[Kokoro voices directory](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/tree/main/voices)
works, e.g. `af_bella`, `am_michael`, or `bf_emma` (the prefix is
accent/gender: `a`merican/`b`ritish, `f`emale/`m`ale). After changing the
`preset` or `voice`, run `banshee setup` to download the new files, then restart
the daemon.

## Troubleshooting

Start with `banshee status`; it catches most setup problems and tells you the
fix. Beyond that:

- **The microphone looks dead: you record, and nothing ever comes back.**
  Usually the machine is slow, not broken: on an older CPU the `balanced`
  model can take minutes on a few seconds of speech. Run `banshee serve` and
  watch the `Transcribed` line; if it warns about slower-than-realtime, set
  `preset = "fast"` and run `banshee setup`. On a 2014 dual-core laptop that
  took one clip from 104s to 4.8s.
- **`banshee record start` says the microphone is busy.** A previous
  push-to-talk never got its `stop`. `banshee record stop` clears it; the
  daemon also releases the mic on its own after two minutes.
- **Audio sounds muffled on Bluetooth earbuds while Banshee runs.** macOS
  switches earbuds to their telephony profile while any app holds their mic.
  In **System Settings > Sound**, set _Input_ to the built-in microphone and
  leave _Output_ on the earbuds — the built-in mic transcribes better anyway.
- **Hotkeys or typing stopped working, but no error appears.** macOS withholds
  input events silently when an Input Monitoring or Accessibility grant is
  stale. Remove the Banshee entries from both lists in **System Settings >
  Privacy & Security**, restart the daemon, and approve the fresh prompts.
- **Permissions granted, but Banshee keeps asking.** Grants only apply to newly
  started processes. Restart the daemon with `banshee start`.

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev
setup and architecture.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
