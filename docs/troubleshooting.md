# Troubleshooting

This page lists what breaks, and the fix.

- Start with `banshee status`; it catches most setup problems and names the
  fix.
- The daemon logs to `~/.banshee/daemon.log`.
- Run `banshee serve` in the foreground to watch it work.

- **The microphone looks dead: you record, and nothing ever comes back.**
  - Usually the machine is slow, not broken; the `balanced` model can take
    minutes on a few seconds of speech on an older CPU.
  - Run `banshee serve` and watch the `Transcribed` line; if it warns about
    slower-than-realtime, set `preset = "fast"` and run `banshee setup`.
  - On a 2014 dual-core laptop that took one clip from 104s to 4.8s.
- **`banshee status` fails the microphone check on a fresh install.**
  - Capture closes when a model cannot load, so the check fails until
    `banshee setup` finishes.
  - It names the download as the fix.
  - Run `banshee setup`, then `banshee start`; the line then names your
    device.
- **`banshee record start` says the microphone is busy.**
  - A previous push-to-talk never got its `stop`.
  - Run `banshee record stop`, or wait two minutes for the daemon to
    release the mic on its own.
- **Audio sounds muffled on Bluetooth earbuds while Banshee runs.**
  - macOS switches earbuds to their telephony profile while any app holds
    the mic.
  - In **System Settings > Sound**, set _Input_ to the built-in microphone.
  - Leave _Output_ on the earbuds; the built-in mic transcribes better
    anyway.
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
- **Dictation on Wayland says there is no typer on `PATH`.**
  - A Wayland session needs a helper to type into the focused window.
  - Install `wtype`, or `ydotool` where GNOME denies `wtype` the
    virtual-keyboard protocol it needs.
  - The text is not lost either way: `banshee history` holds it.
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

## Binding an F-key on a Mac

- macOS ships the top row as media keys.
- A plain `F5` press starts Apple's own Dictation and never reaches the
  daemon.
- Hold `Fn` to send the real key.
- To make F-keys single presses, turn on _Settings > Keyboard > "Use F1, F2,
  etc. keys as standard function keys"_.
