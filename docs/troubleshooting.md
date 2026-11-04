# Troubleshooting

This page lists what breaks, and the fix.

- Start with `banshee status`; it catches most setup problems and names the
  fix.
- On macOS the daemon logs to `~/Library/Logs/com.banshee.app/daemon.log`, and
  the tray to `tray.log` in the same folder. Console.app shows both under Log
  Reports. `banshee status` prints the path. On
  Linux, run `journalctl --user -u banshee`. Every line carries a clock and a
  level.
- `BANSHEE_LOG=debug banshee serve` adds what Banshee heard and typed. Stop
  Banshee first; on macOS the background daemon ignores `BANSHEE_LOG`.
- On macOS, if `banshee status` says Banshee is switched off, turn it on in
  System Settings > General > Login Items.
- Run `banshee serve` in the foreground to watch it work.

- **The microphone looks dead: you record, and nothing ever comes back.**
  - Usually the machine is slow, not broken; a large model can take minutes on a few
    seconds of speech on an older CPU.
  - Run `banshee serve` and watch the `Transcribed` line; if it warns about
    slower-than-realtime, pick a smaller model: `banshee setup --list`, then
    `banshee setup --model <id>` and `banshee config set stt.model <id>`.
- **The daemon will not start and names `stt.preset` or `stt.language`.**
  - Those keys are gone. Run `banshee config set stt.preset null`, and the same for
    `stt.language`.
  - The window shows the reason too. The tray's first row runs `banshee start`
    and reads `Starting Banshee` while it runs. If it fails, the row reads
    `Start failed. Open Banshee` and the window's Start shows the reason.
- **`banshee status` fails the microphone check on a fresh install.**
  - Capture closes when a model cannot load, so the check fails until
    `banshee setup` finishes.
  - It names the download as the fix.
  - Run `banshee setup`, then `banshee start`; the line then names your
    device.
- **A new Mac install plays no earcons.**
  - A new install on macOS starts on `visual`. It shows the figure above the
    Dock and plays no earcons while the menu bar icon draws the figure.
  - Run `banshee config set feedback.mode both` for the figure and the
    earcons, or `sound` for the earcons alone.
- **`banshee record start` says the microphone is busy.**
  - A previous push-to-talk never got its `stop`.
  - Run `banshee record stop`, or wait two minutes for the daemon to
    release the mic on its own.
- **Audio sounds muffled on Bluetooth earbuds while Banshee listens.**
  - macOS switches earbuds to their telephony profile while any app holds
    the mic. Banshee holds it only for a use, so the sound returns to stereo
    right after.
  - If it stays muffled, check `audio.keep_open`. When true, the mic stays open.
    Run `banshee config set audio.keep_open false`.
  - To avoid the switch during a use, set _Input_ to the built-in microphone in
    **System Settings > Sound**, and leave _Output_ on the earbuds.
- **The start cue comes late on a Bluetooth headset.**
  - The cue waits for the first sound. The headset gives it about 300 ms after
    the open, measured on one headset.
- **The first reply after a quiet spell starts a little late.**
  - Banshee unloaded the voice after 5 idle minutes and loads it again, 195 to
    320 ms measured. Raise `models.unload_after`, or set 0, to keep it loaded.
  - On macOS the process keeps the freed memory for reuse and gives it back when
    the system runs short. On Linux it goes back at once.
- **The window shows an agent as not installed, but your shell finds it.**
  - The daemon takes its PATH from a shell it starts itself, not from your
    terminal. A service manager gives that shell almost no PATH to begin with.
  - Banshee asks an interactive login shell first, so a directory that only
    `.zshrc` or `.bashrc` adds still counts. An npm or nvm install is the
    common case.
  - The daemon log names the PATH the search used, and names every
    probe that failed with what the shell printed.
  - If the log shows a probe that timed out, your profile is slow. Move the
    PATH line to `.zshenv` on zsh, or `.profile` on bash, where a plain login
    shell reads it.
  - `~/.local/bin` is always searched, so the native installer needs nothing.
- **Hotkeys or typing stopped working, but no error appears.**
  - macOS withholds input events silently when the Accessibility grant is
    stale.
  - Remove the Banshee entry from **System Settings > Privacy & Security >
    Accessibility**.
  - Restart the daemon and approve the fresh prompt.
- **You reinstalled the app, the Accessibility row is on, and Banshee still
  says the grant is missing.**
  - Deleting and reinstalling `/Applications/Banshee.app` leaves the old row
    behind.
  - A row that looks on does not cover the new copy.
  - Select the row, click the minus button, restart the daemon, and approve
    the fresh prompt.
- **Permissions granted, but Banshee keeps asking.** Grants only apply to
  newly started processes. Restart the daemon with `banshee start`.
- **You switched the microphone off for Banshee, and nothing shows it.**
  - On macOS 13, the running daemon keeps the microphone permission it read at
    its start. Run `banshee start`, and the new daemon shows the problem.
  - With `audio.keep_open = true`, a refusal shows at the next status read or
    window focus. Recordings come back empty until then.
- **Dictation on Wayland says there is no typer on `PATH`.**
  - A Wayland session needs a helper to type into the focused window.
  - Install `wtype`, or `ydotool` where GNOME denies `wtype` the
    virtual-keyboard protocol it needs.
  - Banshee copies the words instead, and `banshee history` holds them too.
- **`banshee status` says the remote listener refused the key.**
  - The check asks that server for `/models` with the key, and the server
    rejected it.
  - Set the key again with `banshee config set stt.remote.api_key`, or with
    `banshee config set tts.remote.api_key` for the speaker.
- **The window says the remote listener's key file is unreadable.**
  - A new key is written into the same file, so removing it is the fix.
  - Run `rm ~/.banshee/credentials.toml`, then set the keys again with
    `banshee config remote`.
- **Banshee.app does not open, and the `banshee` command dies with no
  message.** Run the `xattr` line in [install.md](install.md).
- **Banshee and the menu bar icon are gone after an upgrade.** Run `banshee start`,
  then `banshee tray`.
  - macOS can refuse the first launch of a new build. A second start of each, run by hand
    14 to 44 s after the first, worked on 3 of 3 installs over a running build.
  - `banshee start` and `banshee tray` register once more by themselves. On one real
    upgrade, that retry brought up both the daemon and the menu bar icon.

## Binding an F-key on a Mac

- macOS ships the top row as media keys.
- A plain `F5` press starts Apple's own Dictation and never reaches the
  daemon.
- Hold `Fn` to send the real key.
- To make F-keys single presses, turn on _Settings > Keyboard > "Use F1, F2,
  etc. keys as standard function keys"_.
