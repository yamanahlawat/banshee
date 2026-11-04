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
  `banshee tray`, about a minute later, brought both up. ~~Seen on 2026-10-31: launchd kept
  respawning the login items while the bundle was being replaced. That is the likely cause,
  not proven.~~ launchd's log shows `Launch Constraint Violation` on the first launch after
  `banshee start` re-registers a new build, and macOS rebuilt its record about 10 s later. A
  second start then worked. That happened on 3 of 3 installs over a running build, and a
  re-register of a build that had already run worked 4 of 4. That the record pins something
  per build, because the self-signed build has no Team ID, is inferred and not measured.
  `banshee start` and `banshee tray` now retry once. Still open: a cask or tarball upgrade
  followed by a login, with no `banshee start`, is not measured and not covered. The window
  reopens the tray through its own unregister and register (`keep_the_tray_running` in
  `banshee-app/src/login_items.rs`), and that waits for nothing.
- The window's Start button runs `banshee start` and shows nothing while it waits, up to about
  50 s after a refused launch.
- The first sound from a Bluetooth headset arrives about 300 ms after the microphone opens.
  That is measured on one headset only. Other headsets are not checked.
- A restricted microphone (a managed Mac) still offers "Open System Settings", which cannot
  lift the restriction.
- On macOS 13 the daemon's microphone permission read does not change while it runs, and
  the window offers no restart after a microphone grant, so `banshee start` applies it. Not
  measured on a macOS 13 machine.
- The live microphone read is measured on macOS 26 only; macOS 14 and 15 are not measured.
- With `audio.keep_open = true`, or during one long open such as an `ask_user` answer, a
  microphone refusal shows only at the next open or status read. Until then recordings come
  back empty.
- An Accessibility grant revoked while Banshee runs reaches the tray, the window and the spoken
  line only at the next state change. That is inferred from the code and not measured.
- The voice files and the remote key are not blockers, so no surface shows them.
- The tray's socket `call` has no timeout. That is inferred from the code and not measured.
- A `banshee start` that hangs keeps the tray's first row on `Starting Banshee`.
- When the tray brings the window forward with a panel open, focus lands on the button that
  opened the panel. The problem sits above it.
- A model file removed while Banshee is idle shows in the tray at the next use.
- Every recording pipeline fault shares one id. When one fault turns into another without
  clearing, Banshee does not say the new one.
- Waybar shows no problem until the Linux capsule item lands.
- An agent's question loads the listening model only after it has played, so the first answer
  after an idle unload waits for the load. Dictation overlaps its load with the press. The
  wait is not measured.
- Each use walks every input device before it opens one. The added press latency is not
  measured.
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
- An X11 hotkey listener that stopped at start (`start_global_hotkey` in
  `bansheed/src/hotkey.rs`) takes a rebind and still hears nothing. A restart is the only
  way back.
- A model download cannot be cancelled once it starts.
- A daemon that started without the voice detection model still needs `banshee start` before it
  records.
- No upper bound is measured for `stt.endpoint_silence_ms`, `models.unload_after`,
  `tts.remote.sample_rate` or the length of `stt.vocabulary`. The settings table leaves three of
  them open. It stops `tts.remote.sample_rate` at the `u32` maximum, 4294967295, the limit
  `config set` enforces.
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
- The remote provider code lives inside the daemon. Moving it into a crate of its own is worth
  doing once a second provider adapter exists. Today the remote listener and speaker both use
  the OpenAI-compatible client.
- `banshee setup` downloads the local voice even when speech is remote, unless `tts.fallback` is `none`.
- The remote speaker gets the same spoken text as the local voices, with versions, paths, units,
  PR numbers and hyphenated names written out as words. How a remote voice server reads that text
  is not measured.
- Each remote request builds a runtime and opens a connection of its own, so every spoken line
  and every dictation pays a TLS handshake. The client keeps no idle connection
  (`pool_max_idle_per_host(0)`), because nothing runs between requests to see a server close one.
  One long-lived runtime per client would allow a pool. The latency is not measured.
- Each remote dictation copies the whole recording and starts a thread for the request.
- The listener decides a switch on both sides of the command queue. It could decide alone from
  the saved config, but only once a `persist: false` write of `stt.remote.model` is refused,
  since that write never reaches the saved config.

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
- One state is called "pending" in some names and "waiting" in others: `DaemonState::pending`
  beside `Outcome.waiting`, and `markPending` beside the `$waiting` store. The `pending` status
  field is part of the protocol and keeps its name.
- No `rust-version` is declared.
- `rdev` pulls crates the compiler reports as future-incompatible, and has no newer release.
- RUSTSEC-2024-0429 flags glib 0.18, which comes with Tauri's GTK 3 stack. It affects the Linux
  window only. No Banshee or stack code calls `VariantStrIter` (checked with grep). It moves with
  Tauri's GTK 4 work, tauri-apps/tauri#12562.
