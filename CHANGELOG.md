# Changelog

All notable changes to Banshee are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `banshee devices` lists the microphones and marks which one the daemon opened,
  so `audio.input_device` can be set to a name you have seen rather than one you
  guessed. The daemon answers the same question over
  `banshee.list_input_devices`. Enumeration needs no daemon, which matters
  because you need the names before you can start one on the right microphone.
  The list does not say whether each device opens: probing them all would steal
  the microphone from the running daemon.

- `banshee readiness` lists what still blocks recording: a missing permission, a
  model that is not on disk, or a recording pipeline that died at startup. Each
  entry says what breaks and how to fix it. The daemon answers the same question
  over `banshee.readiness`, so a future GUI shows a checklist rather than
  inventing its own.

- `banshee config set <key> <value>` writes one setting to `config.toml`. The key
  is the section and the field, as in `stt.language`. Comments and layout
  survive, because the file is edited rather than rewritten. A value the field
  does not accept is refused, and the message lists the ones it does.
  `vad_threshold` takes effect at once; everything else needs a restart, and the
  command says so.

### Changed

- **Breaking.** `banshee.configure` now takes `{"settings": {"stt.language":
  "de"}, "persist": false}` instead of a flat `{"vad_threshold": 0.6}`. Keys are
  dotted, so every setting in `config.toml` is reachable through one call rather
  than one field at a time, and an unknown key returns `-32602` instead of
  succeeding silently. `persist` writes the value to the file as well as
  applying it, and is required for any setting the daemon reads only at startup,
  which is all of them except `vad_threshold`. The reply says which keys landed
  and which need a restart.

- A `vad_threshold` outside 0.0 to 1.0 is now refused wherever it arrives. The
  daemon used to start with a hand-edited `5.0` and never detect speech again.

## [0.7.0] - 2026-08-01

The no-silent-failures release: dictation no longer kills the daemon, the hotkey
no longer sends your words to the wrong place without saying so, and a config
key written under the wrong section now fails at startup instead of quietly
doing nothing.

Two changes affect existing setups. `F5` and `Shift + F5` have swapped meaning,
and a config file with a misplaced or stale key will now refuse to start until
you fix it; `banshee doctor` names the offending key.

### Added

- `hotkey_mode = "toggle"` under `[audio]` now works: tap the hotkey to start
  recording and tap it again to stop, instead of holding it down. The field has
  been parsed since the config landed but was never read. Holding a key through
  a long dictation couples your pace to your finger; the existing push-to-talk
  watchdog still releases a session you walk away from.

- `banshee doctor` now prints the settings actually in effect (hotkey mode,
  barge-in, cues, STT preset, VAD threshold, endpoint, vocabulary size, voice,
  speed, history) instead of only reporting that the file parsed.

### Changed

- Unknown config keys are now an error instead of being ignored. TOML binds a
  key to whatever table precedes it, so `hotkey_mode` written under `[tts]`
  parsed cleanly and left `audio.hotkey_mode` at its default with nothing
  reported anywhere. A stale key from an older version now fails startup and
  names itself rather than silently doing nothing.

- `F5` alone now dictates into the focused app, and `Shift + F5` captures to the
  mailbox for `banshee listen`. Dictation is the common case, so it no longer
  carries the modifier. The shift state was sampled at the instant `F5` went
  down, so pressing the two keys near-simultaneously sent the utterance to the
  mailbox instead, and both targets play the same ready cue, which made the
  misroute silent.

### Fixed

- Dictation no longer aborts the daemon. Pasting resolved `v` through
  `Key::Unicode`, which reaches Text Input Services on macOS; TIS is
  main-thread-only and intermittently called `abort()` from the transcription
  thread, killing the daemon after the transcription was saved but before the
  paste. macOS now uses the raw `kVK_ANSI_V` keycode, which needs no lookup.

## [0.6.1] - 2026-07-27

The one-install release: connecting a coding agent no longer needs a second
package.

### Changed

- `banshee-mcp-shim` now ships alongside `banshee` in the same release archive,
  Homebrew formula, and shell installer. It used to be published as its own
  formula, so `brew install banshee` left you without the MCP server and
  `claude mcp add banshee -- banshee-mcp-shim` failed with command not found.

## [0.6.0] - 2026-07-26

The Wayland release: dictation types into your focused window on Hyprland and
Sway, you can name the microphone you actually want, and a push-to-talk that
never got its release no longer holds the mic hostage.

### Added

- Dictation types under Wayland via `wtype` (wlroots compositors) or `ydotool`,
  where the X11 path could not. With neither installed it reports an error
  instead of silently dropping the text, which stays in `banshee history`.
- `[audio] input_device` picks the microphone by a case-insensitive substring of
  its name, so `"yeti"` matches `Blue Yeti Stereo Microphone`. A name that
  matches nothing refuses to start and lists the devices it found, rather than
  quietly recording from the wrong microphone.
- Push-to-talk watchdog: a `record start` with no matching `stop` releases the
  microphone after two minutes and transcribes what it captured, so a dropped
  key release or a script that died mid-recording no longer wedges the daemon
  into refusing every later start as busy.
- Transcription warns when it runs more than 2x slower than realtime and names
  the fix (`[stt] preset = "fast"`). On a slow CPU the default model can take
  minutes, which is indistinguishable from a microphone that never captured
  anything.
- README section on binding push-to-talk in your compositor, with the Hyprland
  `bind`/`bindr` snippet that gives you hold-to-talk without the global hotkey.
- Issue templates for bug reports and feature requests.
- `banshee start` says when macOS Accessibility is missing and opens the
  settings pane for it, instead of leaving you with a hotkey that does nothing.
- The daemon restarts itself the moment that grant lands, so you no longer have
  to know that a restart was needed for a permission to take effect.

### Changed

- Default voice is now `af_sky` at 1.2x speed. Upgrading users who never set
  `tts.voice` should re-run `banshee setup` to fetch the new voice file;
  without it Kokoro falls back to system TTS.
- `banshee doctor` reports what a Wayland session actually supports (which
  typing tool it found, and the compositor bind to use for the hotkey) instead
  of failing the session and telling you to log in to X11.
- The daemon says at startup that the global hotkey needs X11 rather than
  leaving a Wayland user with a hotkey that looks broken for no visible reason,
  and a listener that dies names the commands that still work.

## [0.5.0] - 2026-07-26

The vocabulary release: the voice pronounces your jargon instead of reciting it
letter by letter, and dictation stops eating the clipboard on Linux.

### Added

- espeak-ng fallback pronounces unknown words instead of spelling them out.
  Optional; `banshee doctor` reports it and prints the install command.
- Pronunciation dictionary for terms the voice got wrong, including `yaml`,
  `toml`, `kubernetes`, `webhook`, and `symlink`.
- Pi coding agent extension (`integrations/pi/banshee.ts`), talking to the
  daemon directly since Pi has its own extension API rather than MCP.

### Changed

- `[stt] endpoint_silence_ms` defaults to 2500 (was 1000), so pausing to think
  no longer cuts your answer short.
- `ask_user` scales its playback wait with the length of the question.
- `speak_status` and `ask_user` descriptions steer the agent to treat speech as
  its reply, and to ask one question per call.
- Dictated text is kept out of clipboard manager history on Linux.

### Fixed

- Dictation no longer destroys the clipboard on Linux, where clipboard contents
  live in a process rather than a system service.
- All-caps terms such as `YAML` are pronounced rather than spelled out.
- The clipboard restore no longer delays the ready cue.
- The Pi extension no longer hangs if the daemon closes the connection mid-call.

## [0.4.0] - 2026-07-21

The install release: prebuilt binaries you can install with one command,
start-at-login on macOS and Linux, and a doctor that finds setup problems
before they turn into bug reports.

### Added

- Prebuilt binaries and installers (via cargo-dist): every release now carries
  macOS (Apple Silicon and Intel) and Linux binaries with a shell installer and
  a Homebrew tap, so `brew install yamanahlawat/banshee/banshee` or a single
  `curl` command gets you running. macOS binaries build with Metal and CoreML
  acceleration automatically; installing from source still works with
  `cargo install`.
- `banshee doctor`: read-only diagnostics for config parsing, model presence,
  the microphone device, macOS Accessibility permission, and the daemon socket,
  exiting nonzero when something is wrong.
- Start-at-login service management: `banshee start` and `banshee stop` install
  a launchd agent (macOS) or a systemd user unit (Linux) so the daemon runs at
  login and restarts on crash.
- `banshee.record_start` and `banshee.record_stop` RPCs drive push-to-talk from
  a script or client without touching the physical hotkey.
- Pronunciation fixes for more developer terms, plus a passive log of words the
  voice spelled out letter by letter so the fixup list can grow from real use.

### Changed

- `banshee.history` returns transcriptions oldest first, so a terminal shows the
  newest entry at the bottom next to your prompt.
- Now dual-licensed under MIT OR Apache-2.0 (previously MIT only).

## [0.3.0] - 2026-07-17

The hands-free release: the daemon speaks with its own offline voice, and an
agent can ask a question aloud and hear the answer without a single keypress.

### Added

- Kokoro TTS: offline neural speech synthesis with a pure-Rust G2P (no
  espeak dependency), streamed sentence by sentence so long messages start
  playing immediately. Pick a voice with `[tts] voice`; when the model is not
  on disk, `[tts] fallback` selects the system voice or refuses to start.
  `banshee setup` downloads the model and voice data.
- `ask_user` MCP tool and `banshee.ask_user` RPC: one complete voice turn.
  The question is spoken aloud, the microphone arms once playback ends, and
  online voice-activity endpointing captures the answer, with trailing
  silence ending it (`[stt] endpoint_silence_ms`, default 1000). The
  transcript returns scoped to the calling agent; staying silent returns
  empty text after `timeout_ms`.
- Manual override while armed: hold `F5` to answer on your own terms; the
  transcript captured during the hold becomes the answer.
- Arm and disarm earcons mark exactly when the hands-free microphone goes
  hot and shuts. A concurrent `ask_user` is refused with
  `-32004 MICROPHONE_BUSY` instead of stealing the microphone.
- Identifier verbalization for spoken text: snake_case and camelCase names
  are split into words, and common developer terms get pronunciation fixes.

### Changed

- An `ask_user` question interrupts queued status speech instead of waiting
  behind it.

### Fixed

- A freshly started MCP shim no longer replays speech from before its
  session; its cursor is primed to the newest transcription at startup.
- An armed listening session always ends: hard ceilings on answer length and
  question playback mean continuous background noise or stalled speech can
  no longer hold the microphone open.

## [0.2.0] - 2026-07-11

The conversation loop release: transcriptions are never lost, agents can wait
for speech instead of polling, every action has an audible cue, and the daemon
is a well-behaved citizen.

### Added

- Transcription ring with cursor-based reads: the daemon keeps the last 16
  transcriptions with monotonic ids, and `banshee.get_transcription` accepts
  `since_id` (return only newer entries) and `wait_ms` (long-poll for new
  speech). Multiple clients can read the same utterances without stealing them
  from each other, and stale cursors from a previous daemon run self-heal.
- Audio cues: short tones confirm recording start/stop, successful delivery,
  and every failure path, so you know what happened without looking at the
  screen. Configurable via `[audio.cues] enabled`.
- Vocabulary biasing: `[stt] vocabulary` words are fed to Whisper as the
  initial prompt, improving recognition of project-specific jargon.
- Anti-hallucination gates: segments where Whisper both doubts speech was
  present and doubts its own words (high `no_speech_prob` and low average
  log-probability) are discarded, killing the infamous invented captions on
  near-silence. Per-segment confidence is logged for threshold calibration.
- `banshee.stop_speaking` RPC to halt playback, and `banshee.speak` now
  returns an `utterance_id` and accepts `interrupt` to jump the queue.
- Speech queue: concurrent speak requests play one at a time in order (capped
  at 8 pending) instead of overlapping into noise.
- Barge-in: pressing the hotkey while the daemon is speaking silences it
  (`[audio] barge_in = "stop"`, the default).
- Daemon hygiene: single-instance lock (a second `banshee serve` refuses to
  start), stale socket cleanup after crashes, graceful shutdown on Ctrl+C and
  SIGTERM, and an owner-only (0600) socket.
- `banshee status` now reports a real `speaking` flag.

### Changed

- Protocol: `banshee.get_transcription` returns
  `{"transcriptions": [{"id", "text"}]}` instead of a single destructive
  `transcription` string; reads no longer consume the entry.
- The transcription pipeline runs on a dedicated thread, so heavy Whisper
  inference no longer degrades RPC latency, and transcription time is logged
  alongside audio duration.
- MCP tool descriptions rewritten for eyes-free use: agents are told to speak
  only decisions, questions, and phase completions, conversationally, and to
  keep paths, code, and URLs in text output.
- The MCP shim tracks its own transcription cursor and supports `timeout_ms`
  on `listen_for_prompt` to wait for a spoken answer.

### Fixed

- A second utterance no longer silently overwrites an unread one (the old
  single-slot mailbox is gone).
- Multiple simultaneous `speak` calls no longer play over each other.
- A second daemon instance no longer silently steals the socket from the
  running one.
- The audio capture buffer is sized from the device's real sample rate instead
  of assuming 48 kHz.
- Empty transcriptions from noise no longer reach the ring or the clipboard.
- `banshee.speak` without text returns an error instead of silent success.
- The ready cue now plays only after delivery actually succeeds; a failed
  dictation paste plays the error cue instead.

## [0.1.0] - 2026-06-26

First public release. macOS only for now; Windows and Linux support is planned.

### Added

- Local dictation via a global hotkey: hold `F5` to capture to a mailbox, or
  `Shift + F5` to type straight into the focused app.
- Offline speech-to-text with Whisper (`whisper-rs`), gated by Silero
  voice-activity detection.
- Text-to-speech for spoken status updates (via the macOS `say` command as a
  placeholder backend).
- JSON-RPC API over a Unix socket, with a `banshee` CLI: `serve`, `setup`,
  `status`, `listen`, and `speak`.
- MCP server (`banshee-mcp-shim`) exposing speak and listen tools to MCP-capable
  hosts such as Claude Code, Cursor, and OpenCode.
- Configurable VAD threshold via `config.toml` and the `banshee.configure` RPC,
  reported back through `banshee status`.

[Unreleased]: https://github.com/yamanahlawat/banshee/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/yamanahlawat/banshee/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/yamanahlawat/banshee/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/yamanahlawat/banshee/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/yamanahlawat/banshee/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/yamanahlawat/banshee/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/yamanahlawat/banshee/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/yamanahlawat/banshee/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/yamanahlawat/banshee/releases/tag/v0.1.0
