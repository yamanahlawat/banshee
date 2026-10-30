# Backlog

What is missing or wrong today. Nothing here is ordered. `ROADMAP.md` holds what lands next.

## macOS

- SentinelOne flags every new Banshee build on a managed Mac as `persistence_deception`: the
  daemon, the window, and even `tar` unpacking the app. Each install or upgrade raises it again.
  The app is self-signed and not notarized.
- Dictating replaces a copied image for good. Banshee saves only clipboard text to put back: with
  a 2x2 image copied, the clipboard held no image and only the dictated words 1.5 s later. Files
  and rich text are likely the same; not tested. The Linux path is separate and not tested.
- Banshee writes its own launchd plists and calls `launchctl`, which is how persistence malware
  looks to an EDR. Apple's `SMAppService` is the sanctioned route, and it accepts a self-signed
  app.

## Listening and speaking

- Any process running as the user can start a recording through the daemon's socket, and can
  read every transcript in `banshee.db`. Nothing shows which client opened the microphone.
- Transcription runs on whisper.cpp only, after the speaker stops. No streaming model is wired
  in, so text arrives only after the silence and a full pass.
- Banshee downloads the Silero VAD model from the `master` branch with no pinned commit and no
  hash check (`banshee-common/src/lib.rs:437`), while the Kokoro files are pinned. The file
  matched the pinned hash on 2026-10-30, but it can change under every install.
- Dictation types the words exactly as heard. Nothing drops fillers such as "um" or applies a
  spoken edit such as "make that a list". An idea to test: a small local model as a cleanup
  step after transcription. It could also change what the person meant.
- In a crowded room, other people's voices end up in an answer. Nothing separates the user's
  voice from the room.
- The hotkey still fires a dictation while the window is capturing a new hotkey.
- A model download cannot be cancelled once it starts.
- On Linux, a microphone chosen by name can open the `hw:` device that offers no float format.
- An agent installed under `nvm` can stay invisible to `banshee connect`.

## Remote listener and speaker

- Both remote keys live in a file in `~/.banshee`, not in the Keychain.
- A server that refuses the `speed` field refuses every reply at that speed.
- The speaker takes the OpenAI `/audio/speech` shape only.
- `banshee setup` downloads Kokoro even when speech is remote.

## The window

- Sensitivity runs backwards. "High" writes `stt.vad_threshold` 0.85, and the daemon counts
  audio as speech only above the threshold (`bansheed/src/hotkey.rs:390`), so High hears the
  least. `docs/configuration.md` says higher is stricter, which matches the daemon. The window's
  labels are the wrong side (`banshee-app/ui/src/jobs/MicrophonePanel.svelte:40-44`).
- The download shows no rate or time left, so a stalled download looks like a slow one.
- A vocabulary word removed by mistake has to be typed again.
- The same model is called `Speech model` in one place and `Model` in another.
- Buttons show they can be pressed only on hover.
- Speech settings have no panel of their own.
- A screen reader hears a segmented control change only after the daemon answers.

## Build and tests

- `remote_probe`'s `a_closed_port_names_the_host` binds a port, frees it, then probes it. A
  test running in parallel can bind that freed port first, so the probe reaches the other
  test's server: both tests then fail (`remote_probe.rs:78` and `:118`). Seen once in a full
  Linux run on 2026-10-31; five isolated and two full reruns passed.
- Nothing tests the window and the daemon together.
- No `rust-version` is declared.
- `rdev` pulls crates the compiler reports as future-incompatible, and has no newer release.
