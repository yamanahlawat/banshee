# Backlog

What is missing or wrong today, recorded so it is not lost. Nothing here is a promise, and
nothing here is ordered. `ROADMAP.md` holds what lands next.

## The daemon

- The hotkey fires a dictation while the window captures a new one. The daemon binds the key
  at OS level, so window focus does not stop it, and no protocol method suspends it. A fix
  needs a suspend with a timeout, so a window that dies does not leave the hotkey dead.
- Reading the status starts the daemon as a side effect. The restart poll asks twelve times,
  so a daemon slow to load can be kickstarted more than once.
- No protocol method cancels a download. A person on a metered connection can start 862 MB
  and has no way to stop it from the window.
- On Linux the daemon spawns `wtype` or `ydotool` by bare name, while `banshee status`
  reports them from the login shell's `PATH`. A supervised daemon holds a smaller `PATH`, so
  the checklist can name a typer the daemon cannot run. `connect` resolves an agent CLI and
  hands the child the `PATH` it searched; dictation does neither. Not reproduced: this
  machine is macOS.
- `daemon.log` carries no timestamps, so no interval in it can be measured.
- Past eight queued utterances the oldest is dropped silently, and `speak` still answers with
  an id for it.

- `banshee status` one second after `banshee start` reports "the daemon is not running": the
  socket is not bound yet while Whisper loads. Measured on a fresh 0.12.0 install; the same
  command a few seconds later reports running. `start` should wait for the socket, or `status`
  should say the daemon is starting.
- A release published by the release workflow raises no event another workflow can see. The
  bundle workflow's `release: published` trigger never fired for 0.12.0, and the bundle came
  from a hand dispatch. #84 chains the bundle after announce; until it merges, a release needs
  the dispatch by hand.
- The remote speaker takes one shape only: an OpenAI-compatible `/audio/speech` endpoint.
  ElevenLabs and any other API shape need a backend of their own.
- The remote speaker's voice is a plain field the person fills in. The endpoint has no call
  that lists voices, so nothing can offer the names the server accepts. A fetched list stays
  out on purpose: `/v1/audio/voices` exists on two servers and neither OpenAI nor Groq, and a
  server that refuses a voice names the ones it accepts, which the failure reason already
  shows. Type a wrong voice once and read the answer.
- The daemon chooses the speaker once, when it starts. No call picks a speaker for one
  utterance, so every reply goes to the same one.
- An utterance that fails after its first chunk ends where it stopped. The samples already
  played are words the listener has heard, so no fallback starts the reply again.
- The remote speaker's 15 s bound covers the wait for headers as well as each read, since the
  blocking client keeps one timeout. A server that renders a whole reply before its first byte
  costs the full 15 s.
- A stop does not drop the in-flight request. The worker ends at the next byte or the bound,
  whichever comes first.
- `speed` goes out on every speech request, since it is part of the original OpenAI schema. A
  server that refuses the field refuses every utterance, not only the ones where speed changed.
- Both remote keys live in `~/.banshee/credentials.toml`, which only the owner can read. The
  macOS Keychain holds neither, so a key stays a file on disk.
- The status reply says what the config asked for, not what `select_backend` built. A speaker
  that refuses to start still reports its host on every surface, so each surface guards the
  cases it knows. A reply that named the built backend would remove those guards.
- `banshee setup` downloads the Kokoro model whatever `tts.provider` says, while
  `banshee status` skips the Kokoro check under a remote speaker. The two disagree about what
  a remote-speaker machine needs.
- A remote `base_url` is parsed twice: the config deserializer proves it has a host, and
  `host_of` parses it again with an empty-host path the config can no longer reach. One
  `RemoteUrl` newtype with an infallible `host()` would remove the second parse and the dead
  path; it touches config, both remote backends, status and the CLI.
- `compositor` imports `Change`, `render`, `apply_all` and `confirm` from `connect`, so a key
  binding depends on the agent connector. The plan-show-apply machinery deserves a module of
  its own with `connect` and `compositor` as two users; the move touches `api.rs`, the connect
  tests and the app crate's imports.

## The window

- A segmented control reports `aria-checked` from the daemon's answer, so a
  screen-reader user arrows to a cell and hears nothing become checked until the
  round trip lands. The tab stop already moves at once.
- `Foot.svelte` and `Segmented.svelte` each implement the roving tab stop over
  the same `arrowStep`, in two shapes, and only one carries the lag fix.
- `history.ts` holds its clear generation in a module variable beside the store
  rather than in the store, so a subscriber cannot see it and `readNewest`
  guards the same hazard a second way.
- `App.svelte` holds two copies of the focus-return idiom (`await tick()` then
  focus by id) that belongs beside `arrowStep` in `lib/keys.ts`.
- The download reports a percent, and no bytes, rate or time. A reader cannot tell a stalled
  download from a slow one.
- A vocabulary word removed by mistake cannot be put back except by typing it again.
- The blocker calls it `Speech model` and the panel calls it `Transcription`. One thing needs
  one name.
- No control on the home screen has a resting affordance, so what can be pressed is learned
  rather than seen.
- Speech has no panel of its own. The Voice panel holds the local voice and the remote
  speaker, so both sets of controls grow inside one screen.
- The remote key row is duplicated whole between the Microphone panel and the Voice panel:
  about 40 lines of script and markup, plus the `.held` style rule, in each of them. Only the
  setting name differs.

## banshee tell

- A written reply from the agent never reaches a person who used the hotkey. `tell::run`
  answers with the reply, `cli::tell` prints it, and the hotkey arm drops it. The MCP shim
  tells every agent to "reserve written output for what must be read on screen, such as
  code, file paths, commands, URLs, and lists", which assumes a screen the written half
  lands on. Started from the key, there is none. Speaking the whole reply was rejected:
  ten steps is unbearable aloud. Keeping it in the history was rejected: the window is too
  small to read it. The agreed start is that the user asks for it, by saying "show me", and
  Banshee reopens that thread in a terminal through Omarchy's own
  `omarchy agent prompt "<text>"`. Nothing opens unbidden, and no length decides anything,
  because no length has been measured. Left here rather than designed further, so real use
  can say what it should be.
- Nothing tells a person by ear that their words went to the agent and not into the
  window. Both routes end on the same record-stop cue. The spec's step 3 asks for a cue
  when the words land, and a three-note one was built and then removed: it played right
  after the record-stop cue, so one command sounded five notes. A swap was offered, the
  new cue instead of the stop cue rather than after it, and refused. The decision is to
  add no cue at all until the right interface is settled, and to keep only the cues that
  already existed. A failed run still sounds the error cue, and `banshee status` names the
  reason.
- Banshee itself says nothing on the hotkey path. It spoke the agent's scope, a failure
  reason and any warnings, and all three were removed after real use: the reasons are
  machine text, and a voice reading `opencode exited exit status: 1` is worse than a
  beep. The scope difference is written down instead. The cost is that a failed run tells
  you that it failed and not why, until you ask `banshee status`.
- OpenCode runs unscoped, and nothing narrower works. It auto-rejects a write outside its
  run directory, and the `opencode.json` permission block that should allow one makes a
  headless run hang instead. Measured twice, past 100 seconds each time. Only `--auto`
  works, and it allows any edit anywhere. The cause of the hang is not known, and it
  deserves a report upstream.
- A timed-out run leaks one thread and one descriptor inside the daemon. The threads
  draining the agent's pipes are never joined, because a surviving descendant can hold
  those pipes open for ever. In the CLI the cost ends with the process. In the daemon it
  accumulates until a restart.
- `banshee bind` writes binds the running daemon may not understand. After an upgrade with
  no daemon restart, the tell key records to the mailbox and nothing says so.
- Nothing checks the derived tell chord against bindings the user already has. `strays`
  only finds `banshee record` lines in the file Banshee writes.
- A running tell cannot be stopped. `banshee tell --undo` takes the same lock, correctly,
  so a person waits out `tell.run_timeout_min` while the agent edits.
- A restore does not put back directory permissions. `create_dir_all` applies the umask, so
  a folder that was `0700` comes back `0755`.
- A restore writes no snapshot of what it replaces, so a mistaken `--undo` after a day of
  edits by hand has nothing to return to.
- `undo` prints its failure through `Debug`, so a total failure reads
  `Error: Rejected("...")` rather than the sentence it was written as. Every command in the
  binary reads that way; one `Display` wrapper in `main` would fix all of them.
- Nobody knows when a changed `tell` setting takes effect. `banshee config set` reports
  every `tell` key as needing a restart, while `settings::configure` hands the daemon the
  whole new config. The two disagree and neither was measured, so the documentation makes
  no claim either way.
- A transcribed `?` or `,` defeats `start over` and `show me`. The match strips a trailing
  `.` and `!` only, so "Show me?" runs headlessly as an agent command and costs a run.
  Whisper emits a question mark on a short rising phrase, so this is common rather than
  rare. The match mirrors `is_reset`, which is the consistency that argued against
  widening it, and no wider set has been measured against real transcripts.
- The six state marks are drawn in three independent places: the PNGs in
  `bansheed/assets/tray/`, the generator `scripts/make-state-tiles.sh`, and the Svelte
  component `banshee-app/ui/src/marks/Mark.svelte`. All three carry the same reasoning
  in their own comments. Changing the tray left the window showing the old listening bar
  until somebody noticed, which is the first time the duplication actually cost anything.
- `speaking`'s arcs sit five units tighter in the menu bar than in the window, because
  36 pixels cannot hold the pair at the window's spacing. The two are deliberately
  different and nothing records that outside this line.
- The window's pending message reads the transcribing flag alone, so it stays silent
  while an agent started by the tell key runs. The icon shows busy; the words do not.
- No real speech has ever reached the tell arm inside a test. The path is proven on a real
  machine and by a state-machine round trip, and by nothing in between.

## Testing

- No WebDriver acceptance layer, so nothing exercises the Rust socket and the Svelte face
  together.
- `banshee status` has no test harness for the checklist. Each check prints to stdout and
  `run` probes a live daemon, so the speaker's voice check and the espeak gate carry no test.
- Three loopback HTTP fixtures live in three test modules: the listener's reads a body by
  `Content-Length`, the speaker's writes chunked pieces with gaps, and the probe's answers a
  bare `GET`. The bind, accept and read-until-blank-line steps are the same in all three, so
  a shared fixture would hold them once.
- A `resolve.alias` in `vite.config.ts` that points at `src/mocks` passes both mock guards.
  The lint rules read the specifier a module writes, and the bundle check reads the literals
  a mock holds, so an aliased import of `not-running.json` ships unseen. Every other mock
  file holds a guarded literal, and `{"running": false}` is harmless, so the route needs a
  visible config change to be worth anything. Left open on purpose.
