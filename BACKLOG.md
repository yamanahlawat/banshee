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
- `daemon.always_on` is parsed from `config.toml` and read nowhere, so the key does nothing and
  the configuration page does not list it. Either a consumer or a removal, with the parse kept
  so an old file still loads.
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
