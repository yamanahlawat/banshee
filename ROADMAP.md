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
- The desktop window and the menu bar icon run on Linux. `make install-window` builds both
  from a source clone, and the icon sits in any bar that hosts StatusNotifierItem. The
  blockers band names a missing Wayland typer. On Wayland the compositor holds the hotkey,
  and the window says so instead of naming a key nothing binds.

## Next, maintainer

1. Bring your own keys, in three steps, each its own release. First because it serves two
   people at once: the CPU-only machine that Whisper leaves waiting, and the agent user who
   wants a voice with feeling. Local stays the default throughout. A key is set with
   `banshee config set` or in the window and lives in an owner-only file beside
   `config.toml`, never in `config.toml` itself: that file is world-readable and rides inside
   every status reply.
   1. The provider seam. `stt.provider` and `tts.provider` with `local` as the only value, a
      transcriber trait beside the speech backend trait that exists, and the local engines
      moved under their own module so a remote one arrives as a sibling. The status reply
      gains a `remote` object and `banshee status` says that audio and text stay on this
      machine. Nothing leaves the machine, so the README stays as it is.
   2. Remote transcription. The recorded utterance goes out, text comes back where Whisper's
      text lands today. A failure fails that utterance aloud; Banshee never changes provider
      on its own. First because it is what blocks a CPU-only machine: Kokoro already runs on
      the CPU on every platform, and the `balanced` model can take minutes on an older CPU.
      The OpenAI-compatible transcription shape comes first, because one provider covers
      OpenAI, Groq and a self-hosted Whisper server. Lands with the key file, the tray and
      the window saying when audio leaves the machine, and the honest edit to the README's
      "No API keys, no audio leaving your laptop". Deepgram waits for that shape to prove
      itself on a real machine.
   3. Remote speech. The same OpenAI-compatible shape first, ElevenLabs second for its
      expressive controls. After transcription because it touches the output sink path,
      which the output-sinks item below records as fragile.
2. Linux packaging: an AppImage and an AUR package, built by the bundle workflow with GTK and
   WebKit installed. The window and the tray already run there, but only from a source clone,
   which asks a user for a Rust toolchain and Node. The daemon's `PATH` still wants a blockers
   row. It also gives the WebDriver acceptance layer its first host: `tauri-driver` runs on
   Linux, not on macOS.
3. Output sinks that survive a device change (the cue and Kokoro sinks still open once).
   Measure whether spoken status dies with the earcons before designing.
4. Notarisation with an Apple Developer ID, so the bundle opens with no dialog. Last, because
   the Homebrew cask ships with 0.12.1 and the README shows the one-time Gatekeeper step, so
   this removes a dialog rather than a gate.

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
