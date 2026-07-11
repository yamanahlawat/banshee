# Changelog

All notable changes to Banshee are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/yamanahlawat/banshee/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/yamanahlawat/banshee/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/yamanahlawat/banshee/releases/tag/v0.1.0
