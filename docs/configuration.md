# Configuration

Every setting Banshee reads. Nothing here is required: the defaults work. To
override one, create `~/.banshee/config.toml`. The defaults:

```toml
[daemon]
save_history = true    # keep transcriptions in ~/.banshee/banshee.db

[stt]
preset = "balanced"      # fast | balanced | quality (see below)
vad_threshold = 0.5      # 0.0 to 1.0; higher means stricter speech detection
vocabulary = ["banshee"] # words Whisper keeps mangling, e.g. ["clippy", "tokio"]
language = "en"          # a Whisper code, or "auto" to detect it
translate = false        # true answers in English whatever you spoke
endpoint_silence_ms = 2500  # trailing silence that ends a spoken answer

[tts]
voice = "af_sky"       # any voice from the Kokoro voices directory
speed = 1.2            # playback speed multiplier
fallback = "system"    # system = use `say` when Kokoro is unavailable | none

[audio]
input_device = "default"  # "default" = follow the OS; otherwise match a device name
hotkey = "RightOption" # F1-F12, a lone modifier, or a chord like "Ctrl+Alt+D"
hotkey_mode = "hold"   # hold = record while the hotkey is down | toggle = tap to start, tap to stop
barge_in = "stop"      # stop = the record hotkey cuts off whatever Banshee is saying | none

[audio.cues]
enabled = true         # tones on record start/stop, success, and errors
```

`input_device` is a case-insensitive substring of the microphone name, so
`"yeti"` matches `Blue Yeti Stereo Microphone`. An exact name wins over a longer
name that contains it, so a `Yeti` next to a `Blue Yeti Pro` opens its own
device. `banshee devices` shows the names to choose from:

```
$ banshee devices
  Blue Yeti               system default, in use
  BlackHole 2ch
  MacBook Pro Microphone
```

**`"default"` follows the OS while Banshee runs.** Connect a headset, macOS makes
it the system default, and Banshee moves capture to it within about five seconds.
A device you name is not treated this way: Banshee opens the device you named,
and the system default never takes its place while it is present.

**A microphone that disappears does not stop dictation.** Unplug the headset you
named and Banshee records from the system default instead, so a press still
works. It says which microphone it moved to, and it says which one it is still
waiting for: the tray, `banshee status` and `banshee watch --waybar` all show
`MacBook Pro Microphone (waiting for "yeti")`. Reconnect the headset and Banshee
takes it back within about five seconds. Nothing needs a restart.

Banshee never picks a different microphone in silence. If it cannot open any
device at all, it says so and refuses to record rather than returning silence.

## Pronouncing unknown words

Install `espeak-ng` and Banshee pronounces unfamiliar words (tech jargon, proper
nouns) instead of spelling them out letter by letter. On macOS that is
`brew install espeak-ng`; `banshee status` prints the command for your system.

## Choosing a voice

`banshee voices` lists the voices on disk and marks the one the daemon loaded:

```
$ banshee voices
  af_heart
  af_sky    in use
  am_adam
  am_santa

Speak with one by: banshee config set tts.voice "<name>"
```

It lists only what is downloaded, so every name it prints works today. Nothing
is marked in use when Kokoro did not load: the system fallback speaks in
whatever voice macOS is set to, which Banshee did not choose.

## Changing a setting without an editor

`banshee config set` writes one key and keeps your comments and layout:

```bash
banshee config set audio.hotkey RightOption
banshee config set stt.vad_threshold 0.7
banshee config set stt.vocabulary '["tokio", "clippy"]'
banshee config set audio.cues.enabled false
```

The key is the section and the field, as they appear in the file. A number, a
`true`, or a `[list]` is read as that type; anything else is read as text.
Quote twice to force text, as in `banshee config set audio.input_device '"12"'`.
A value the field does not accept is refused, and the message lists the legal
ones. This works whether or not the daemon is running.

Most settings take effect at once: `stt.vad_threshold`, `stt.vocabulary`,
`stt.preset`, `stt.language`, `stt.translate`, `audio.input_device`,
`audio.barge_in`, `audio.cues.enabled`, `tts.voice`, `tts.speed` and
`daemon.save_history`. Four are read when the daemon starts, so the command
tells you to restart: `audio.hotkey`, `audio.hotkey_mode`,
`stt.endpoint_silence_ms` and `tts.fallback`.

A live setting whose model is not downloaded yet waits for the file. Once
`banshee setup` fetches it, a running daemon applies the setting as the
download finishes. A daemon that started without its models is a different
case: it has no pipeline to change, so the first setup on a new machine ends
with a restart.

`endpoint_silence_ms` is how long you can go quiet mid-answer before Banshee
decides you're done. Lower it if replies feel sluggish, raise it if you keep
getting cut off.

The `preset` picks which Whisper model Banshee uses:

| Preset     | Model                          | Trade-off                                  |
| ---------- | ------------------------------ | ------------------------------------------ |
| `fast`     | `ggml-base.en.bin`             | Fastest and lightest, English only         |
| `balanced` | `ggml-large-v3-turbo-q5_0.bin` | The default; accurate and reasonably quick |
| `quality`  | `ggml-large-v3-q5_0.bin`       | Most accurate, heaviest                    |

For `voice`, any file in the
[Kokoro voices directory](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/tree/main/voices)
works, e.g. `af_bella`, `am_michael`, or `bf_emma` (the prefix is
accent/gender: `a`merican/`b`ritish, `f`emale/`m`ale). After changing the
`preset` or `voice`, run `banshee setup` to fetch the new file; the daemon
starts using it as the download finishes, with no restart. The exception is a
daemon that started with no models at all: it has no pipeline to change, so the
first setup on a new machine still ends with a restart. The window's Voice panel
lists every voice Banshee can name and fetches the one you pick.

## The hotkey

Hold the key to record and let go to stop. Any recording that runs past 120
seconds, held or toggled, is ended by the watchdog, which returns the
microphone and still transcribes what it heard.

The key is rebindable: `banshee config set audio.hotkey F6`, then
`banshee start`. Legal values are an F-key (`F1`–`F12`), a modifier alone
(`RightOption`, `LeftOption`, `LeftControl`, `LeftCommand`, plus
`RightCommand` and `Fn` on macOS), or modifiers and a key, as in `Ctrl+Alt+D`.
A modifier bound alone still works as a modifier: `RightOption+E` types é, and
banshee discards the accidental recording instead of transcribing it.

