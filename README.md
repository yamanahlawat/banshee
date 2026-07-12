# Banshee

Banshee is an offline, always-on voice daemon for macOS. It does local
speech-to-text dictation and text-to-speech, and exposes everything over a
JSON-RPC API plus an MCP server so your favorite LLM host can talk and listen
through it. It's pure Rust, and nothing ever leaves your machine.

You hold a hotkey, say what you're thinking, and Banshee transcribes it locally
with Whisper. It either types the text straight into whatever app you're in, or
hands it off to an LLM. No cloud, no API keys, no audio leaving your laptop.

> **Heads up: Banshee is a work in progress.** Today it runs on **macOS only**.
> Windows and Linux support is on the roadmap and coming in a future release, so
> stay tuned!

## Features

- **Local dictation.** Hold a hotkey, speak, release, and the text lands in
  whatever app is focused.
- **Offline STT.** Whisper (`whisper-rs`) gated by Silero voice-activity
  detection. No network, no API keys.
- **Offline TTS.** Banshee speaks with the Kokoro neural voice, synthesized
  locally via ONNX. If the model isn't downloaded, it falls back to the macOS
  `say` command.
- **Audio cues.** Short tones confirm recording start/stop, a successful
  transcription, and errors, so you know what happened without looking at the
  screen.
- **JSON-RPC API** over a Unix socket, so it's easy to script from the CLI or
  any client you like.
- **MCP server** that exposes speak and listen tools to LLM hosts (like Claude
  Desktop), so an assistant can speak and listen through Banshee.

## Requirements

- **macOS** (Apple Silicon or Intel). Audio capture and hotkeys are macOS-only
  for now.
- **Rust** (stable, edition 2024). Grab it from [rustup](https://rustup.rs).
- **Around 1 GB** of free disk for the default models (see the table below).

## Install

Install Banshee onto your `PATH` so you can use the `banshee` command from
anywhere:

```bash
git clone https://github.com/yamanahlawat/banshee.git
cd banshee
cargo install --path bansheed --features apple   # installs the `banshee` command
cargo install --path banshee-mcp-shim            # installs the MCP shim (optional)
```

The `apple` feature turns on Metal and CoreML acceleration for Whisper. Both
binaries land in `~/.cargo/bin`, so from here on you can just run `banshee`.

If `banshee` comes back as "command not found", `~/.cargo/bin` isn't on your
`PATH` yet (common if you installed Rust via Homebrew rather than rustup). Add
it and reload your shell:

```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

Prefer to build without installing? `cargo build --release --features apple`
drops the binaries in `target/release/` instead.

## Setup

**1. Download the models** into `~/.banshee/models/`:

```bash
banshee setup
```

This fetches everything Banshee needs to work offline:

| File | Size | What it is | Downloaded from |
| --- | --- | --- | --- |
| `ggml-large-v3-turbo-q5_0.bin` | ~547 MB | Whisper STT model (default `balanced` preset) | [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) on Hugging Face |
| `silero_vad.onnx` | ~2 MB | Silero voice-activity detection | [snakers4/silero-vad](https://github.com/snakers4/silero-vad) on GitHub |
| `kokoro-v1.0.onnx` | ~310 MB | Kokoro TTS model | [onnx-community/Kokoro-82M-v1.0-ONNX](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX) on Hugging Face (pinned revision) |
| `af_heart.bin` | ~512 KB | Kokoro voice style (the configured `voice`) | same Kokoro repo, `voices/` directory |

Files that already exist are skipped, so re-running `banshee setup` after
changing the STT preset or the TTS voice only downloads what's missing.

**2. Grant macOS permissions.** Banshee needs two of them, otherwise it quietly
fails to record or type:

- **Microphone**, so it can capture audio.
- **Accessibility**, so it can listen for the global hotkey and type out your
  transcribed text.

You'll find both under **System Settings > Privacy & Security**. Add your
terminal (or the `banshee` binary) to the *Microphone* and *Accessibility*
lists.

## Usage

Start the daemon:

```bash
banshee serve
```

With the daemon running, the global hotkeys are:

- **Hold `F5`** to record. On release, the transcription is saved, and you can
  grab it later with `banshee listen`.
- **Hold `Shift + F5`** to record. On release, the transcription is typed
  straight into the app you're focused on (this is dictation mode).

The CLI commands all talk to the running daemon over its socket:

| Command | What it does |
| --- | --- |
| `banshee serve` | Start the background daemon |
| `banshee setup` | Download the required models |
| `banshee status` | Show daemon health and state |
| `banshee listen` | Print recent transcriptions |
| `banshee speak "<text>"` | Speak some text aloud |
| `banshee history` | List all saved transcriptions |
| `banshee clear-history` | Clear the saved transcriptions |

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

For `voice`, any file in the
[Kokoro voices directory](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/tree/main/voices)
works, e.g. `af_bella`, `am_michael`, or `bf_emma` (the prefix is
accent/gender: `a`merican/`b`ritish, `f`emale/`m`ale). After changing it, run
`banshee setup` to download the new voice, then restart the daemon.

### Picking a Whisper model

The `preset` picks which Whisper model Banshee uses:

| Preset | Model | Trade-off |
| --- | --- | --- |
| `fast` | `ggml-base.en.bin` | Fastest and lightest, English only |
| `balanced` | `ggml-large-v3-turbo-q5_0.bin` | The default; accurate and reasonably quick |
| `quality` | `ggml-large-v3-q5_0.bin` | Most accurate, heaviest |

After changing the preset, run `banshee setup` again to download the matching
model into `~/.banshee/models/`.

## MCP integration

`banshee-mcp-shim` is an MCP stdio server that bridges LLM hosts to the daemon
and exposes speak and listen tools. It needs `banshee serve` to be running.

If you installed it with `cargo install` above, the shim lives at
`~/.cargo/bin/banshee-mcp-shim`. It works with any MCP-capable tool, like Claude
Code, OpenCode, Cursor, and others. Most of them use the same `mcpServers`
config shape:

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
up the binary from your `PATH`, use the full path
(`~/.cargo/bin/banshee-mcp-shim`) instead. Restart the tool, and the speak and
listen tools will show up.

## Architecture

| Crate | Role |
| --- | --- |
| `bansheed` | The daemon: audio capture, VAD, STT, hotkeys, JSON-RPC API |
| `banshee-mcp-shim` | MCP stdio to daemon bridge |
| `banshee-common` | Shared protocol types (JSON-RPC, errors, config) |

The daemon exposes a JSON-RPC 2.0 API over a Unix socket at
`~/.banshee/banshee.sock`. The CLI and the MCP shim are both just clients of it.

## License

Licensed under the [MIT License](LICENSE).
