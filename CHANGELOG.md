# Changelog

All notable changes to Banshee are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **A small rust capsule above the Dock shows what Banshee does.** On
  macOS, the Banshee figure appears there while Banshee records, works, waits
  for your answer or fails, and shows nothing at rest. The figure moves with
  your voice. When an agent asks a question, it waits in headphones and tilts
  its head in once you start answering. The menu bar icon draws this
  figure. A new install starts on On screen, or on Both when VoiceOver is on.
- **`banshee watch --events cues,level` prints each event as a JSON line.** Follow the
  earcons and the microphone level from a script.
- **`banshee.subscribe` takes the events `cues` and `level`, and a `draws` flag.** `draws`
  marks the connection as a chip that draws the figure, so `visual` stays silent for it.
- **`transcribing` now also covers the speech check and the typing.** `banshee watch`,
  waybar and the menu bar icon report busy for the whole handling, not only the
  transcription itself.

### Changed

- **Breaking: Claude Code, Codex and Antigravity are sent back once to speak when a turn ends
  in silence.** Each gets a Stop hook that runs `banshee turn-end`. The daemon counts each
  agent's voice calls, so no hook reads a transcript and nothing needs `jq`.
  Rerun `banshee connect claude`: it replaces the old hook and removes
  `banshee-speak-check.sh`. Rerun `banshee connect codex` and `banshee connect antigravity` to
  add theirs, and trust the Codex hook once in `/hooks`. Restart each agent after.
- **Breaking: `feedback.mode` replaces `audio.cues.enabled`.** It takes `sound`, `both`,
  `visual` or `none`, and the window's Sounds row is now Feedback. `visual` (On screen in
  the window) plays no earcons while the menu bar icon draws the figure, and plays every
  sound with no menu bar icon running. `both` (Both in the window) adds every sound to the
  figure. A file that still sets `audio.cues.enabled = false` reads as `none` until it sets
  `feedback.mode`. `banshee config set audio.cues.enabled` is refused and names the new key.

### Fixed

- **A daemon stopped during a command no longer blocks the next one.** Banshee
  refused every command as "already running" for five and a half minutes. The
  lock now frees when the process that holds it exits. A lock file that cannot
  open now gives its path, not "already running".
- **A command that runs out of time keeps its thread.** "Show me" now opens the
  thread that hung, so you see how far the agent got. Your next command continues
  it. Claude Code now runs with `--output-format stream-json`, so it names its thread
  on its first line, as OpenCode does.
- **A reply arrives even when the agent's server outlives the run.** OpenCode leaves
  its server holding the output open. Banshee lost the reply after a 2-second
  wait. A new thread was lost with it. The warning said "its output did not
  arrive in time". Banshee now keeps what arrived, and the warning is gone.
- **"Show me" says how old the saved thread is.** It said "The last thread has timed
  out", which read as the run timing out. It now gives the age of the thread.
- **`banshee watch | head -n 1` ends.** `watch` stopped only at its next write, and an
  idle daemon sends none. It now ends when its reader closes the pipe. Measured on
  Linux. The macOS path is not measured.

## [0.15.1] - 2026-09-22

### Added

- **The Microphone panel fetches a speech model it lacks.** It names what the
  download costs and offers it in the row. The state word reads Working while
  the file is read, and the model takes over without a restart.
- **The record ends where it starts.** One line under the oldest turn names the
  day it begins and says Banshee keeps it on this machine.
- **The foot's four values read as the controls they are.** Each carries a rule
  at rest; an open cell drops it and keeps the accent bar above.
- **The record mounts only what is on screen.** Thousands of turns scroll with
  no paging control, and find still reads every row.
- **The Voice panel sets the audio format of a remote speaker.** A `WAV`/`PCM`
  choice sits above the speech error, and a sample rate field shows only under
  `PCM`. An empty rate field goes back to 24000 Hz. The change takes effect when
  Banshee restarts. The "First audio" log line now names the rate and channel
  count of the reply. In the Voice and Microphone panels, the key now sits
  under the server it belongs to.
- **The header names a failed reply or dictation, and opens the panel that holds
  it.** The notice sits beside the state word. Pressing it opens the Voice or
  Microphone panel at the failure, and leaving that panel returns to the notice.
  The failure itself now reads as two lines, Banshee's own words and then the
  server's, with a control that copies both. The window speaks a failure when it
  arrives, even when no panel is open.
- **Opening the app puts the `banshee` command on your PATH.** The macOS
  tarball placed nothing there. It links into `/usr/local/bin`, leaves a
  `banshee` that already answers alone, and `banshee uninstall` takes it away.
  A clean macOS has no `/usr/local/bin`, and
  [install.md](docs/install.md) carries the line that makes it.

### Fixed

- **A long record scrolls to the right place.** Row heights were measured
  without the margin that spaces every turn, so the space held for the turns
  above the viewport fell short by that much for each of them.
- **A download starts once however fast you press.** The daemon answers before
  the transfer begins, so every control that starts one stayed live in between
  and a second press was refused as though it had failed.
- **The Voice panel shows a voice's whole description.** The row carried a label
  saying a voice was absent beside a Play nobody could press; one Get replaces
  both. The arrow keys now take a voice without fetching it.
- **A panic while reading a speech model no longer leaves Banshee reading
  Working.** A load that merely failed already cleared the flag; one that
  unwound did not.
- **Choosing a heavier speech model offers its download.** The window judged
  what was missing by the model already loaded, so a preset whose file was
  absent raised nothing to press and no figure anywhere.
- **The Language note no longer names a model you did not choose.** It says the
  chosen model is not in force yet, and leaves the language reachable.
- **A failed dictation names a panel the window shows.** The notice said
  "Microphone", and the cell that opens it is labelled Listening. It carries
  the button that opens it too.
- **The window reads the daemon again when you return to it.** A grant made in
  System Settings, a hand-edited `config.toml` and a model file added outside
  Banshee reach no push, so the window could show a setting that had moved.
- **The download says what it costs and that it survives.** It names the size
  the percentage is a share of, and says the transfer resumes and keeps going
  if the window closes.
- **The Voice panel states no size it does not hold.** Every undownloaded voice
  carried the same invented figure.
- **A `banshee` on your `PATH` that points at another install stays.** Linking
  the command took any link at that name, so a second copy of Banshee could
  have its link repointed. Only a link nobody can follow is replaced now, and
  the advised command frees such a name rather than failing on it.
- **The Copy button under a failure no longer says Copied for the next
  failure.** The confirmation was keyed to the panel's row, not to the text, so
  a second failure inside 1.5 seconds left it standing over words nobody
  copied.
- **The window no longer reports an installed agent as absent.** The daemon's
  own shell could not see an install under `~/.local/bin`, or one an rc file
  adds, such as npm. It now searches that directory, asks an interactive shell
  first, re-checks when the Agents panel opens, and logs why a probe failed.
- **"Skip to the jobs" now enters the foot where you left it.** It always landed
  on Microphone, whichever job the foot held, so the cell it focused was not the
  one Tab leaves from. The link is a keyboard route past the record's copy
  controls, and it is on screen only while it holds focus.
- **A remote speaker under `wav` no longer sends `sample_rate`.** Groq wrote the
  requested rate into the WAV header without resampling, so a reply asked at
  44100 Hz played 1.8 times too fast. A WAV header states its own rate, so
  Banshee now sends the field only under `pcm`.

## [0.15.0] - 2026-09-19

### Added

- **`banshee uninstall` undoes what Banshee installed.** It stops the daemon,
  takes both login entries out, and names the tool that owns the rest:
  Homebrew's copy stays Homebrew's, because deleting files it records leaves the
  records pointing at nothing. A copy from the shell installer or the tarball is
  removed here. A source build records nothing, so its files stay, and the
  command names the binary it runs from. `~/.banshee` holds the models, the
  history and the keys, and it stays unless `--data` asks for it. Nothing is
  removed without a yes, and a script with no terminal is told to pass `--yes`
  rather than asked a question nobody will see.

- **`banshee status` says why a headset sounds dull.** A Bluetooth headset gives
  macOS its microphone or its speaker in full quality, never both, so while
  Banshee holds the microphone the same device plays at 16 kHz. The line appears
  when the open microphone and the default speaker are the same device and its
  rate has dropped, and it names the rate. Pinning `[audio] input_device` to
  another microphone is the way out.

- **`banshee-update` comes with the shell installer.** That route had no update
  path at all: Homebrew has `brew upgrade`, and the downloaded app is replaced by
  running its own command again, but the installer's route had nothing.

### Changed

- **`banshee start` starts, and downloads nothing.** A first run no longer
  spends ~860 MB before you have seen that speech models come in three sizes.
  `banshee start` names the models that are missing and stops; `banshee setup`
  fetches them. In the window, the download box asks first and carries the
  preset chooser, so `fast` or `quality` is picked before the bytes move. A
  daemon that was already running built its pipeline without those files, so
  `banshee setup` now says when a restart is what loads them.

- **Every line in the daemon log carries a clock and a level, and a dictated
  sentence reaches it only when asked for.** Lines read
  `12:00:01.500 INFO  hotkey: Transcribed 3.2s of audio in 0.41s`. The text
  Banshee heard, typed or told the agent is logged at `debug`. A supervisor
  hands the daemon an environment of its own, so `BANSHEE_LOG=debug banshee
  start` writes the level into the login service file and the daemon runs at
  it. Use `BANSHEE_LOG=warn banshee start` for failures alone, and
  `banshee start` on its own to go back to the default.

- **The voice detector asks for one thread instead of four, and Kokoro asks for
  the core count up to eight instead of a fixed four.** The detector reads 0.06 ms
  at one, two and four threads alike, so the extra three bought nothing on every
  chunk the microphone captured. Kokoro does scale: one utterance measured 484 ms
  at one thread and 155 at eight on a 24-thread machine, and 1001 at one, 388 at
  four and 243 at eight on a 15-thread one. It now asks for no more threads than
  the machine holds, so a dual-core laptop is no longer oversubscribed either.

- **A request whose `jsonrpc` field is not `"2.0"` is not answered.** The
  daemon read the field as free text and answered any value. It now parses only
  the version it speaks, as its replies already declared.

- **The window and the daemon upgrade together.** Banshee no longer carries the
  paths that let a new `banshee` command read an older running daemon. Upgrade
  and then run `banshee start`, which every install path already does. A daemon
  older than 0.8.0 left running now reports that its checklist cannot be read,
  rather than reporting a shorter one.

### Removed

- **A `[logging]` table in `config.toml` is refused by name.** It was parsed and
  ignored since 0.11.1. Delete the table and the file loads again.

### Fixed

- **A microphone that delivers only whole numbers records.** Banshee asked every
  microphone for floating-point samples. A raw ALSA device, such as a USB
  microphone opened without PipeWire, can refuse them. Capture then failed with
  "Sample format f32 is not supported". Banshee now opens the format that the
  device offers, and converts each sample to floating point.

- **The window says ready as soon as the microphone opens.** The window read
  `ready` only with the full status, and the daemon pushed no word when the
  microphone finished opening or broke. The window could keep the old state
  until something else asked again. The daemon now pushes the microphone state,
  and the window reads the status again when it changes.

- **A reply never plays into a sound card that PipeWire does not hold.** When
  the default output would not open, Banshee opened whatever other device
  would. On Linux, with PipeWire restarting or stopped, that was a raw sound
  card. The card took the audio, so every later reply played into it, and the
  headset stayed silent until the daemon restarted. On Linux, Banshee now opens
  the default output only. A reply that meets no output ends, and it says why. The
  next reply opens the default again, and while there is still no output,
  `banshee speak` fails with the reason and does not answer `ok`.

- **The window opens at its own height on Hyprland.** Hyprland sized the new
  window as a tile before it read the window's size limits, so the window
  floated at the full height of the screen. The window now asks for its
  configured size when it opens.

- **A microphone that stays gone is logged once.** Banshee tries it again
  every 5 seconds and wrote an error line each time, 720 lines an hour for one
  fault. It now writes the fault once, and again only when the reason changes.

- **A quarantined Banshee says so instead of dying silently.** macOS kills the
  `banshee` command with no message at all when the app still carries Homebrew's
  quarantine flag, which made a blocked install look like a broken daemon. The
  cask now writes `banshee` and `banshee-mcp-shim` as small wrappers outside the
  app, because macOS kills anything run from inside a quarantined bundle, a
  shell script included. They name what happened, and in a terminal offer to
  clear the flag. They ask first, they only ever ask a person, and a hook or an
  agent gets the line to run rather than a prompt nobody can answer. The flag
  returns with every `brew upgrade`, which the caveat and the docs now say.

- **The earcons come out of the same speaker as the voice.** The cue player held
  an audio device of its own, opened once and never again, so a device that went
  away left every later beep unheard while the voice carried on elsewhere. Every
  sound the daemon makes now goes through one output, which follows the device
  and is opened only when there is something to play, so cues turned off still
  hold no audio hardware.

- **A question follows the microphone.** A device that changed while Banshee was
  already listening left the answer unheard: the new microphone arrived as a
  command, and the question itself was holding the thread that reads commands,
  so it went on reading a device that no longer existed until it timed out. The
  capture is now shared, so a question already listening reads whatever
  microphone is there, keeps what was said before the change, and rebuilds only
  what belonged to the old device.

- **A reply follows the speaker.** When the device Banshee was playing through
  disappeared, the rest of the reply went into it and was never heard, the daemon
  believed it was still speaking, and every later reply queued behind a device
  that was gone until it was restarted. Banshee now notices that nothing is
  taking the audio, opens the device that is default now, and carries on,
  repeating at most the sentence that was cut.

- **A box that stands above the record lines up with the boxes beside it.** A
  drawn absence is indented to the turn text column, which is right where it
  stands in for a turn. "Banshee is not running" stands above the record, where
  the indent aligned it to a column no turn was drawing and broke the left edge
  it shares with the blocker above it. It now takes the band gutter.

- **A question no longer outlives the agent that asked it.** `ask_user` holds
  the microphone until someone answers, and an agent that died in the meantime
  went unnoticed, so the session listened on to its timeout and every other
  question was refused as busy. The daemon now watches the connection while a
  call runs and closes the session when the caller goes, while a request sent
  during a call is still held and answered.

- **The Accessibility advice covers a switch that is already on.** macOS keys
  that grant to the signature of the build that asked for it, so a reinstall or
  an update can leave a row that is listed and switched on while the grant
  reaches nothing, and no prompt appears because a record already exists.
  Banshee now names the repair: remove Banshee from the list with the minus
  button and add it back.

- **`banshee status` says what happened instead of quoting a JSON parser.** A
  daemon that closed the connection was reported as `EOF while parsing a value
  at line 1 column 0`, and a socket file left behind was called a crash, though
  a clean stop leaves the same file. The checklist now names the state it found
  and offers a second look before a restart, since a daemon that is still
  starting answers nothing and restarting only starts the wait again.

- **Granting Accessibility no longer kills a running download.** Banshee has
  to start again for a grant to reach it, and it used to leave the moment one
  landed, taking the first-run download with it and leaving the socket file
  behind for the next run to report as a crash. It now waits for the download
  to finish, then leaves the way a stop does, with nothing left behind.

- **A slow microphone no longer makes Banshee unreachable.** The daemon opens
  its socket before it touches the audio devices, so a device whose driver
  stops answering leaves Banshee answering. Walking the devices enters Core
  Audio, which was measured stalling for minutes on this machine, and every
  client that connected in that window waited with it: `banshee status` said
  the daemon answered the socket but not the call, and restarting only started
  the wait again. `banshee status` now reports "the microphone is still
  opening" while it waits, and a press before it opens answers with the error
  cue.

- **A reply that never starts no longer leaves the daemon deaf.** When a new
  reply interrupted one already playing and the speaker then refused it, for
  example a voice that is not installed, Banshee went on believing it was
  speaking. The hotkey listener drops every sound it captures while that is
  true, so the microphone stopped answering until the next reply played.

- **A `config.toml` that does not parse names itself.** The message pointed at
  the line at fault but not at the file it was in, and Banshee reads two toml
  files.

- **`banshee.speak` with a voice the speaker cannot take answers an
  invalid-params error, not an internal one.** The system voice and a remote
  speaker take no per-utterance voice, and Kokoro refuses a voice that is not
  installed. Each said so under the internal-error code.

- **An error reads as the sentence it was written as.** Every failure that was
  not a refusal or an answer from the daemon printed `Internal error:` in front
  of its text, so `config.toml does not parse` and `home dir not found` each
  called themselves internal. A typing failure on dictation printed its Rust
  form in the daemon log; it prints the sentence now.

- **`banshee start` writes its login service file whole, and finds `launchctl`,
  `systemctl` and `open` under a supervisor's short PATH.** The launchd plist
  and the systemd unit were written in place, so a crash mid-write left a file
  the next login could not start. Every file Banshee writes now lands on disk
  before it replaces the old one.

- **`banshee status` reports a `credentials.toml` that others can read, with
  the `chmod` that fixes it.** Banshee writes the file owner-only, but a copy
  made by hand or restored from a backup kept whatever mode it came with.

- **A Kokoro sentence that fails to synthesise sounds the error cue and shows
  in `banshee status`, as a remote speaker's failure already did.** It wrote
  one line to the daemon log and the reply went silent with no other sign.

- **A `banshee` command that fails exits with status 1 and prints the reason
  as a sentence.** `stop`, `listen`, `speak`, `history`, `clear-history` and
  `record` reported a failure and exited 0, so a script could not see it. An
  error that reached the top printed its Rust form, such as
  `Error: Rejected("...")`; it now prints the text alone.

- **The MCP shim answers an unknown tool and stays silent on a notification.**
  A call to a tool name the shim does not serve got no reply at all, so the
  agent waited until its own timeout. It now answers an invalid-params error
  that names the tool. A notification such as `notifications/cancelled` got a
  method-not-found error with no id, which JSON-RPC forbids. The shim now
  sends nothing for any message without an id. A tool call the daemon refuses
  comes back as a tool result marked as an error, with the daemon's own words,
  so the agent reads why; it was a protocol error with an internal-error code
  before. A line that is not a JSON-RPC request gets a parse error from the
  shim and from the daemon's socket, where both dropped it in silence. The
  shim's own log lines carry a clock and a level like the daemon's, and
  `BANSHEE_LOG=debug` in the MCP server's environment names each tool call.

- **A version number, a decimal and a year are spoken as what they are.**
  `0.12.1` reads "zero twelve one" instead of "zero one two one", `1.2` reads
  "one point two" instead of "one two", and `2026` reads "twenty twenty six"
  instead of "two thousand and twenty six". A two-part version such as `1.10`
  is still read as a decimal, so it says "one point one". The same upgrade lets
  commas and full stops reach Kokoro as pause cues, which changes pacing
  slightly throughout.

- **A version number, a decimal or a file name is spoken as one phrase, not cut
  into separate sentences.** Banshee ended a sentence at every `.`, so
  `0.12.1` became three chunks with a pause inside the number, and
  `config.toml` became two. A terminator now ends a sentence only where one
  ends: before whitespace, at the end of the text, or after a closing quote or
  bracket. An abbreviation such as `e.g.` still ends a chunk, and a reply with
  no space after a full stop is spoken as a single chunk.

## [0.14.0] - 2026-09-16

### Added

- **`banshee tell "<text>"` sends a command to your coding agent, which
  changes your desktop.** Banshee sends your words and nothing else: the
  agent's own skills carry the desktop knowledge. The agent edits the config
  and speaks the result back through Banshee. `banshee bind hyprland` now
  writes a tell key beside the dictate key: the dictate chord plus the first
  free modifier of `Super`, `Ctrl` or `Alt`, and a key that already holds all
  three gets none, which `bind` says. Within `tell.thread_timeout_min` the
  next command continues the same conversation, so "a bit more" works.
  "start over" ends the thread and starts no agent. "show me" reopens the
  thread in a terminal, so you can read what the agent wrote. Before
  each agent command Banshee copies the folders in `tell.paths` to
  `~/.banshee/tell/snapshots/`, keeps the newest `tell.snapshots` copies, and
  `banshee tell --undo` puts the newest back. The agent runs in
  `~/.banshee/tell/run/`, so it never lists Banshee's own snapshots and edits a
  copy of your config. Claude Code gets those folders, and the run directory it
  works in. OpenCode takes no folder list, so it can
  edit anything: `banshee tell` prints which of the two it is on the first
  command of a thread, and `docs/configuration.md` states the difference.
  Banshee itself speaks nothing. A failed run sounds the error cue, and so does
  a run that finished after Banshee refused it a tool, because that one leaves
  you with silence. A reset that works sounds nothing at all. `banshee status`
  names the reason, the agent it would run, and whether that agent is scoped,
  and it keeps the last tell failure until the next tell run, so a dictation in
  between no longer hides it. The cues carry every tell failure, so
  `banshee status` says when `audio.cues.enabled` is false.
  `tell.run_timeout_min` defaults to 5 minutes, which is a stated default and
  not a measurement: no run has been timed to a limit.

## [0.13.1] - 2026-09-14

### Added

- **`banshee bind hyprland` binds the key in your Hyprland config.** It asks
  for the key and whether you hold it or tap it, and writes the matching block
  to `~/.config/hypr/bindings.lua` on Omarchy, or to
  `~/.config/hypr/hyprland.conf` elsewhere. It replaces only the lines between
  its `BEGIN BANSHEE MANAGED BLOCK` and `END BANSHEE MANAGED BLOCK` markers.
  It names any older `banshee record` line outside them by file and line, and
  removes none. It shows the change first, and runs `hyprctl reload`. It saves both answers as `audio.hotkey` and
  `audio.hotkey_mode`. A lone modifier is refused, because Hyprland fires its
  binds only on the release. `--yes` binds the saved key and mode.
  `banshee bind` alone prints the block for you to paste, and names the file
  it would write to. On macOS it says the daemon binds the key itself and
  names `banshee config set audio.hotkey`.
- **`banshee record toggle` is one key that starts a recording and stops
  it.** `banshee record toggle --dictate` types what it hears. `banshee
  record toggle` records without typing. It suits a compositor with no
  release bind.
- **`banshee start` downloads the models it is missing.** It says so, then
  runs the download after it starts the daemon, the way `banshee setup`
  does. The hotkey hint prints after the download. Ctrl-C leaves the daemon
  running and the download resumable. `banshee setup` stays as the
  standalone command and as the re-run that fetches only what is missing.
- **A remote voice, when you want one.** `tts.provider = "remote"` sends each
  reply's text to the OpenAI-compatible server in `[tts.remote]`. Banshee plays
  the audio as it streams back, so the first words start before the server has
  finished. Name the server, the model, the voice and an optional
  `instructions` line. Set the key with `banshee config set tts.remote.api_key`
  or in the window. It lives in the same owner-only file as the listener's, in
  its own table. `response_format` asks the server for `wav` or `pcm`, and the
  default `wav` states its own rate, so a server that answers at 22050 Hz plays
  right. Banshee identifies every answer from its own bytes and refuses one it
  cannot play by name, so an error page never plays as noise.
  `banshee config remote` now sets up both sides in one go. The
  tray, `banshee status` and the window say when text leaves the machine. An
  utterance the server refuses plays the error tone, says what the server said,
  and the system voice says it instead, so you still hear an agent's question.
  Local stays the default.
- **A remote listener, when you want one.** `stt.provider = "remote"` sends each
  utterance to the OpenAI-compatible server in `[stt.remote]`. Set the key with
  `banshee config set stt.remote.api_key` or in the window; it lives in an
  owner-only file and never in `config.toml`. The tray, `banshee status` and the
  window say when audio leaves the machine, and a failed transcription is
  reported rather than swallowed. Local stays the default. The daemon reads the
  provider and its keys when it starts, so `banshee config set` tells you to
  restart.

### Changed

- **`make install` on Linux builds the desktop window too.** It builds the
  window when WebKitGTK, GTK 3, npm and the Tauri CLI are present. Without
  them it installs the daemon and the CLI, and names what is missing. It
  replaces `make install-window`, which is gone.
- **The README leads with the Linux quickstart.** It gives macOS the same
  weight second, and moves every other install route to `docs/install.md`.
- **The daemon names who listens and who speaks.** `stt.provider` and
  `tts.provider` in `config.toml` name the backend. Each takes `local` or
  `remote`. A status reply carries `remote`, which says whether audio or text
  leaves the machine. A config without the keys reads as before. The daemon
  reads both keys when it starts, so `banshee config set` tells you to restart.

### Removed

- **`barge_in = "duck"`, `daemon.always_on` and the four `audio.cues` file
  paths are refused.** Nothing reads `duck`, `always_on`, or `cues.start`,
  `cues.stop`, `cues.ready`, `cues.error`. A `config.toml` that names one
  fails to load, with the key in the message. `banshee config set` cannot
  write any of them.

### Fixed

- **`banshee connect claude` knows its own Stop hook by the script's exact
  file name.** A hook that runs a script such as `my-banshee-speak-check.sh`
  or `banshee-speak-check.sh.bak` no longer counts as Banshee's, so connect
  adds Banshee's hook beside it. A hook that runs `banshee-speak-check.sh`
  from a path of your own is still left alone. A quoted path that holds a
  space is read whole, so connect names the file you registered.
- **The system fallback voice speaks on Linux too, and names itself when it
  cannot start.** `tts.fallback = "system"` uses `say` on macOS and
  `espeak-ng` on Linux. When the fallback itself fails to start, the reason
  names it: "…, and the fallback voice did not start: …".
- **`banshee status` asks each remote server whether the key works.** After
  the key check passes, it sends the key to the server's `/models` path. It
  passes on an answer, fails a refused key with the `banshee config set`
  command that fixes it, and fails an unreachable server with "check
  `<side>.remote.base_url` and the network". A server with no `/models` path
  earns a note instead of a failure.
- **`stt.remote.base_url` and `tts.remote.base_url` must be a real URL.** Each
  needs an `http` or `https` scheme and a host. A value that is neither is
  refused by name, with `https://api.openai.com/v1` shown as the shape to
  follow.
- **The Microphone panel says when the remote listener never started.** It
  reads "Banshee cannot reach `<host>` to hear you." in place of the sentence
  for a working listener. The model row shows the restart mark when a local or
  remote model change waits on one.
- **A credentials file that will not parse gets its own blocker.** "The
  remote listener's key file is unreadable" names the fault on its own, and
  its fix is to remove the file. `banshee status` and the window both show it.
- **`banshee config remote` writes `tts.provider = "local"` when the voice is
  left empty.** Earlier the wizard left the field alone, so a `remote` config
  from a previous run stayed in place with no voice to send text with, and a
  restart never fixed it.
- **The remote speaker refuses a voice named for one request.** It answers
  only with the voice `[tts.remote]` names, and does not use its own in its
  place, so a caller cannot steer a reply past the configured voice.
- **A malformed answer from the remote listener names no raw client error.**
  It is reported as "the remote listener answered something that was not a
  transcription", and the raw error goes to the log instead, so a status
  reply never carries the server's URL or key.
- **A key typed with a leading or trailing space, or a trailing newline, is
  trimmed before it is written.** A key typed at a prompt carries the newline
  the terminal adds, and a pasted one often carries a leading or trailing
  space.

## [0.13.0] - 2026-09-12

### Added

- **The menu bar icon now runs on Linux.** `banshee tray` puts the mark in any
  bar that hosts StatusNotifierItem, and the desktop window starts it for you.
  The menu names the state and the microphone, copies your last dictation, and
  opens the window. Right click opens it; left click waits on a fix upstream.

- **The desktop window now builds and installs on Linux.** Run `make
  install-window` from a source clone to build it; it needs GTK, WebKitGTK
  and Node 22. The install adds a desktop entry and icons, so your launcher
  finds Banshee.

- **The window now shows a blocker when no Wayland typer is installed.** Off
  macOS, dictation types by shelling out to `wtype` or `ydotool`. `banshee
  status` already named a missing one, but the window's blockers band stayed
  empty. It now lists a blocker for the missing tool, and names `wtype` and
  `ydotool` as the fix.

### Fixed

- **Dictation runs the typing tool it told you about.** The daemon looked up
  `wtype` and `ydotool` in its own `PATH`, while `banshee status` looked in the
  login shell's. A supervised daemon holds the smaller one, so the checklist
  could name a tool the daemon could not run. One search now answers both, and
  every installed tool gets a turn before dictation reports a failure.

- **The window stops naming a hotkey nobody listens for.** On Wayland no
  protocol grants a global hotkey, so Banshee binds none and the compositor
  holds the binding. The window said `Right Option` in five places and offered
  a key-capture control that wrote a setting nothing read. It now says the
  compositor holds it, and the Hotkey panel gives you the two commands to bind.

- **The window's Find hint names a key Linux keyboards have.** The record's
  header said `⌘F`, the macOS Command glyph, on every platform. It now says
  `Ctrl+F` off macOS. The shortcut itself always accepted both.

- **The `ask_user` tool now says why it exists.** Its description tells the agent
  you cannot see the screen, so a question written as text or put in an on-screen
  menu never reaches you. An agent reads this once, when its session starts, so
  restart the agent to pick it up.
- **`english_only` in the status reply follows the model the daemon loaded.**
  A preset applied without `persist` moved the model and left the flag on the
  configured preset, so the window could offer a language the running model
  cannot read.

## [0.12.2] - 2026-09-04

### Added

- **Your coding agent is told to ask out loud.** The MCP handshake now says you
  are working eyes-free, so the agent asks with `ask_user` instead of printing a
  question you would have to read. An agent reads this once, when its session
  starts, so restart the agent to pick it up.

### Fixed

- **A question no longer cuts off the status it was spoken with.** `ask_user`
  waits for speech already playing to finish before it speaks, and it arms the
  microphone before that wait, so the hotkey while Banshee talks holds to answer
  instead of opening a dictation session that leaves the question unasked.
- **Nothing claims a microphone Banshee has not opened.** The window, the tray
  and the bar said "No microphone" on a first run, before the models were
  fetched, on a machine with three of them. Every surface now says the stream is
  not open, and `banshee status` fails that check and names the download instead
  of printing a tick beside it.

## [0.12.1] - 2026-09-03

### Added

- **Homebrew installs the window.** `brew install --cask
  yamanahlawat/banshee/banshee` puts `Banshee.app` in `/Applications` and the
  `banshee` and `banshee-mcp-shim` commands on your `PATH`, one copy of
  everything. Until Banshee is notarised, run
  `xattr -dr com.apple.quarantine /Applications/Banshee.app` once after it, or
  macOS refuses the app and kills the command silently. The cask refuses to
  install beside the `banshee` formula, which is the same daemon without the
  window. The README says which path fits which use, and how to uninstall each.

## [0.12.0] - 2026-09-02

### Added

- **Banshee has a window.** Choose `Open Banshee` from the menu bar to set up
  dictation, copy what you said, and change any setting. It sets Banshee up on
  its own, models and all, so nothing in it sends you to a terminal. Everything
  it does, the CLI still does. It needs no new permission and holds nothing the
  daemon needs: quit it and dictation carries on.
- **The window sets Banshee up from nothing.** A first run offers the three
  speech models with what each costs to download, fetches them, says which file
  is arriving and how far it has come, and restarts the daemon when the files
  need it. Every fix it names is a button.
- **The window lists every voice Banshee can name.** The Voice panel shows all
  of them, marks the ones this machine does not hold, and fetches the one you
  choose. `banshee voices` still prints only the voices that work today.
- **A setting waiting on a model applies when the model arrives.** Choosing a
  speech model or a voice before its file is downloaded no longer waits for a
  restart: a running daemon asks the setting again as the download finishes.
  The first setup on a new machine still ends with a restart, because a daemon
  that started with no models has no pipeline to change, and the window offers
  the restart when it is needed.
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

### Changed

- **The daemon holds 169 MB less memory.** The pronunciation lexicon kept every
  word twice, once per casing, and the speech engine pre-packed its weights for
  a speed gain that did not show up in a measurement. An idle daemon with the
  same models went from 1403 MB to 1234 MB. What you hear is unchanged, apart
  from the casing fix below.

### Fixed

- **A flag sent with the wrong type is refused.** `interrupt` on `speak`,
  `dictate` on `record_start` and `persist` on `configure` used to read a
  string such as `"true"` as `false` and carry on. They now answer `-32602`
  naming the flag, as `disconnect` already did.
- **A word whose two casings sound different keeps both.** The lexicon used to
  swap them, so `Polish` was said as `polish` and the other way round. 361 words
  in the table have such a pair.
- **The desktop window ships in a release.** A release now carries
  `Banshee.app.tar.gz` with all four binaries in it, so the window no longer
  needs a source build. Install it with `curl … | tar -xzf - -C /Applications`.
  It is signed but not notarised, so fetch it with `curl` rather than a browser,
  or clear the quarantine flag once with `xattr -dr com.apple.quarantine
  /Applications/Banshee.app`.
- **Banshee installs to `/Applications`.** That is the folder Finder's
  `Applications` favourite opens, and the one System Settings offers when a
  permission pane asks which app to add. If you installed an earlier build,
  delete the bundle left at `~/Applications/Banshee.app` by hand, then run
  `banshee connect <agent>` once per agent: a registration holds the old
  absolute path until something rewrites it.
- **The window connects Claude Code again.** `Connect` failed with `No such
  file or directory` and applied none of its changes, because the daemon runs
  with a supervisor's four-directory `PATH` and looked for `claude` there.
  Banshee now records where it found each agent's CLI and runs that path. The
  other agents were never affected: their setup writes files and spawns nothing.
- **Banshee asks for the permission it needs, and asks for one fewer.** The
  daemon asks macOS for Accessibility as it starts, so Banshee appears in the
  Privacy & Security list with a switch to turn on. It no longer asks for Input
  Monitoring: the hotkey works without it, measured, and that pane never listed
  Banshee to switch on. `banshee status` keeps one line about key presses as a
  note, instead of reporting a grant that was never made.
- **A download that cannot reach its host no longer hangs for ever.** The
  fetch had no timeout of any kind, so one unreachable server held the whole
  run and stranded every file behind it. A connect now gives up after ten
  seconds and a stall between chunks after thirty, and the run carries on to
  the remaining files and says which one failed.
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

[Unreleased]: https://github.com/yamanahlawat/banshee/compare/v0.15.1...HEAD
[0.15.1]: https://github.com/yamanahlawat/banshee/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/yamanahlawat/banshee/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/yamanahlawat/banshee/compare/v0.13.1...v0.14.0
[0.13.1]: https://github.com/yamanahlawat/banshee/compare/v0.13.0...v0.13.1
[0.13.0]: https://github.com/yamanahlawat/banshee/compare/v0.12.2...v0.13.0
[0.12.2]: https://github.com/yamanahlawat/banshee/compare/v0.12.1...v0.12.2
[0.12.1]: https://github.com/yamanahlawat/banshee/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/yamanahlawat/banshee/compare/v0.11.1...v0.12.0
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
