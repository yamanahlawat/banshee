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
preset = "balanced"      # fast | balanced | quality (see below)
vad_threshold = 0.5      # 0.0 to 1.0; higher means stricter speech detection
vocabulary = ["banshee"] # words Whisper keeps mangling, for example ["clippy", "tokio"]
language = "en"          # a Whisper code, or "auto" to detect it
translate = false        # true answers in English whatever you spoke
endpoint_silence_ms = 2500  # trailing silence that ends a spoken answer

[stt.remote]                           # read when provider = "remote"
base_url = "https://api.openai.com/v1" # an OpenAI-compatible /v1 root; an http or https URL with a host
model = "whisper-1"                    # the model that server names

[tts]
provider = "local"     # local | remote; see "A remote voice" below
voice = "af_sky"       # any voice from the Kokoro voices directory
speed = 1.2            # 0.5 to 2.0; playback speed multiplier
fallback = "system"    # system = use the OS voice (say, or espeak-ng on Linux) | none

[tts.remote]                           # read when provider = "remote"
base_url = "https://api.openai.com/v1" # an OpenAI-compatible /v1 root; an http or https URL with a host
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

[tell]
agent = ""              # empty asks Omarchy's default agent, then the connected agents
thread_timeout_min = 10 # minutes the same agent conversation stays open
run_timeout_min = 5     # minutes before one run is killed
snapshots = 10          # copies of the folders below to keep; 1 is the floor
paths = [               # the folders Banshee copies, and the list a scoped agent gets
  "~/.config/hypr", "~/.config/omarchy", "~/.config/alacritty",
  "~/.config/foot", "~/.config/kitty", "~/.config/ghostty",
]
```

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
banshee config set audio.cues.enabled false
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
  - `stt.vad_threshold`, `stt.vocabulary`, `stt.preset`, `stt.language`,
    `stt.translate`
  - `audio.input_device`, `audio.barge_in`, `audio.cues.enabled`
  - `tts.voice`, `tts.speed`, `daemon.save_history`
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

- **Install `espeak-ng`** and Banshee pronounces unfamiliar words, such as tech
  jargon and proper nouns.
- **Without it** Banshee spells those words out letter by letter.
- **On macOS** that is `brew install espeak-ng`.
- **`banshee status`** prints the command for your system.

## Choosing a voice

`banshee voices` lists the voices on disk and marks the one the daemon loaded
with `*`:

```
$ banshee voices
  Heart  American, soft  (af_heart)
* Sky  American, clear  (af_sky)
  Adam  American, low  (am_adam)
  Santa  American, deep  (am_santa)

Speak with one by: banshee config set tts.voice "<id>"
```

- **It lists only what is downloaded,** so every name it prints works today.
- **Nothing is marked when Kokoro did not load.** The system fallback speaks in
  whatever voice macOS is set to, which Banshee did not choose.
- **`voice` takes any file** in the [Kokoro voices
  directory](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/tree/main/voices),
  for example `af_bella`, `am_michael`, or `bf_emma`.
- **The prefix is accent and gender:** `a`merican/`b`ritish, `f`emale/`m`ale.
- **After a change to `preset` or `voice`,** run `banshee setup` to fetch the
  new file.
- **The window's Voice panel** lists every voice Banshee can name and fetches
  the one you pick.

## Tuning the listener

- **Banshee waits `endpoint_silence_ms`** before it decides you are done.
- **Lower it** if replies feel sluggish.
- **Raise it** if Banshee cuts you off.

The `preset` picks which Whisper model Banshee uses:

| Preset     | Model                          | Trade-off                                  |
| ---------- | ------------------------------ | ------------------------------------------ |
| `fast`     | `ggml-base.en.bin`             | Fastest and lightest, English only         |
| `balanced` | `ggml-large-v3-turbo-q5_0.bin` | The default; accurate and reasonably quick |
| `quality`  | `ggml-large-v3-q5_0.bin`       | Most accurate, heaviest                    |

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

### A remote listener

- **`provider = "remote"` under `[stt]`** sends each utterance to the server in
  `[stt.remote]` and types the text it answers.
- **Any OpenAI-compatible transcription server works:** OpenAI, Groq, or a
  Whisper server you run.
- **The `preset` is not read.** The server's `model` is.
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
- **`tts.speed` still applies, and it stays live.** A write reaches the server
  on the next reply.
- **`banshee config set tts.remote.api_key`** asks for the key by itself.
- **`banshee config set tts.remote.voice marin`** names the voice.
- **`response_format` is what the server is asked to send.** The default is
  `wav`, because every OpenAI-compatible server offers it.
- **A WAV file states its own rate and channel count,** so Banshee plays it at
  the rate the server chose.
- **`pcm` omits the 44-byte header,** and OpenAI and Kokoro-FastAPI both answer
  it.
- **Bare samples describe nothing,** so Banshee reads them as 16-bit mono at
  24000 Hz.
- **`sample_rate` changes that rate,** for a server that answers `pcm` at
  another one. Banshee sends the field to the server only when you set it.
- **Banshee identifies every answer from its own bytes.** It refuses one it
  cannot play, and it names what arrived.
- **A server that sends MP3, Ogg or an error page** in place of audio says so in
  `banshee status`. Nothing plays as noise.
- **Bare samples are the one answer no byte can prove,** so Banshee reads
  unrecognised bytes as samples only under `response_format = "pcm"`.
- **With `fallback = "system"`,** an utterance the server refuses plays the
  error tone, and the system voice says it instead.
- You still hear an agent's question.
- **That voice is `say` on macOS and `espeak-ng` on Linux.** Without `espeak-ng`
  installed, the fallback does not start, and the reason names the install
  command.
- **With `fallback = "none"`** the tone plays and Banshee says nothing.
- **A question asked through `ask_user`** still opens the microphone after it,
  so you hear the tone and then silence while Banshee waits for your answer.
- **Either way three places say why:** `banshee status`, the status reply's
  `last_speech_error` and the window's Voice panel.
- **A reply that fails after its first words ends where it stopped.** A restart
  in the system voice mid-sentence is worse than a stop.
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
- **`Shift` and `CapsLock` are reserved, not absent.** `Shift` plus the hotkey
  sends the speech to `banshee listen`, and Banshee does not type it.
- **A bound `CapsLock`** flips the lock state on every press.
- **A modifier bound alone still works as a modifier.** `RightOption+E` types é,
  and Banshee discards the accidental recording. It does not transcribe it.
- **On Wayland the daemon sees no key,** so `banshee bind hyprland` writes the
  binding in the compositor instead. See [linux.md](linux.md).

## Telling your agent

- **`banshee tell "<text>"` hands your words to a coding agent,** which edits
  your config and speaks the result.
- **Banshee ships no prompt and no desktop knowledge.** It sends your words and
  nothing else, and the agent's own skills carry the rest.
- **An empty `agent` asks `omarchy-default-agent` first,** then takes the first
  connected agent that has a headless mode.
- **`banshee config set tell.agent claude`** pins one instead.
- **Claude Code and OpenCode are the two agents with a headless mode Banshee
  has measured.** `banshee tell` names any other agent and refuses to run it.
- **The two agents get different scopes, and the difference is wide. Read it
  before you pick one:**
  - **Claude Code gets the folders in `tell.paths`, and its own run directory.**
    Nothing else is writable.
  - **OpenCode gets no folder list, so it can edit any file on the machine.**
    It refuses every write outside its own run directory, and the config block
    that should allow one hangs the run instead. Only `--auto` works, and
    `--auto` allows any edit anywhere.
- **The agent runs in `~/.banshee/tell/run/`, and Banshee keeps nothing there.**
  The snapshots, the saved thread and the run lock sit one level up, in
  `~/.banshee/tell/`. An agent that lists its own directory must not find a
  copy of your config there and edit the copy.
- **The tell key names no scope aloud.** Banshee never speaks for itself.
  `banshee tell` prints the scope in your terminal, on the first command of a
  thread. For the key, this page is the record.
- **`banshee status` names the agent it would run,** and whether it is scoped.
- **A failed run sounds the error cue, and nothing else.** Run `banshee status`
  for the reason: it names the last failure.
- **A run that was refused a tool sounds the same cue.** The agent finished, but
  it could not speak, so the key gives you silence and silence is what success
  sounds like. `banshee status` names the tool.
- **A run whose reply arrived too late only reaches `banshee status`.** The agent
  already spoke while it ran, so this one sounds no cue.
- **"start over" clears the thread and sounds nothing.** Silence is the chosen
  answer for a reset that works. A reset that fails sounds the error cue, and
  `banshee status` names the file Banshee could not remove.
- **The cues carry every tell failure, so `audio.cues.enabled = false` hides
  them.** `banshee status` says so while the cues are off.
- **A dictation no longer hides a tell failure.** `banshee status` keeps the last
  tell failure until the next tell run, whatever else you dictate in between.
- **`thread_timeout_min` is how long the same conversation stays open.** Within
  it, "a bit more" reaches the agent that did the work. After it, the next
  command starts a new thread.
- **`run_timeout_min` is a stated default, not a measurement.** No run has been
  timed to a limit. Raise it if a command is killed before it finishes.
- **Before every command Banshee copies each folder in `paths`** to
  `~/.banshee/tell/snapshots/<number>/`. The number is the Unix time in seconds.
  Two runs in one second get separate copies, because the second name rises
  above the first.
- **Each copy carries the whole path of its folder,** with `/` written as `%`.
  Two watched folders that share a basename then keep separate copies.
- **It keeps the newest `snapshots` copies,** and never fewer than one.
- **`banshee tell --undo` puts the newest copy back,** and names every folder
  it replaced.
- **A folder that is itself a symlink is refused rather than replaced,** and
  named, so a link into a dotfiles repo survives.
- **`paths` defaults to the six folders the Omarchy agent skill names.**
