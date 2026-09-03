# Backlog

What is missing or wrong today, recorded so it is not lost. Nothing here is a promise, and
nothing here is ordered. `ROADMAP.md` holds what lands next.

## The daemon

- The hotkey fires a dictation while the window captures a new one. The daemon binds the key
  at OS level, so window focus does not stop it, and no protocol method suspends it. A fix
  needs a suspend with a timeout, so a window that dies does not leave the hotkey dead.
- `english_only` reports the configured preset, not the model the daemon loaded. The window
  reads that field to enable its Language picker, so it can offer a language the running
  model cannot transcribe, and nothing says the two differ.
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

- `banshee status` prints a check mark beside "daemon has the microphone: No microphone" when
  no device is open. Seen on a fresh 0.12.0 install before the models were fetched. The line
  should fail, or name the device it has.
- `banshee status` one second after `banshee start` reports "the daemon is not running": the
  socket is not bound yet while Whisper loads. Measured on a fresh 0.12.0 install; the same
  command a few seconds later reports running. `start` should wait for the socket, or `status`
  should say the daemon is starting.
- A release published by the release workflow raises no event another workflow can see. The
  bundle workflow's `release: published` trigger never fired for 0.12.0, and the bundle came
  from a hand dispatch. #84 chains the bundle after announce; until it merges, a release needs
  the dispatch by hand.

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

## Testing

- No WebDriver acceptance layer, so nothing exercises the Rust socket and the Svelte face
  together.
