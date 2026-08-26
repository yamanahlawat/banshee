# Roadmap

What Banshee does next, in the order it is likely to land. Dates are not promises.
Issues labelled `help wanted` are sized for a first contribution; each one says how to
verify it on a real machine.

## Landed

- `banshee connect <agent>` wires Antigravity, Claude Code, Codex, Cursor, OpenCode and Pi
  after showing the change. Antigravity, Claude Code, OpenCode and Pi are verified on a real
  install; Cursor and Codex wait for a report (#53, #54).
- A microphone that disappears no longer stops dictation (#47).

## Next, maintainer

1. Notarisation with an Apple Developer ID, so the `Banshee.app` bundle installs without
   the Gatekeeper dance. Gates every distribution channel and the desktop UI.
2. The desktop UI (Tauri). Device picker, connect-an-agent, status, all over the daemon's
   existing RPC. The last three pieces of work (#47, live config reload,
   `banshee connect`) were its prerequisites; it does not wait for anything else.
3. Bring your own keys: a remote provider for TTS, then for STT, behind `tts.provider` and
   `stt.provider`. Local stays the default; the tray and `banshee status` say when text or
   audio leaves the machine; keys live in the environment or the Keychain, never in
   `config.toml`. Lands with the honest edit to the README's offline promise.
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
