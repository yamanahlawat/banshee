<p align="center">
  <img src="assets/banshee-icon.png" alt="banshee" width="104">
</p>

# Banshee

- Gives your AI coding agent a voice.
- Speaks its decisions and questions out loud.
- You answer by talking, hands-free.
- Local Whisper and a local neural voice by default. No account.
- Words leave your laptop only with a remote listener or voice you set. The tray and `banshee status` say so.
- Also a dictation tool: hold a hotkey, speak. The text lands in the app you are focused on.
- Tell it to change your desktop: `banshee tell "make the window gaps bigger"` hands your words to your coding agent.

## Demo

<https://github.com/user-attachments/assets/912c94af-baac-4385-b135-07a4eeb11b0e>

Claude Code finds a first-run bug in Banshee's own code, says out loud what it would change, and asks how far to take the fix. The answer is spoken back. Nothing was typed.

## Quickstart on Linux

Hyprland, and Omarchy on it. Four commands:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/yamanahlawat/banshee/releases/latest/download/banshee-installer.sh | sh
banshee start
banshee bind hyprland
banshee connect claude
```

- The installer says how to put `banshee` on your PATH. Open a new shell if the next command is not found.
- `banshee start` downloads the models (~860 MB) the first time.
- `banshee bind hyprland` shows the block before it writes. Omarchy ships `wtype`, so nothing more is installed.
- `banshee connect claude` shows the change before it writes.
- The Claude Code hook needs `jq`. Restart Claude Code.
- **Hold `F9`** and speak. The text is typed into the app you are focused on.
- **Hold `Shift` and `F9`** to keep the text instead. `banshee listen` prints it.
- **Hold `Super` and `F9`** and say what to change. Your coding agent edits the config and speaks the result. [What it does](docs/cli.md#telling-your-agent-to-change-the-desktop)
- Omarchy's own dictation tool also uses `F9`. Unbind one if both are there.
- `banshee status` names the fix, and changes nothing itself.
- [Another compositor, or a status bar](docs/linux.md)

## Quickstart on macOS

Apple Silicon, with the window.

```bash
brew install --cask yamanahlawat/banshee/banshee
xattr -dr com.apple.quarantine /Applications/Banshee.app
open /Applications/Banshee.app
```

- The `xattr` line is needed until Banshee is notarised. [A direct download](docs/install.md#macos-without-homebrew) needs none.
- Banshee downloads the models (~860 MB) and starts.
- Approve the **Microphone** and **Accessibility** grants. Without them Banshee cannot record or type.
- **Hold `Right Option`**, speak, let go. The text is typed into the app you are focused on.
- `banshee status` names the fix, and changes nothing itself.

Then give your coding agent a voice:

```bash
banshee connect claude
```

- Restart the agent. It speaks its decisions and asks you questions out loud.
- [Every other agent](#connect-your-coding-agent)

## Use it

- **Hold the hotkey** and speak: `F9` on Linux, `Right Option` on macOS. On release the text is typed.
- **Add `Shift`** to keep the text instead of typing it. `banshee listen` prints it.
- **Talk over it.** The hotkey stops whatever Banshee says. `barge_in = "none"` lets it finish.
- **The window.** `Open Banshee` in the menu bar: the last dictation with a copy button, the day's history, every setting. Quit it and dictation carries on.

<p align="center"><img src="assets/window.png" width="360" alt="The Banshee window: the last dictation in large type with a copy button, the day's earlier dictations beneath it, and a footer naming the microphone, hotkey, voice and connected agents."></p>

- [Toggle instead of hold, and rebind the key](docs/configuration.md#the-hotkey)
- The menu bar icon answers one question: can I speak right now.

| Idle | Recording | Busy | Speaking | Waiting for you | Not running |
|:----:|:---------:|:----:|:--------:|:---------------:|:-----------:|
| <img src="assets/states/idle.png" width="52" alt=""> | <img src="assets/states/recording.png" width="52" alt=""> | <img src="assets/states/busy.png" width="52" alt=""> | <img src="assets/states/speaking.png" width="52" alt=""> | <img src="assets/states/listening.png" width="52" alt=""> | <img src="assets/states/notrunning.png" width="52" alt=""> |

- Busy means Banshee transcribes what you said, or the agent a `banshee tell` started still runs.
- Waiting for you means an agent asked a question and holds for your answer.
- The states differ by shape, never by colour alone, and macOS tints the template image.

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

- Each command shows the exact change, and asks before it writes. Restart the tool after.
- The Claude Code hook needs `jq` on your PATH.
- Antigravity, Claude Code, OpenCode and Pi are verified on a real install. Codex and Cursor follow their published formats, and wait for a report.
- Pi has its own extension API, so it gets a [native extension](integrations/pi) instead.
- The window's Agents panel does the same work.

<p align="center"><img src="assets/agents.png" width="360" alt="The Agents panel, listing Antigravity, Claude Code, OpenCode and Pi as connected, and noting that Banshee also works with Codex and Cursor."></p>

- `banshee-mcp-shim` is the MCP stdio server behind this.
- Any other MCP host takes the same shape. Use the shim's full path if the bare name does not resolve:

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

## Other ways to install

|                       | With the desktop window                                                                                           | Terminal only                                                           |
| --------------------- | ----------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| Linux (x86_64, arm64) | [from source](docs/linux.md#building-the-desktop-window)                                                          | the installer above, or [Homebrew](docs/install.md#linux-with-homebrew) |
| macOS (Apple Silicon) | [the cask](docs/install.md#macos-with-the-window), or [a direct download](docs/install.md#macos-without-homebrew) | [the formula](docs/install.md#macos-terminal-only)                      |
| Windows               | not yet                                                                                                           | not yet                                                                 |

- [Build from source](docs/install.md#from-source), [set up from the terminal](docs/install.md#set-up-from-the-terminal), or [remove any install](docs/install.md#uninstall)

## More

- [docs/why.md](docs/why.md) - what Banshee does that a dictation tool does not
- [docs/configuration.md](docs/configuration.md) - every setting, the voices, the hotkey
- [docs/cli.md](docs/cli.md) - every command
- [docs/linux.md](docs/linux.md) - other compositors, typing, the tray, a Waybar module
- [docs/troubleshooting.md](docs/troubleshooting.md) - what breaks, and the fix
- [CONTRIBUTING.md](CONTRIBUTING.md) - a build from source, and the window

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).
