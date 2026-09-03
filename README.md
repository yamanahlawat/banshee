<p align="center">
  <img src="assets/banshee-icon.png" alt="banshee" width="104">
</p>

# Banshee

Banshee gives your AI coding agent a voice. It speaks its decisions and
questions out loud, and you answer by talking, hands-free, while it works.
Everything runs on your machine: local Whisper for listening, a local neural
voice for speaking. No API keys, no audio leaving your laptop.

It is a dictation tool too: hold a hotkey, speak, and the text lands in
whatever app you are focused on.

## Demo

<https://github.com/user-attachments/assets/006132bd-9710-4322-a35a-4a5e5004371c>

The daemon running with the Pi coding agent. It asks which language to use,
hears "let's go with python", and writes the file. Nothing was typed.

## Install

|                       | With the desktop window            | Terminal only                          |
| --------------------- | ---------------------------------- | -------------------------------------- |
| macOS (Apple Silicon) | [the cask](#macos-with-the-window) | [the formula](#macos-terminal-only)    |
| Linux (x86_64, arm64) | not yet                            | [the formula or the installer](#linux) |
| Windows               | not yet                            | not yet                                |

Pick one. Needs ~1 GB of disk for the models. Intel Macs are not supported.

### macOS, with the window

```bash
brew install --cask yamanahlawat/banshee/banshee
xattr -dr com.apple.quarantine /Applications/Banshee.app
```

The second line is needed until Banshee is notarised. Homebrew marks the
download as quarantined; a quarantined Banshee will not open, and its `banshee`
command dies with no message.

Then open Banshee from `/Applications`. It fetches the models, asks for the
two grants and starts the daemon. The `banshee` command is on your `PATH` too.

Installed the app with `curl` before? Delete `/Applications/Banshee.app` first.
Installed the formula before? Run these first, in this order:

```bash
banshee tray --uninstall
banshee service uninstall
brew uninstall --formula banshee
```

Without Homebrew: download and unpack the app, which sets no quarantine flag.

```bash
curl -fsSL https://github.com/yamanahlawat/banshee/releases/latest/download/Banshee.app.tar.gz \
  | tar -xzf - -C /Applications
```

The `banshee` command is then `/Applications/Banshee.app/Contents/MacOS/banshee`;
link it onto your `PATH` if you want it short. No admin account? Unpack into
`~/Applications` and read that path instead.

### macOS, terminal only

```bash
brew install --formula yamanahlawat/banshee/banshee
```

The daemon, the `banshee` command and the menu bar icon. To add the window
later, follow the switch above, then install the cask.

### Linux

```bash
brew install --formula yamanahlawat/banshee/banshee
```

Or without Homebrew:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/yamanahlawat/banshee/releases/latest/download/banshee-installer.sh | sh
```

The daemon and the `banshee` command. `banshee watch --waybar` feeds a status
bar. See [docs/linux.md](docs/linux.md) for the typing tool and the service.

### From source

Clone the repo and run `make install`; see [CONTRIBUTING.md](CONTRIBUTING.md).

## Uninstall

- The cask: `brew uninstall --zap --cask banshee`. `--zap` also removes
  `~/.banshee`, which holds the models, the history and `config.toml`.
- The formula or the installer: `banshee tray --uninstall`, then
  `banshee service uninstall`, then `brew uninstall --formula banshee` or delete
  the binaries. Delete `~/.banshee` if you want the models and history gone too.
- The app from `curl`: the two `banshee` commands above, then delete
  `/Applications/Banshee.app`, and `~/.banshee` if you want.

## Set up

With the window installed, from the cask or from `curl`, open Banshee. It
fetches the models, asks for the grants and starts the daemon. The steps below
are the same work from a terminal.

**1. Download the models** (~860 MB: Whisper, Silero VAD, Kokoro):

```bash
banshee setup
```

An interrupted download resumes, and a re-run fetches only what is missing.

**2. Grant the macOS permissions.** Banshee needs two, or it quietly fails to
record or type: **Microphone** to capture, and **Accessibility** for the global
hotkey and for typing. macOS asks for each the first time Banshee needs it.
Approve, and the daemon restarts itself to pick the grant up.

**3. Start it, then check it:**

```bash
banshee start
banshee status
```

`banshee start` runs the daemon now and at every login. `banshee status`
reports the models, the microphone, the permissions and the daemon, and prints
a fix for anything that is off. It changes nothing itself.

## Use it

- **Hold the hotkey** (Right Option by default) and speak. On release the text
  is typed into the app you are focused on.
- **Hold `Shift` and the hotkey** to keep the text instead: `banshee listen`
  prints it.
- **The window.** Choose `Open Banshee` from the menu bar for the last
  dictation with a copy button, the day's history, and every setting. It sets
  Banshee up on its own, models and all. Quit it and dictation carries on.

<p align="center">
  <img src="assets/window.png" width="360"
       alt="The Banshee window: the last dictation in large type with a copy button, the day's earlier dictations beneath it, and a footer naming the microphone, hotkey, voice and connected agents.">
</p>

To tap once to start and once to stop instead of holding, set
`hotkey_mode = "toggle"`. The key is rebindable, and both live in
[docs/configuration.md](docs/configuration.md).

The menu bar icon answers one question: can I speak right now.

| Idle | Recording | Speaking | Waiting for you | Not running |
|:----:|:---------:|:--------:|:---------------:|:-----------:|
| <img src="assets/states/idle.png" width="52" alt=""> | <img src="assets/states/recording.png" width="52" alt=""> | <img src="assets/states/speaking.png" width="52" alt=""> | <img src="assets/states/listening.png" width="52" alt=""> | <img src="assets/states/notrunning.png" width="52" alt=""> |

Waiting for you means an agent has asked a question and is holding for your
answer.

The states differ by shape, never by colour alone, and the icon is a template
image, so macOS tints it to match the menu bar in light and dark.

## Connect your coding agent

```bash
banshee connect            # which agents are installed, and which are connected
banshee connect antigravity # Antigravity IDE, agy CLI and SDK: the MCP server in ~/.gemini/config/mcp_config.json
banshee connect claude      # Claude Code: the MCP server and a stop hook that refuses to end a turn with no spoken status
banshee connect codex       # Codex CLI: the MCP server in ~/.codex/config.toml
banshee connect cursor      # Cursor: the MCP server in ~/.cursor/mcp.json
banshee connect opencode    # OpenCode: the MCP server
banshee connect pi          # Pi: the native extension
```

Each command shows the exact change to that tool's config and asks before it
writes, then you restart the tool. The Claude Code hook needs `jq` on your
PATH. Antigravity, Claude Code, OpenCode and Pi are verified on a real install;
Codex and Cursor follow their published formats and wait for a report.

Pi has its own extension API instead of MCP, so `banshee connect pi` installs a
native extension that talks to the daemon directly; see
[integrations/pi](integrations/pi).

The window's Agents panel does the same work: it lists what is installed, shows
the change before it writes, and says which agents are connected.

<p align="center">
  <img src="assets/agents.png" width="360"
       alt="The Agents panel, listing Antigravity, Claude Code, OpenCode and Pi as connected, and noting that Banshee also works with Codex and Cursor.">
</p>

`banshee-mcp-shim` is the MCP stdio server behind this. Any other MCP host takes
the same shape, with the shim's full path if the bare name does not resolve:

```json
{
  "mcpServers": {
    "banshee": {
      "command": "banshee-mcp-shim"
    }
  }
}
```

It exposes three tools:

| Tool                | What the agent does with it                                       |
| ------------------- | ----------------------------------------------------------------- |
| `speak_status`      | Say something aloud, for decisions made and work finished         |
| `ask_user`          | Ask a question aloud, then wait for and return your spoken answer |
| `listen_for_prompt` | Pick up anything you've said since it last checked                |

## More

- [docs/why.md](docs/why.md) - what Banshee does that a dictation tool does not
- [docs/configuration.md](docs/configuration.md) - every setting, the speech
  presets, the voices, and the hotkey
- [docs/cli.md](docs/cli.md) - every command
- [docs/linux.md](docs/linux.md) - Wayland hotkeys, typing, and a Waybar module
- [docs/troubleshooting.md](docs/troubleshooting.md) - what breaks, and the fix
- [CONTRIBUTING.md](CONTRIBUTING.md) - building from source, and the window

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).
