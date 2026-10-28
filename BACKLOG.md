# Backlog

What is missing or wrong today. Nothing here is ordered. `ROADMAP.md` holds what lands next.

## macOS

- SentinelOne flags every new Banshee build on a managed Mac as `persistence_deception`: the
  daemon, the window, and even `tar` unpacking the app. Each install or upgrade raises it again.
  The app is self-signed and not notarized.
- Banshee writes its own launchd plists and calls `launchctl`, which is how persistence malware
  looks to an EDR. Apple's `SMAppService` is the sanctioned route, and it accepts a self-signed
  app.

## Listening and speaking

- Any process running as the user can start a recording through the daemon's socket, and can
  read every transcript in `banshee.db`. Nothing shows which client opened the microphone.
- Transcription runs on whisper.cpp only, after the speaker stops. No streaming model is wired
  in, so text arrives only after the silence and a full pass.
- In a crowded room, other people's voices end up in an answer. Nothing separates the user's
  voice from the room.
- The hotkey still fires a dictation while the window is capturing a new hotkey.
- A model download cannot be cancelled once it starts.
- On Linux, a microphone chosen by name can open the `hw:` device that offers no float format.
- An agent installed under `nvm` can stay invisible to `banshee connect`.

## Remote listener and speaker

- A remote URL can be plain `http://` to any host, so audio and text can cross a network
  unencrypted.
- Both remote keys live in a file in `~/.banshee`, not in the Keychain.
- A server that refuses the `speed` field refuses every reply at that speed.
- The speaker takes the OpenAI `/audio/speech` shape only.
- `banshee setup` downloads Kokoro even when speech is remote.

## The window

- The download shows no rate or time left, so a stalled download looks like a slow one.
- A vocabulary word removed by mistake has to be typed again.
- The same model is called `Speech model` in one place and `Model` in another.
- Buttons show they can be pressed only on hover.
- Speech settings have no panel of their own.
- A screen reader hears a segmented control change only after the daemon answers.

## Build and tests

- Nothing tests the window and the daemon together.
- No `rust-version` is declared.
- `rdev` pulls crates the compiler reports as future-incompatible, and has no newer release.
