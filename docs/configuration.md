# Configuration

Every setting that changes behaviour.

- **Nothing here is required.** The defaults work.
- **To override one,** create `~/.banshee/config.toml`.

The defaults:

```toml
[daemon]
save_history = true    # keep transcriptions in ~/.banshee/banshee.db

[stt]
provider = "local"       # local | remote; see "A remote listener" below
model = "parakeet-unified-en-0.6b"  # a listening model id; see "Tuning the listener"
vad_threshold = 0.5      # 0.0 to 1.0; higher means stricter speech detection
vocabulary = []          # words the model keeps mangling, for example ["clippy", "tokio"]
translate = false        # true answers in English, for a model or server that translates
endpoint_silence_ms = 2500  # trailing silence that ends a spoken answer

[stt.remote]                           # read when provider = "remote"
base_url = "https://api.openai.com/v1" # an OpenAI-compatible /v1 root; https, or http on your own network
model = "whisper-1"                    # the model that server names

[tts]
provider = "local"     # local | remote; see "A remote voice" below
voice = "companion"    # companion, or any voice from the Kokoro voices directory
fallback = "local"     # local = the voice above speaks a failed remote reply | none

[tts.remote]                           # read when provider = "remote"
base_url = "https://api.openai.com/v1" # an OpenAI-compatible /v1 root; https, or http on your own network
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

[feedback]
mode = "both"          # visual | sound | both | none
```

- **A new install on macOS starts on `visual`.** Its first model download writes
  `visual`, or `both` when VoiceOver is on. Elsewhere the absent key means both.
- **`audio.cues.enabled` is replaced by `feedback.mode`.** A file that still
  sets it to false reads as none, until it sets `feedback.mode`.
- **`visual` draws a small Banshee figure above the Dock on macOS.** The menu
  bar icon draws it, and the window starts the icon. `visual` plays no earcons
  while the menu bar icon draws the figure, and plays every sound with no
  menu bar icon running.
- **`input_device` is a case-insensitive substring** of the microphone name, so
  `"yeti"` matches `Blue Yeti Stereo Microphone`.
- **An exact name wins** over a longer name that contains it, so a `Yeti` next
  to a `Blue Yeti Pro` opens its own device.
- **`banshee devices`** shows the names to choose from:

```
$ banshee devices
  Blue Yeti               system default, in use
  BlackHole 2ch
  MacBook Pro Microphone
```

- **`"default"` follows the OS while Banshee runs.**
- Connect a headset, macOS makes it the system default, and Banshee moves
  capture to it within about five seconds.
- **A device you name is not treated this way.**
- Banshee opens the device you named, and the system default never takes its
  place while it is present.
- **A microphone that disappears does not stop dictation.** Unplug the headset
  you named, Banshee records from the system default instead, and a press still
  works.
- **Banshee says which microphone it moved to,** and which one it still waits
  for.
- **The tray, `banshee status` and `banshee watch --waybar`** all show
  `MacBook Pro Microphone (waiting for "yeti")`.
- **Reconnect the headset** and Banshee takes it back within about five seconds.
  Nothing needs a restart.
- **Banshee never picks a different microphone in silence.** If it cannot open
  any device at all, it says so and refuses to record.

## Changing a setting without an editor

`banshee config set` writes one key and keeps your comments and layout:

```bash
banshee config set audio.hotkey RightOption
banshee config set stt.vad_threshold 0.7
banshee config set stt.vocabulary '["tokio", "clippy"]'
banshee config set feedback.mode none
```

- **The key is the section and the field,** as they appear in the file.
- **A number, a `true`, or a `[list]`** is read as that type. Anything else is
  read as text.
- **Quote twice to force text,** as in
  `banshee config set audio.input_device '"12"'`.
- **A value the field does not accept is refused,** and the message lists the
  legal ones.
- **The command works** whether or not the daemon runs.
- **Most settings take effect at once:**
  - `stt.vad_threshold`, `stt.vocabulary`, `stt.model`, `stt.translate`
  - `audio.input_device`, `audio.barge_in`, `feedback.mode`
  - `tts.voice`, `daemon.save_history`
- **The rest are read when the daemon starts,** so the command tells you to
  restart. Among them:
  - `audio.hotkey`, `audio.hotkey_mode`, `stt.endpoint_silence_ms`
  - `stt.provider` and the `[stt.remote]` keys
  - `tts.fallback`, `tts.provider` and the `[tts.remote]` keys
- **A live setting whose model is not downloaded yet waits for the file.**
- Once `banshee setup` fetches it, a running daemon applies the setting as the
  download finishes.
- **A daemon that started without its models is a different case.**
- It has no pipeline to change, so the first setup on a new machine ends with
  a restart.

## Pronouncing unknown words

- **A Kokoro voice spells a word it does not know letter by letter**, so
  `kustomize` reads "K U S T O M I Z E".

## Choosing a voice

`banshee voices` lists the voices on disk and marks the one the daemon loaded
with `*`:

```
$ banshee voices
* Companion  Pocket TTS by Kyutai, CC-BY-4.0  (companion)
  Heart  American, soft  (af_heart)
  Sky  American, clear  (af_sky)
  Adam  American, low  (am_adam)

Speak with one by: banshee config set tts.voice "<id>"
```

- **It lists only what is downloaded,** so every name it prints works today.
- **The default is `companion`,** a synthetic voice made for Banshee. It runs
  on Pocket TTS by Kyutai, CC-BY-4.0.
- **Its files are about 400 MB.** They land in
  `~/.banshee/models/pocket-english_2026-04/` when `banshee setup` or the
  window's Get button fetches them.
- **Speech plays at 1x.** No setting changes the rate.
- **Nothing is marked when the voice did not load.** The daemon stays up and
  silent, `speak` fails and names `banshee setup`, and `banshee status` says
  why. Once the files arrive, the next reply loads the voice with no restart.
- **A Kokoro voice needs two files:** Kokoro's engine file and the voice's own.
- **Any other `voice` takes a file** in the [Kokoro voices
  directory](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/tree/main/voices),
  for example `af_bella`, `am_michael`, or `bf_emma`.
- **The prefix is accent and gender:** `a`merican/`b`ritish, `f`emale/`m`ale.
- **After a change to `voice`,** run `banshee setup` to fetch the new file.
- **The window's Voice panel** lists every voice Banshee can name and fetches
  the one you pick, and its Microphone panel fetches a listening model you choose
  and do not have.

## Tuning the listener

- **Banshee waits `endpoint_silence_ms`** before it decides you are done.
- **Lower it** if replies feel sluggish.
- **Raise it** if Banshee cuts you off.

`stt.model` names the listening model. Banshee runs it on transcribe.cpp.

| Model id                    | Size   | Trade-off                                                                 |
| --------------------------- | ------ | ------------------------------------------------------------------------- |
| `parakeet-unified-en-0.6b`  | 731 MB | The default. English, shows live words in the chip while you speak.       |
| `whisper-large-v3-turbo`    | 619 MB | Many languages. It does not translate, so `stt.translate` has no effect.  |

- **Any of the 73 transcribe.cpp models downloads by id:** `banshee setup --model <id>`,
  then `banshee config set stt.model <id>`. `banshee setup --list` shows each id with its
  size, languages, whether it streams and its licence.
- **18 models need `--accept-licence`:** the CC-BY-NC ones, Moonshine, HAI-DEF and FunASR.
  The refusal names the licence and the command.
  Banshee records an acceptance in `~/.banshee/accepted-licences`, one licence name per line.
- **Typed text comes from a full pass over the recording** when you let go. The stream
  drives only the live words in the chip, on macOS, during dictation and spoken answers.
- **`stt.vocabulary` is empty by default.** A model that takes vocabulary receives the
  words. For any other model, a corrector swaps a close-sounding word for the listed one,
  with the rule from Handy (threshold 0.10, Soundex weight 0.5).
- **`stt.preset` and `stt.language` are gone.** The daemon refuses to start while either is
  in `config.toml`, and one error names every removed key. `banshee config set stt.preset null`
  removes a key.
- **Banshee deletes the old Whisper files** `ggml-base.en.bin`,
  `ggml-large-v3-turbo-q5_0.bin` and `ggml-large-v3-q5_0.bin` from `~/.banshee/models` at start.

## Remote servers

- **`banshee config remote` sets both sides up in one run.** It asks about the
  listener first, then the speaker.
- **For each side** it asks for the server's `/v1` root, the model and the key,
  and for the speaker a voice as well.
- **Every prompt but the key shows its current value,** and Enter keeps it.
- **The key is typed without echo,** and Enter keeps the key already on file.
- **The command writes `tts.provider = "local"`** when the voice is left empty,
  and `"remote"` otherwise.
- **Both keys go to `~/.banshee/credentials.toml`,** which only you can read.
- **A key is never written to `config.toml`** and never appears in a status
  reply.
- **The daemon reads the providers and both remote tables when it starts,** so
  the command ends with a restart.
- **A server out on the internet needs `https`.** Banshee accepts plain `http`
  to this machine and to your own network:
  - a private or link-local address, such as `192.168.1.5`
  - a Tailscale address, or a name ending in `.ts.net`
  - a name with no dot, such as `gpu-box`
  - a name ending in `.local`, `.lan`, `.home.arpa` or `.internal`
- **Banshee reads only the URL and resolves no name.** It refuses a public name
  that points at a home address. Use the address or the Tailscale name instead.
- **A config that already holds plain `http` to a public host still loads.**
  That side refuses to send, and `banshee status` names the setting to change.
- **A redirect follows the same rule.** Banshee refuses a server's redirect to
  plain `http` on the internet, and names the host it was sent to.

### A remote listener

- **`provider = "remote"` under `[stt]`** sends each utterance to the server in
  `[stt.remote]` and types the text it answers.
- **Any OpenAI-compatible transcription server works:** OpenAI, Groq, or a
  Whisper server you run.
- **`stt.model` is not read.** The server's `model` is.
- **The server gets no language hint.** `stt.language` is gone.
- **The `vocabulary` list goes out with each request** as the server's prompt,
  so those words leave the machine too.
- **`banshee config set stt.remote.api_key`** asks for the key by itself.
- **To remove the key, pass an empty one:**
  `banshee config set stt.remote.api_key ""`.
- **A failed transcription plays the error tone,** and `banshee status` and the
  window say why.
- **Banshee never falls back** to the local model on its own.
- **`banshee status` probes a side** whose provider is `remote` and whose key is
  on file. It asks that server for `/models` with the key.
- **A server that answers earns a pass.**
- **It reports a refused key** with the `banshee config set` command that fixes
  it.
- **It reports an unreachable server by name.**
- **It reports any other error code** as "the remote listener answered HTTP
  <code>", and the fix it names is "check the server at <host>".
- **A server with no `/models` path** earns a note instead of a failure.

### A remote voice

- **`provider = "remote"` under `[tts]`** sends each reply's text to the server
  in `[tts.remote]` and plays the audio it answers.
- **Any OpenAI-compatible `/audio/speech` endpoint works.**
- **`tts.voice` is not read.** `tts.remote.voice` is.
- **The speaker needs one.** The endpoint has no call that lists voices, so
  Banshee cannot pick one for you.
- **`instructions` is optional.** A model such as `gpt-4o-mini-tts` takes tone
  and delivery from it.
- **The default `tts-1` ignores it.** A server may also refuse the whole request
  over the field: Groq answers ``unknown field `instructions` in request body``.
- **Leave `instructions` empty** unless the model you name reads it.
- **The request carries no `speed` field.** Speech plays at 1x.
- **`banshee config set tts.remote.api_key`** asks for the key by itself.
- **`banshee config set tts.remote.voice marin`** names the voice.
- **`response_format` is what the server is asked to send.** The default is
  `wav`, because every OpenAI-compatible server offers it.
- **A WAV file states its own rate and channel count,** so Banshee plays it at
  the rate the server chose.
- **`pcm` omits the 44-byte header,** and OpenAI and Kokoro-FastAPI both answer
  it.
- **Under `pcm`, Banshee also sends `stream_format = "audio"`.** That is OpenAI's
  own field, and it makes a server such as audio.cpp stream its answer. A server
  that refuses unknown fields refuses the request over it. Groq is one, but it
  answers only `wav`, where the field is never sent.
- **Bare samples describe nothing,** so Banshee reads them as 16-bit mono at
  24000 Hz.
- **`sample_rate` changes that rate,** for a server that answers `pcm` at
  another one. Banshee sends the field to the server only when you set it,
  and never under `wav`, where the header states the rate.
- **The Voice panel sets both.** It shows the rate only under `pcm`, and an
  empty rate field goes back to 24000 Hz.
- **The type the server declares names the format.** A type that names none,
  such as `application/octet-stream`, leaves the format Banshee asked for.
  tts.ai and audio.cpp answer `pcm` with a WAV file and declare it, and Banshee
  plays it.
- **An answer declared as MP3, Ogg or another format Banshee cannot play,** or
  as something that is not audio, is refused, and `banshee status` names the
  type.
- **Under `pcm`, the bytes play as samples whatever they look like.** A server
  that sent another format under a type that names none would play as noise.
  None of the servers tested so far does.
- **With `fallback = "local"`,** the default, an utterance the server refuses
  before its reply starts plays the error tone, and the local voice named by
  `tts.voice` says it instead.
- You still hear an agent's question.
- **The local voice loads on first use.** If its files are missing, the fallback
  stays silent, and the reason names `banshee setup`.
- **With `fallback = "none"`** the tone plays and Banshee says nothing. A remote
  speaker then downloads no local voice.
- **`fallback = "system"` is refused by name.** Change it to `local` or `none`.
- **A question asked through `ask_user`** still opens the microphone after it,
  so you hear the tone and then silence while Banshee waits for your answer.
- **Either way three places say why:** `banshee status`, the status reply's
  `last_speech_error` and the window's Voice panel.
- **A reply that fails after its first words ends where it stopped.** A restart
  in another voice mid-sentence is worse than a stop.
- **A speaker that will not start never stops the daemon,** so dictation goes
  on.

## The hotkey

- **Hold the key to record.** Let go to stop.
- **On Wayland** the daemon binds no key and reads neither `hotkey` nor
  `hotkey_mode`. `banshee bind hyprland` asks for both, binds them in
  Hyprland, and saves them here.
- **With `hotkey_mode = "toggle"`** a tap starts the recording, and the next tap
  stops it.
- **The watchdog ends a recording after 120 seconds.** It returns the microphone
  and still transcribes what it heard.
- **The key is rebindable:** `banshee config set audio.hotkey F6`, then
  `banshee start`.
- **Legal values** are an F-key (`F1`-`F12`), a modifier alone, or modifiers and
  a key, as in `Ctrl+Alt+D`.
- **The modifiers are** `RightOption`, `LeftOption`, `LeftControl` and
  `LeftCommand`, plus `RightCommand` and `Fn` on macOS and `RightControl` on
  Linux.
- **Banshee refuses `Shift` and `CapsLock` as the hotkey.** It ignores Shift,
  so Shift can never be part of the hotkey, even in a chord. A bound `CapsLock`
  flips the lock state on every press, so dictation toggles caps as you speak.
- **A modifier bound alone still works as a modifier.** `RightOption+E` types é,
  and Banshee discards the accidental recording. It does not transcribe it.
- **On Wayland the daemon sees no key,** so `banshee bind hyprland` writes the
  binding in the compositor instead. See [linux.md](linux.md).
