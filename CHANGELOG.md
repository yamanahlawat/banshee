# Changelog

All notable changes to Banshee are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/yamanahlawat/banshee/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yamanahlawat/banshee/releases/tag/v0.1.0
