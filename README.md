# Banshee

Banshee is an offline, always-on voice daemon for macOS and Linux. It does local
speech-to-text dictation and text-to-speech, and exposes everything over a
JSON-RPC API plus an MCP server so your favorite LLM host can talk and listen
through it. It's pure Rust, and nothing ever leaves your machine.

You hold a hotkey, say what you're thinking, and Banshee transcribes it locally
with Whisper. It either types the text straight into whatever app you're in, or
hands it off to an LLM. No cloud, no API keys, no audio leaving your laptop.

## Install

Runs on **macOS (Apple Silicon)** and **Linux** (x86_64 / aarch64); Intel Macs
aren't supported. Needs ~1 GB of disk for the models.

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
cargo install --path bansheed          # the `banshee` command
cargo install --path banshee-mcp-shim  # the MCP shim (optional)
```

## Setup

**1. Download the models** into `~/.banshee/models/`:

```bash
banshee setup
```

| File | Size | What it is |
| --- | --- | --- |
| `ggml-large-v3-turbo-q5_0.bin` | ~547 MB | Whisper STT model (default `balanced` preset) |
| `silero_vad.onnx` | ~2 MB | Silero voice-activity detection |
| `kokoro-v1.0.onnx` | ~310 MB | Kokoro TTS model |
| `af_heart.bin` | ~512 KB | Kokoro voice style (the configured `voice`) |

Files that already exist are skipped, so re-running `banshee setup` after
changing the STT preset or the TTS voice only downloads what's missing.

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
banshee doctor
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

- **Hold `F5`** to record. On release, the transcription is saved, and you can
  grab it later with `banshee listen`.
- **Hold `Shift + F5`** to record. On release, the transcription is typed
  straight into the app you're focused on (this is dictation mode).

The CLI commands all talk to the running daemon over its socket:

| Command | What it does |
| --- | --- |
| `banshee start` | Start the daemon, now and at every login |
| `banshee stop` | Stop the running daemon |
| `banshee setup` | Download the required models |
| `banshee status` | Show daemon health and state |
| `banshee doctor` | Diagnose setup problems and report fixes |
| `banshee serve` | Run the daemon in the foreground |
| `banshee service uninstall` | Remove the start-at-login launch agent |
| `banshee listen` | Print recent transcriptions |
| `banshee record start` / `stop` | Push-to-talk without the hotkey (for keybinds and scripts) |
| `banshee speak "<text>"` | Speak some text aloud |
| `banshee history` | List all saved transcriptions |
| `banshee clear-history` | Clear the saved transcriptions |

---

## Configuration

Configuration is optional. If you want to override the defaults, create
`~/.banshee/config.toml` (defaults shown here):

```toml
[daemon]
save_history = true    # keep transcriptions in ~/.banshee/banshee.db

[stt]
preset = "balanced"    # fast | balanced | quality (see below)
vad_threshold = 0.5    # 0.0 to 1.0; higher means stricter speech detection
vocabulary = []        # words Whisper keeps mangling, e.g. ["banshee", "clippy", "tokio"]

[tts]
voice = "af_heart"     # any voice from the Kokoro voices directory
speed = 1.0            # playback speed multiplier
fallback = "system"    # system = use `say` when Kokoro is unavailable | none

[audio.cues]
enabled = true         # tones on record start/stop, success, and errors
```

You can also change `vad_threshold` at runtime through the `banshee.configure`
RPC, no restart needed. `vocabulary` biases Whisper toward project jargon and
proper nouns it would otherwise misspell; it is read once at startup, so
restart the daemon after changing it.

The `preset` picks which Whisper model Banshee uses:

| Preset | Model | Trade-off |
| --- | --- | --- |
| `fast` | `ggml-base.en.bin` | Fastest and lightest, English only |
| `balanced` | `ggml-large-v3-turbo-q5_0.bin` | The default; accurate and reasonably quick |
| `quality` | `ggml-large-v3-q5_0.bin` | Most accurate, heaviest |

For `voice`, any file in the
[Kokoro voices directory](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/tree/main/voices)
works, e.g. `af_bella`, `am_michael`, or `bf_emma` (the prefix is
accent/gender: `a`merican/`b`ritish, `f`emale/`m`ale). After changing the
`preset` or `voice`, run `banshee setup` to download the new files, then restart
the daemon.

## MCP integration

`banshee-mcp-shim` is an MCP stdio server that bridges LLM hosts to the daemon
and exposes speak and listen tools. It needs the daemon to be running.

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
the speak and listen tools will show up.

## Troubleshooting

Start with `banshee doctor`; it catches most setup problems and tells you the
fix. Beyond that:

- **Audio sounds muffled on Bluetooth earbuds while Banshee runs.** Banshee
  keeps the microphone open so the hotkey can start recording instantly, and
  macOS switches Bluetooth earbuds to their low-quality telephony profile
  whenever any app holds their mic. The fix: in **System Settings > Sound**, set
  *Input* to your Mac's built-in microphone and leave *Output* on the earbuds.
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
