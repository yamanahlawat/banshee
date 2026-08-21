# Banshee

Banshee gives your AI coding agent a voice. Your agent - Claude Code, Cursor,
or any MCP host - speaks its decisions and questions out loud, and you answer
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
- **Not tied to one vendor.** It's an MCP server, so Claude Code, Cursor,
  OpenCode, and anything else that speaks MCP all work.
- **One daemon, both jobs.** Agent voice and system-wide dictation share the
  same process, models, and microphone.

## Install

Runs on **macOS (Apple Silicon)** and **Linux** (x86_64 / aarch64); Intel Macs
aren't supported. Needs ~1 GB of disk for the models.

On Linux, the **global hotkey** goes through X11; the library Banshee listens
with has no Wayland path, so under Wayland `F5` does nothing. Bind
`banshee record start` / `stop` in your compositor instead (see
[Wayland](#wayland)). Dictation typing does work under Wayland if `wtype` or
`ydotool` is installed. The agent voice (`speak_status`, `ask_user`) is
unaffected either way, and `banshee status` reports what your session supports.

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

**1. Download the models** into `~/.banshee/models/`:

```bash
banshee setup
```

| File                           | Size    | What it is                                    |
| ------------------------------ | ------- | --------------------------------------------- |
| `ggml-large-v3-turbo-q5_0.bin` | ~547 MB | Whisper STT model (default `balanced` preset) |
| `silero_vad.onnx`              | ~2 MB   | Silero voice-activity detection               |
| `kokoro-v1.0.onnx`             | ~310 MB | Kokoro TTS model                              |
| `af_sky.bin`                   | ~512 KB | Kokoro voice style (the configured `voice`)   |

Files that already exist are skipped, so re-running `banshee setup` after
changing the STT preset or the TTS voice only downloads what's missing.

**An interrupted download resumes.** Each file is written to `<name>.part` and
renamed into place only once it is complete, so a half-finished file is never
mistaken for a model. Press Ctrl-C, lose your connection, or run out of disk,
and the next `banshee setup` continues from where it stopped rather than
starting the download again.

If a daemon is already running, `banshee setup` asks it to do the downloading
and prints what it reports. One process writes at a time, which is what lets the
partial file be picked up again. Any client can start the same download over
`banshee.download_models` and follow it by subscribing to `downloads`.

**Optional: better pronunciation.** Install `espeak-ng` and Banshee pronounces
unfamiliar words (tech jargon, proper nouns) instead of spelling them out. On
macOS it's `brew install espeak-ng`; `banshee status` prints the command for
your system.

**2. Grant macOS permissions.** Banshee needs three of them, otherwise it
quietly fails to record or type:

- **Microphone**, so it can capture audio.
- **Input Monitoring**, so it can listen for the global hotkey.
- **Accessibility**, so it can type out your transcribed text.

You'll find all three under **System Settings > Privacy & Security**. macOS
prompts for each one the first time Banshee needs it; approve the prompts, then
restart the daemon (permissions don't apply to an already-running process).

**3. Check your setup:**

```bash
banshee status
```

It reports on models, config, microphone, permissions, and daemon health, and
prints a fix for anything that's off. It never changes anything itself.

## Usage

Start the daemon:

```bash
banshee start
```

It runs at every login, restarts if it crashes, and logs to
`~/.banshee/daemon.log`. `banshee stop` pauses it until the next login or the
next `banshee start`. To run the daemon in the foreground instead (say, to
watch the logs while debugging), use `banshee serve`.

With the daemon running, the global hotkeys are:

- **Hold `F5`** to record. On release, the transcription is typed straight into
  the app you're focused on (this is dictation mode).
- **Hold `Shift + F5`** to record. On release, the transcription is saved, and
  you can grab it later with `banshee listen`.

To tap once to start and once to stop instead of holding, set
`hotkey_mode = "toggle"` under `[audio]`. Long dictations are easier that way,
and a session you forget about is still released by the push-to-talk watchdog.

#### On a Mac, `F5` means `Fn + F5`

macOS ships the top row as media keys, so plain `F5` is the microphone key and
starts Apple's own Dictation. The daemon never sees it: rdev watches key and
modifier events, and media keys are not among them. `Fn` is what turns it into
the keycode banshee can read.

Turning on _Settings → Keyboard → "Use F1, F2, etc. keys as standard function
keys"_ makes `F5` a single press, on the key that already has a microphone
printed on it. The trade is that brightness, volume, and the rest then need
`Fn`, so it is only worth it if you use those less than you dictate.

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

Both release binds are there on purpose. Hyprland matches modifiers exactly, so
letting go of `Shift` before `F5` means only the unmodified release matches.

Typing the transcription into the focused app needs **`wtype`** (wlroots
compositors such as Hyprland and Sway) or **`ydotool`** (anywhere, but it needs
its own daemon and uinput access). Without one of them, dictation has no way to
type and reports an error; the transcription is still kept in
`banshee history`. `banshee status` tells you which one it found.

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
| `banshee voices`                | List the speech voices on disk, and mark the one in use    |
| `banshee config set <key> <value>` | Change one setting in `config.toml`                     |
| `banshee serve`                 | Run the daemon in the foreground                           |
| `banshee service uninstall`     | Remove the start-at-login launch agent                     |
| `banshee listen`                | Print recent transcriptions                                |
| `banshee record start` / `stop` | Push-to-talk without the hotkey (for keybinds and scripts) |
| `banshee speak "<text>"`        | Speak some text aloud                                      |
| `banshee history`               | List all saved transcriptions                              |
| `banshee clear-history`         | Clear the saved transcriptions                             |

## Connect your coding agent (MCP)

`banshee-mcp-shim` is an MCP stdio server that bridges your coding agent to the
running daemon. This is what turns Banshee into a voice for the agent. It
exposes three tools:

| Tool                | What the agent does with it                                       |
| ------------------- | ----------------------------------------------------------------- |
| `speak_status`      | Say something aloud, for decisions made and work finished         |
| `ask_user`          | Ask a question aloud, then wait for and return your spoken answer |
| `listen_for_prompt` | Pick up anything you've said since it last checked                |

It works with any MCP-capable tool, like Claude Code, OpenCode, Cursor, and
others. Most of them use the same `mcpServers` config shape:

```json
{
  "mcpServers": {
    "banshee": {
      "command": "banshee-mcp-shim"
    }
  }
}
```

Each tool keeps this config in its own spot (for example, Cursor uses
`~/.cursor/mcp.json`, and Claude Code lets you add it with `claude mcp add`), so
check your tool's docs for where the MCP config lives. If your tool doesn't pick
up the binary from your `PATH`, use its full path instead. Restart the tool, and
the three tools will show up.

### Pi coding agent

Pi has its own extension API instead of MCP, so it gets a native extension that
talks to the daemon directly. Copy it in and restart Pi:

```bash
cp integrations/pi/banshee.ts ~/.pi/agent/extensions/
```

That's the whole setup; see [integrations/pi](integrations/pi) for details.
This is what the demo at the top is running.

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
hotkey_mode = "hold"   # hold = record while F5 is down | toggle = tap to start, tap to stop
barge_in = "stop"      # stop = the record hotkey cuts off whatever Banshee is saying | none

[audio.cues]
enabled = true         # tones on record start/stop, success, and errors
```

`input_device` is a case-insensitive substring of the microphone name, so
`"yeti"` matches `Blue Yeti Stereo Microphone`. Leave it as `"default"` to
follow whatever the OS is set to. If the name matches nothing, the daemon
refuses to start and lists the devices it did find, rather than quietly
recording from the wrong microphone.

`banshee devices` shows the names to choose from:

```
$ banshee devices
  Blue Yeti               system default, in use
  BlackHole 2ch
  MacBook Pro Microphone
```

It reads the microphones fresh each time, so unplugging one changes the list.
The command works whether or not the daemon is running; `in use` only appears
when a daemon has actually opened that device.

`vocabulary` biases Whisper toward project jargon and proper nouns it would
otherwise misspell; it is read once at startup, so restart the daemon after
changing it.

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

It lists only what is downloaded, so every name it prints can be spoken with
today. Nothing is marked in use when Kokoro did not load: the system fallback
speaks in whatever voice macOS is set to, which Banshee did not choose and does
not report. The voice is read once at startup, so a change needs a restart.

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

The first line is the state at the moment you connect, so a script never has to
guess. The daemon pushes the rest as they happen, rather than the command asking
again on a timer. A state that did not move is not printed twice.

The command runs until the daemon stops, and then exits non-zero, so a
supervisor can restart it. For a single answer rather than a stream, ask
`banshee status`: a reader that stops early only ends `banshee watch` at the
next change, which on a quiet daemon may be a long wait.

The same channel is open to any client over `banshee.subscribe`, which answers
with everything `banshee.status` reports and then sends
`banshee.state_changed` notifications on that connection.

### Changing a setting without an editor

`banshee config set` writes one key and keeps your comments and layout:

```bash
banshee config set stt.language de
banshee config set stt.vad_threshold 0.7
banshee config set stt.vocabulary '["tokio", "clippy"]'
banshee config set audio.cues.enabled false
```

The key is the section and the field, as they appear in the file. A number, a
`true`, or a `[list]` is read as that type; anything else is read as text. Quote
twice to force text, as in `banshee config set audio.input_device '"12"'`.

A value that the field does not accept is refused, and the message lists the
ones it does:

```
$ banshee config set audio.hotkey_mode sideways
TOML parse error at line 5, column 15
  |
5 | hotkey_mode = "sideways"
  |               ^^^^^^^^^^
unknown variant `sideways`, expected `hold` or `toggle`
```

`vad_threshold` takes effect immediately. Everything else is read once at
startup, so the command tells you to restart. This works whether or not the
daemon is running.

`endpoint_silence_ms` is how long you can go quiet mid-answer before Banshee
decides you're done. The default is deliberately generous so that thinking out
loud doesn't truncate your answer. Lower it if replies feel sluggish, raise it
if you keep getting cut off.

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
  Usually the machine is just slow, not broken. On an older CPU the default
  `balanced` model can take minutes on a few seconds of speech, which is
  indistinguishable from a mic that never captured anything. Run the daemon in
  the foreground with `banshee serve` and watch for the `Transcribed Ns of
audio in Ms` line; if it warns that transcription ran slower than realtime,
  set `preset = "fast"` in `config.toml` and run `banshee setup`. On a 2014
  dual-core laptop that took one clip from 104s to 4.8s.
- **`banshee record start` says the microphone is busy.** A previous
  push-to-talk never got its matching `stop` (a dropped key release, or a
  script that died mid-recording). `banshee record stop` clears it; the daemon
  also releases the mic on its own after two minutes.
- **Audio sounds muffled on Bluetooth earbuds while Banshee runs.** Banshee
  keeps the microphone open so the hotkey can start recording instantly, and
  macOS switches Bluetooth earbuds to their low-quality telephony profile
  whenever any app holds their mic. The fix: in **System Settings > Sound**, set
  _Input_ to your Mac's built-in microphone and leave _Output_ on the earbuds.
  Playback quality comes back, and the built-in mic transcribes better anyway.
  For a one-off (say, a movie), `banshee stop` releases the mic entirely.
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
