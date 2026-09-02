# Roadmap

What Banshee does next, in the order it is likely to land. Dates are not promises.
Issues labelled `help wanted` are sized for a first contribution; each one says how to
verify it on a real machine. `BACKLOG.md` holds what is missing or wrong today, in no order.

## Landed

- The desktop window. `Open Banshee` in the menu bar opens it: the last dictation with a
  copy button, the day's history with search, and the microphone, hotkey, voice and agent
  settings, each printing the CLI command it stands for. It ships inside the same
  `Banshee.app` as the daemon, the tray and the CLI.
- `banshee connect <agent>` wires Antigravity, Claude Code, Codex, Cursor, OpenCode and Pi
  after showing the change. Antigravity, Claude Code, OpenCode and Pi are verified on a real
  install; Cursor and Codex wait for a report (#53, #54).
- A microphone that disappears no longer stops dictation (#47).

## Next, maintainer

1. Notarisation with an Apple Developer ID, so the `Banshee.app` bundle installs without
   the Gatekeeper dance. Now the only thing gating every distribution channel, and the
   window has made it the first thing a new user meets.
2. Bring your own keys: a remote provider for TTS, then for STT, behind `tts.provider` and
   `stt.provider`. Local stays the default; the tray and `banshee status` say when text or
   audio leaves the machine; keys live in the environment or the Keychain, never in
   `config.toml`. Lands with the honest edit to the README's offline promise.
3. The window on Linux, as an AppImage and an AUR package, built by the bundle workflow with
   GTK and WebKit installed. The blockers band grows rows for the typers and the daemon's
   `PATH`, and the launcher stands in for the tray. After the keys, because no Linux user has
   asked yet and `banshee watch --waybar` already covers the live loop there. It also gives the
   WebDriver acceptance layer its first host: `tauri-driver` runs on Linux, not on macOS.
4. Output sinks that survive a device change (the cue and Kokoro sinks still open once).
   Measure whether spoken status dies with the earcons before designing.

## Community sized

- More agents for `banshee connect`: Windsurf, Zed, GitHub Copilot CLI, Kiro, Cline and
  Roo Code, Goose, Continue. Most take the same `mcpServers` JSON shape, so each is one
  `Agent` variant, one `plan` arm and two tests. One issue per agent (#55 to #58).
- A JSONC-preserving editor for the JSON hosts, so comments and trailing commas survive
  the way they already do for Codex's TOML.

## Not planned

- Spoken-status hooks for agents other than Claude Code. Pi and OpenCode call
  `speak_status` when the tool description tells them to; Claude Code did not always, so it
  got a Stop hook. No other agent exposes a hook that can block a turn, and none has shown
  the same need.
- `banshee narrate`. `banshee speak` is the primitive, and a shell line covers the use:
  `cargo build; banshee speak "build $([ $? = 0 ] && echo passed || echo failed)"`. Parsing
  arbitrary build output to guess the result is brittle, and an agent already speaks results
  with context.
- A process restart on config change. Subsystems reload; the daemon stays up.
- Cloud by default. Nothing leaves the machine unless you turn a provider on, and the
  tray says so while it is on.
