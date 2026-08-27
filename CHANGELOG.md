# Changelog

All notable changes to Banshee are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **The daemon reports when it listens for an answer and when it transcribes.**
  A client subscribed to `state` sees `armed` and `transcribing` beside `recording`
  and `speaking`. `recording` still means the microphone is open, so it stays true
  while armed. A client ranks the four in order: `transcribing`, `armed`, `recording`,
  `speaking`.
- **Voices have names.** `banshee voices` marks the one in use and prints
  `* Sky  American, clear  (af_sky)` for each installed voice, instead of the bare id.
- **Model downloads report which file, of how many, and how big.** A progress
  event carries `label`, `index`, and `count` beside the filename and the byte counts.
- **`banshee status` reports your settings and what waits for a restart.** The
  reply carries the parsed `config.toml` and a sorted list of keys the daemon
  wrote but has not applied.
- **Agents and permission panes answer over the socket.** `banshee.agents`,
  `banshee.connect_plan`, `banshee.connect_apply`, and `banshee.open_permission` do
  what `banshee connect` and `banshee permissions` already do. Any client can offer
  them with no daemon code linked in.
- **You can hear a voice before you choose it.** `banshee.speak` takes an
  optional `voice` for one sentence and leaves your configured voice untouched.
- **`banshee.history` takes a `limit`.** An absent `limit` still returns every
  row. An explicit `0` returns none.
- **The tray menu copies your last dictation.** `Copy last dictation` puts it on
  the clipboard. `Open Banshee (coming soon)` sits next to it, disabled, until
  the app it opens exists.

### Fixed

- **The daemon and `banshee connect` now report the same agents.** The daemon
  reads your login shell's `PATH`, so it sees the agents you have installed
  rather than only the four system directories launchd gives it. What remains: a
  slow shell profile delays every client, not only the first. The `PATH` resolves
  once, behind a `OnceLock`, the first time anything needs it, and every
  concurrent caller waits on that same call. No timeout guards the wait, because
  nothing has measured how long a shell profile should take.
- **A bare command name no longer counts as connected.** Banshee used to ask
  whether `banshee-mcp-shim` resolves on `PATH`. Your agent resolves that name,
  not Banshee, so neither the daemon nor the CLI could answer, and they answered
  differently. An agent registered with the bare name now reads as installed and
  not connected, and `banshee connect <agent>` rewrites the entry to the shim's
  absolute path, which depends on no `PATH` at all. Run it once per agent. What
  remains: if you set `CLAUDE_CONFIG_DIR` in your shell, the daemon cannot see
  that variable and reads the default config instead, so the two can still
  disagree for Claude Code.

## [0.11.1] - 2026-08-26

### Fixed

- **Stopping speech no longer hangs the daemon when the speaker is gone.** Disconnect a
  Bluetooth headset that Banshee was speaking through, interrupt an utterance, and the
  daemon stopped answering anything: `banshee status` timed out, the tray froze on its last
  state, and nothing spoke until a restart. The audio library's `append` waits for a stopped
  player to drain, a player whose device vanished never drains, and that wait happened under
  a lock every stop needs. Each utterance now plays through its own player, so a stop never
  waits on the device, and a stopped utterance never blocks the next one. What remains until
  the headset returns or the daemon restarts: no sound; an utterance that ends on the dead
  device keeps `speaking` true until something stops it, so further speech queues up and the
  oldest is dropped; and each utterance leaves its audio parked in the mixer. That is the
  output half of the device change work, on the roadmap.

## [0.11.0] - 2026-08-26

### Added

- **`banshee connect` wires up your coding agent.** `banshee connect antigravity`,
  `claude`, `codex`, `cursor`, `opencode` or `pi` detects the tool, shows the exact
  change to its config, and writes it only after you say yes. Claude Code gets the MCP
  server and a stop hook that makes each turn end with a spoken status; the others get
  the MCP server entry, and Pi its native extension. `banshee connect` alone lists what
  is installed and what is connected. A server already registered by its bare name
  counts as connected when `PATH` resolves it to the same shim.

### Fixed

- **A microphone that disappears no longer kills dictation.** Unplug a Bluetooth
  headset mid-session and Banshee used to keep naming the device that was gone,
  record nothing, and play no cue. A restart was the only way back. Now the
  daemon notices within a second or two, moves capture to the system default so
  a press still works, and takes your device back when it returns.

  **It never swaps your microphone in silence.** When you named a device and it
  disappears, every surface says which one is recording and which one it is
  waiting for: the tray menu, `banshee status`, and `banshee watch --waybar` all
  show `MacBook Pro Microphone (waiting for "yeti")`. When no device can be
  opened at all, Banshee refuses to record and says why, rather than returning
  silence.

  This also covers the case where the microphone is already missing when the
  daemon starts, so booting with the headset off no longer needs a restart once
  you connect it.

  **`audio.input_device = "default"` now follows the OS.** Connect a headset,
  macOS makes it the system default, and Banshee moves capture to it within
  about five seconds. It used to keep whatever was the default when capture last
  opened, so a headset connected mid-session went unheard. A device you named by
  hand is never given up this way.

  Two limits are worth knowing. If you have no working microphone at all, for
  example because the permission is denied, then granting it still needs a
  daemon restart. And a headset that is both your microphone and your speakers
  loses its cue tones until you reconnect it, because the output side is not
  rebuilt yet.

### Changed

- **A source install is now a signed `Banshee.app`.** `make install` from a
  clone builds the bundle, so macOS shows Banshee's icon in System Settings
  instead of a generic placeholder. The command line `banshee` is unchanged
  and still works. Homebrew and the shell installer are not affected: they
  ship the same binaries as before, signed with the same certificate, so
  nothing below applies to them.

  **If you install from source, you must grant two permissions again, once.**
  The binaries inside the bundle sign under the bundle's identifier, and macOS
  ties each permission to that identity. After you upgrade, open System
  Settings > Privacy & Security and grant Banshee **Accessibility** and
  **Input Monitoring** again. Until you do, the hotkey receives no key presses
  and dictation cannot type. This is expected, and it happens only on this
  upgrade.

  **Input Monitoring may need one extra step.** The list can keep a stale
  Banshee entry, shown with an alias or shortcut arrow, and then show no
  Banshee entry at all once you remove it. Banshee never prompts for
  permissions, it only checks them, so macOS does not always add a fresh row
  on its own. Remove any stale entry, then use the **+** button to add
  `~/Applications/Banshee.app` directly. Accessibility does not need this
  step; its row appears on its own.

- **`audio.input_device` applies without a restart.**
  `banshee config set audio.input_device "yeti"` now reaches the running daemon,
  which rebinds capture in well under a second. Every other setting except
  `stt.vad_threshold` is still read once at startup.

- **An exact microphone name now wins over a longer name that contains it.**
  `input_device` still matches a case-insensitive substring, so `"yeti"` finds
  `Blue Yeti Stereo Microphone`. But a `Yeti` sitting beside a `Blue Yeti Pro`
  now opens its own device rather than the first match. A blank
  `input_device` is read as `default` instead of matching whichever device
  enumerated first.

## [0.10.0] - 2026-08-23

### Added

- **The hotkey is configurable.** `banshee config set audio.hotkey RightOption`,
  restart, done. A binding is an F-key (`F1`-`F12`), a modifier pressed alone
  (`RightOption`, `LeftOption`, `LeftControl`, `LeftCommand`, plus
  `RightCommand` and `Fn` on macOS), or modifiers and a key, as in
  `Ctrl+Alt+D`. A binding the listener could never match refuses to parse and
  names the legal forms: F13 does not exist to the key library, Right Control
  never arrives on macOS, and Shift stays reserved as the mailbox modifier.

  A modifier bound alone still works as a modifier. `RightOption+E` types é,
  Option+click stays a click, and the accidental recording is discarded
  quietly. In toggle mode a tap acts on the release, so a press that becomes a
  chord toggles nothing. Typing a chord's bare letter neither starts nor stops
  a session.

  The daemon's own paste is fenced off from its own listener: dictation types
  into the same event stream the hotkey is read from, so without the fence a
  `LeftCommand` binding would trigger itself on every dictation.

### Changed

- **The default hotkey is Right Option.** A lone right-hand modifier types
  nothing, sits under the thumb, and works on both platforms; on Mac keyboards
  a bare F5 belongs to Apple's own Dictation. **This changes behavior on any
  machine whose config does not set `audio.hotkey`** — to keep F5, run
  `banshee config set audio.hotkey F5`.
- `banshee status` names the key in use, not just the mode: `hotkey F5 hold`
  is now, say, `hotkey RightOption toggle`.

### Fixed

- **A modifier hotkey no longer fires itself after dictation.** On macOS the
  paste pressed Command as a key event of its own, which desynchronised the
  system's modifier state: the next press of that modifier arrived as a release
  with no press, and a lone-modifier binding read it as one tap that both
  started and stopped a recording. The paste now sets Command as a flag on the
  keystroke and emits no modifier event at all. A release also only ends a
  session whose press was seen.

## [0.9.0] - 2026-08-22

### Added

- **A menu bar icon on macOS.** `banshee tray` puts an indicator in the menu bar
  that answers one question: can I speak right now. Four states, told apart by
  shape and never by colour alone, so a tinted menu bar and a colour vision
  difference both stay readable: an outline shroud when idle, a filled one while
  recording, arcs at the shoulders while speaking, and a broken outline when the
  daemon is not running. The icon is a template image, so macOS tints it for
  light and dark. Its menu names the state in words and the microphone in use,
  and quits from there. `banshee tray --uninstall` removes it.

  It runs as its own process with its own launch agent, not as a child of the
  daemon. AppKit has to own the main thread, which belongs to tokio in the
  daemon, and a separate lifetime is what lets the icon report that the daemon
  is down rather than vanishing with it. It reads the socket and nothing else,
  so it asks for no permissions of its own.

- **Banshee has a mark.** A shrouded figure, drawn once and used everywhere: as
  the four menu bar states, and in colour as the app icon that now heads the
  README.

### Changed

- `banshee status` reads in causal order, so a failure that causes the ones
  below it comes first. A stopped daemon is now a failure rather than a note,
  because nothing records without it. Check after `banshee start`, not before.

- A missing permission names whose grant was read. TCC answers for the process
  that asked, so a grant read from the CLI says nothing about a daemon that
  launchd started.

- A microphone that will not open names all three causes it could be, since
  Core Audio does not say which: a denied grant, a disconnected device, or an
  `[audio] input_device` that names something absent.

- A `config.toml` that will not parse reports only the first line of the error
  and names the file's path.

- `banshee service uninstall` removes every launch agent Banshee installed, so
  none is left behind to fail at the next login.

## [0.8.0] - 2026-08-21

**Upgrading from 0.7.0 or earlier asks for macOS permissions once more.** This is
the first signed release. Earlier builds carried an ad-hoc signature, which
changes on every build, so macOS could not tell one version from the next and
dropped the grants each time. Releases are now signed with a stable certificate,
so grant Accessibility, Input Monitoring and Microphone one final time and no
later upgrade will ask again.

### Added

- `banshee watch --waybar` emits one Waybar custom-module object per line, so
  the microphone state can sit in a Wayland bar with no tray and no GUI. `text`
  shows, `alt` selects a `format-icons` entry, and `class` is the CSS hook.
  Readiness is left out on purpose: the daemon answers it once at connect and
  never pushes it, so a bar showing it would go stale. Set `restart-interval`
  in Waybar, since the command exits when the daemon stops.

- **Model downloads resume.** Each file streams to `<name>.part` and is renamed
  into place only when complete, so an interrupted download is never mistaken
  for a model and the next `banshee setup` continues from where it stopped
  instead of starting over. Downloads also no longer hold the whole file in
  memory, which was 547 MB for the balanced preset.

- `banshee.download_models` starts a download in the daemon, and subscribing to
  `downloads` follows it: `banshee.download_progress` reports `model`, `bytes`,
  `total`, and a state of `downloading`, `done`, or `failed`. One download runs
  at a time, and a second call is refused, because the partial file that makes
  resume possible cannot have two writers. `banshee setup` asks a running daemon
  rather than downloading alongside it.

- `banshee.subscribe` takes `{"events": ["state", "downloads"]}`, defaulting to
  `["state"]`, so a client that only wants one kind is not sent the other.

- `banshee voices` lists the text-to-speech voices on disk and marks the one the
  daemon loaded, so `tts.voice` can be set to a name you have seen. Only
  downloaded voices are listed, so every name it prints works today. Nothing is
  marked in use when Kokoro did not load, because the system fallback speaks in
  the voice macOS is set to and Banshee did not choose it. The daemon answers the
  same question over `banshee.list_voices`, and enumeration needs no daemon.

- `banshee watch` follows the daemon and prints one word per state change:
  `idle`, `recording`, or `speaking`. The first line is the state at the moment
  you connect, so nothing has to be guessed, and a state that did not move is
  not printed again. Clients get the same channel over `banshee.subscribe`,
  which answers with everything `banshee.status` reports and then pushes
  `banshee.state_changed` notifications on that connection. The subscription
  lives and dies with the connection, so there is nothing to unsubscribe. This
  replaces asking `banshee.status` on a timer, which made an indicator lag the
  microphone.

- `banshee devices` lists the microphones and marks which one the daemon opened,
  so `audio.input_device` can be set to a name you have seen rather than one you
  guessed. The daemon answers the same question over
  `banshee.list_input_devices`. Enumeration needs no daemon, which matters
  because you need the names before you can start one on the right microphone.
  The list does not say whether each device opens: probing them all would steal
  the microphone from the running daemon.

- `banshee config set <key> <value>` writes one setting to `config.toml`. The key
  is the section and the field, as in `stt.language`. Comments and layout
  survive, because the file is edited rather than rewritten. A value the field
  does not accept is refused, and the message lists the ones it does.
  `vad_threshold` takes effect at once; everything else needs a restart, and the
  command says so.

### Fixed

- `banshee setup` reported success after a failed download. The result was
  discarded, so a network error printed nothing and exited 0, and the only
  symptom was `banshee status` still reporting the model missing.

### Changed

- **`banshee status`, `banshee doctor` and `banshee readiness` are now one
  command.** All three answered overlapping versions of "is it working", and
  `readiness` reported nothing `doctor` did not. `banshee status` is the
  checklist, and `banshee status --json` is the machine-readable state. The
  daemon's `banshee.readiness` method is gone the same way: `banshee.status` now
  carries `ready` and `blockers`, and no longer carries `recording_error`, which
  was the same failure in a second wire shape without a fix attached.

- **The checklist now asks the daemon whether its permissions are granted.** It
  used to check the process it was running in, which cannot speak for a daemon
  launchd started; the code said as much in a comment and did it anyway. Grant a
  permission while the daemon runs and the old output showed a green tick for a
  daemon that was still blind.

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

[Unreleased]: https://github.com/yamanahlawat/banshee/compare/v0.11.1...HEAD
[0.11.1]: https://github.com/yamanahlawat/banshee/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/yamanahlawat/banshee/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/yamanahlawat/banshee/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/yamanahlawat/banshee/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/yamanahlawat/banshee/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/yamanahlawat/banshee/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/yamanahlawat/banshee/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/yamanahlawat/banshee/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/yamanahlawat/banshee/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/yamanahlawat/banshee/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/yamanahlawat/banshee/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/yamanahlawat/banshee/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/yamanahlawat/banshee/releases/tag/v0.1.0
