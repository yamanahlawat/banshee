# Configuration

Every setting Banshee reads. Nothing here is required: the defaults work. To
override one, create `~/.banshee/config.toml`. The defaults:

```toml
[daemon]
save_history = true    # keep transcriptions in ~/.banshee/banshee.db

[stt]
provider = "local"       # local | remote; see "A remote listener" below
preset = "balanced"      # fast | balanced | quality (see below)
vad_threshold = 0.5      # 0.0 to 1.0; higher means stricter speech detection
vocabulary = ["banshee"] # words Whisper keeps mangling, e.g. ["clippy", "tokio"]
language = "en"          # a Whisper code, or "auto" to detect it
translate = false        # true answers in English whatever you spoke
endpoint_silence_ms = 2500  # trailing silence that ends a spoken answer

[stt.remote]                           # read when provider = "remote"
base_url = "https://api.openai.com/v1" # an OpenAI-compatible server's /v1 root
model = "whisper-1"                    # the model that server names

[tts]
provider = "local"     # local | remote; see "A remote voice" below
voice = "af_sky"       # any voice from the Kokoro voices directory
speed = 1.2            # playback speed multiplier
fallback = "system"    # system = use the OS voice (say, or espeak-ng on Linux) | none

[tts.remote]                           # read when provider = "remote"
base_url = "https://api.openai.com/v1" # an OpenAI-compatible server's /v1 root
model = "tts-1"                        # the model that server names
voice = ""                             # a voice that server names; required
instructions = ""                      # tone and delivery, for a model that reads it
response_format = "wav"                # wav | pcm; what the server is asked to send
# sample_rate = 22050                  # unset; only pcm needs it

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
`daemon.save_history`. The rest are read when the daemon starts, so the command
tells you to restart. Among them: `audio.hotkey`, `audio.hotkey_mode`,
`stt.endpoint_silence_ms`, `stt.provider`, the `[stt.remote]` keys,
`tts.fallback`, `tts.provider` and the `[tts.remote]` keys.

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

### A remote listener

`provider = "remote"` under `[stt]` sends each utterance to the server in
`[stt.remote]` and types the text it answers. Any OpenAI-compatible
transcription server works: OpenAI, Groq, or a Whisper server you run. The
`preset` is not read; the server's `model` is. The `vocabulary` list goes out with
each request as the server's prompt, so those words leave the machine too. Set
both sides up in one go:

```
banshee config remote
```

It asks about the listener first, then the speaker. For the listener it asks for
the server's `/v1` root, the model, and the key. Every prompt but the key shows
its current value, and Enter keeps it. The key is typed without echo, and Enter
keeps the key already on file. The key is stored in
`~/.banshee/credentials.toml`, which only you can read. Each setting also stands
alone: `banshee config set stt.remote.api_key` asks for the key by itself. To
remove the key, pass an empty one: `banshee config set stt.remote.api_key ""`.
The key is never written to `config.toml` and never appears in a status reply. When
a transcription fails, the error tone plays, and `banshee status` and the window
say why. Banshee never falls back to the local model on its own.

Once a side has a key on file, `banshee status` asks that server for `/models`
with it. It passes on an answer, reports a refused key with the `banshee
config set` command that fixes it, and reports an unreachable server by name.
A server with no `/models` path earns a note instead of a failure. This runs
for the listener and the speaker alike.

### A remote voice

`provider = "remote"` under `[tts]` sends each reply's text to the server in
`[tts.remote]` and plays the audio it answers. Any OpenAI-compatible
`/audio/speech` endpoint works. `tts.voice` is not read; `tts.remote.voice` is.
The speaker needs one: the endpoint has no call that lists voices, so Banshee
cannot pick one for you. `instructions` is optional. A model such as
`gpt-4o-mini-tts` takes tone and delivery from it. The default `tts-1` ignores
it. A server may also refuse the whole request over the field: Groq answers
``unknown field `instructions` in request body``. Leave `instructions` empty
unless the model you name reads it. `tts.speed` still applies, and it stays
live: a write reaches the server on the next reply.

`response_format` is what the server is asked to send. The default is `wav`,
because every OpenAI-compatible server offers it. A WAV file also states its
own rate and channel count, so Banshee plays it at the rate the server chose.
`pcm` saves the 44-byte header, and OpenAI and Kokoro-FastAPI both answer it.
Bare samples describe nothing, so Banshee reads them as 16-bit mono at 24000
Hz. `sample_rate` changes that rate, for a server that answers `pcm` at
another one. Banshee sends the field to the server only when you set it.

Banshee identifies every answer from its own bytes. It refuses one it cannot
play, and it names what arrived. A server that sends MP3, Ogg or an error page
in place of audio says so in `banshee status`. Nothing plays as noise. Bare
samples are the one answer no byte can prove, so Banshee reads unrecognised
bytes as samples only under `response_format = "pcm"`.

`banshee config remote` sets both sides up. After the listener it asks for the
speaker's server, model, voice and key. Each setting also stands alone:
`banshee config set tts.remote.api_key` asks for the key by itself, and
`banshee config set tts.remote.voice marin` names the voice. The key lives in
`~/.banshee/credentials.toml` beside the listener's, in its own table, and never
in `config.toml` or a status reply. The daemon reads `tts.provider` and the
`[tts.remote]` keys when it starts, so a change to one of them ends with a
restart.

With `fallback = "system"`, an utterance the server refuses plays the error
tone, and the system voice says it instead. You still hear an agent's
question. That voice is `say` on macOS and `espeak-ng` on Linux; without
`espeak-ng` installed, the fallback does not start, and the reason names the
install command.
With `fallback = "none"` the tone plays and Banshee says nothing. A question
asked through `ask_user` still opens the microphone after it, so you hear the
tone and then silence while Banshee waits for your answer. Either way three
places say why: `banshee status`, the status reply's `last_speech_error` and
the window's Voice panel. A reply that fails after its first words ends
where it stopped. A restart in the system voice mid-sentence is worse than a
stop. A speaker that will not start never stops the daemon, so dictation goes
on. `banshee config remote` writes `tts.provider = "local"` when the voice is
left empty and `"remote"` otherwise.

## The hotkey

Hold the key to record and let go to stop. Any recording that runs past 120
seconds, held or toggled, is ended by the watchdog, which returns the
microphone and still transcribes what it heard.

The key is rebindable: `banshee config set audio.hotkey F6`, then
`banshee start`. Legal values are an F-key (`F1`–`F12`), a modifier alone, or
modifiers and a key, as in `Ctrl+Alt+D`. The modifiers are `RightOption`,
`LeftOption`, `LeftControl` and `LeftCommand`, plus `RightCommand` and `Fn` on
macOS and `RightControl` on Linux.
A modifier bound alone still works as a modifier: `RightOption+E` types é, and
banshee discards the accidental recording instead of transcribing it.
