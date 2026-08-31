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
- `daemon.log` carries no timestamps, so no interval in it can be measured.
- Past eight queued utterances the oldest is dropped silently, and `speak` still answers with
  an id for it.

## The window

- The download reports a percent, and no bytes, rate or time. A reader cannot tell a stalled
  download from a slow one.
- A vocabulary word removed by mistake cannot be put back except by typing it again.
- The blocker calls it `Speech model` and the panel calls it `Transcription`. One thing needs
  one name.
- No control on the home screen has a resting affordance, so what can be pressed is learned
  rather than seen.

## Testing

- The accessibility gate does not cover the Record panel, which holds the only destructive
  action in the window.
- No WebDriver acceptance layer, so nothing exercises the Rust socket and the Svelte face
  together.
