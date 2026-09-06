# Troubleshooting

Start with `banshee status`; it catches most setup problems and tells you the
fix. The daemon logs to `~/.banshee/daemon.log`, and `banshee serve` runs it in
the foreground when you want to watch it work. Beyond that:

- **The microphone looks dead: you record, and nothing ever comes back.**
  Usually the machine is slow, not broken: on an older CPU the `balanced`
  model can take minutes on a few seconds of speech. Run `banshee serve` and
  watch the `Transcribed` line; if it warns about slower-than-realtime, set
  `preset = "fast"` and run `banshee setup`. On a 2014 dual-core laptop that
  took one clip from 104s to 4.8s.
- **`banshee status` fails the microphone check on a fresh install.** Capture
  closes when a model cannot load, so until `banshee setup` has finished there
  is no open stream and the check fails, naming the download as the fix. Run
  `banshee setup`, then `banshee start`; the line then names your device.
- **`banshee record start` says the microphone is busy.** A previous
  push-to-talk never got its `stop`. `banshee record stop` clears it; the
  daemon also releases the mic on its own after two minutes.
- **Audio sounds muffled on Bluetooth earbuds while Banshee runs.** macOS
  switches earbuds to their telephony profile while any app holds their mic.
  In **System Settings > Sound**, set _Input_ to the built-in microphone and
  leave _Output_ on the earbuds — the built-in mic transcribes better anyway.
- **Hotkeys or typing stopped working, but no error appears.** macOS withholds
  input events silently when an Accessibility grant is stale. Remove the Banshee
  entry from **System Settings > Privacy & Security > Accessibility**, restart
  the daemon, and approve the fresh prompt.
- **You reinstalled the app, the Accessibility row is on, and Banshee still says
  the grant is missing.** Deleting and reinstalling `/Applications/Banshee.app`
  leaves the old row behind, and a row that looks on does not cover the new copy.
  Select the row, click the minus button, restart the daemon, and approve the
  fresh prompt.
- **Permissions granted, but Banshee keeps asking.** Grants only apply to newly
  started processes. Restart the daemon with `banshee start`.

## Binding an F-key on a Mac

macOS ships the top row as media keys, so a plain `F5` press starts Apple's
own Dictation and never reaches the daemon; hold `Fn` to send the real key. To
make F-keys single presses, turn on _Settings → Keyboard → "Use F1, F2, etc.
keys as standard function keys"_.
