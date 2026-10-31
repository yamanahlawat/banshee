# Backlog

What is missing or wrong today. Nothing here is ordered. `ROADMAP.md` holds what lands next.

## macOS

- SentinelOne flags every new Banshee build on a managed Mac as `persistence_deception`: the
  daemon, the window, and even `tar` unpacking the app. Each install or upgrade raises it again.
  The app is self-signed and not notarized.
- Dictating replaces a copied image for good. Banshee saves only clipboard text to put back: with
  a 2x2 image copied, the clipboard held no image and only the dictated words 1.5 s later. Files
  and rich text are likely the same; not tested. The Linux path is separate and not tested.
- An upgrade can leave both login items unable to start. On 2026-10-31, `make install` over an
  installed build re-registered them, and launchd then failed every spawn with exit 78
  (`EX_CONFIG`), 6 tries for the daemon and 13 for the tray. No Banshee code exits 78. The
  binaries ran from a terminal and the bundle signature verified. A second `banshee start` and
  `banshee tray`, about a minute later, brought both up. The cause is not known.
- Nothing has tested a logout and login with the login items. CI registers, restarts, stops
  and removes them on a runner, but cannot log in again.

## Listening and speaking

- Any process running as the user can start a recording through the daemon's socket, and can
  read every transcript in `banshee.db`. Nothing shows which client opened the microphone.
- The corrector for words to know has its threshold (0.10, Soundex weight 0.5) measured on
  synthetic speech and plain-English references only. Real accents and jargon are not measured.
- Live words on Linux wait for the capsule. The macOS chip shows them today.
- An idea for answers: use the unusual words in an agent's question as vocabulary for that
  answer.
- A listening model download that fails for a reason other than a licence reaches the window
  with no reason.
- A second download that finishes while a model loads queues a duplicate Reload. It costs a
  redundant load.
- Choosing a remote listener while the local model file is missing leaves listening broken.
- The answer path builds its resampler when listening starts.
- The aarch64 Linux release targets plain armv8-a. Whether a dotprod floor is faster is not
  measured.
- Banshee downloads the Silero VAD model from the `master` branch with no pinned commit and no
  hash check (`banshee-common/src/lib.rs:437`), while the Kokoro files are pinned. A download
  also follows a redirect to any scheme. The file
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
- Some readings the voices get are untested by the round trip: `127.0.0.1` reads "one hundred
  twenty seven point zero point zero point one", `I/O` and `N/A` read with "slash", and `€1.50`
  reads "€one point five zero". Change each only with round-trip evidence.
- The reading rules for units, numbers and paths each carry their own boundary checks, and those
  checks already differ. Before more rules land, classify each whitespace-delimited token once
  (URL, path, number, unit, word), drop the per-rule checks, and prove it with the round trip.

## Remote listener and speaker

- Both remote keys live in a file in `~/.banshee`, not in the Keychain.
- The speaker takes the OpenAI `/audio/speech` shape only.
- `banshee setup` downloads the local voice even when speech is remote, unless `tts.fallback` is `none`.
- The remote speaker gets the same spoken text as the local voices, with versions, paths, units,
  PR numbers and hyphenated names written out as words. How a remote voice server reads that text
  is not measured.

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
