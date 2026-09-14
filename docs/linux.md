# Linux

- **The daemon, the CLI and the agent voice** work the same as on macOS.
- **This page** holds what differs: the key, typing, the state in a bar, and the
  window.

## The key on Wayland

- **The global hotkey needs X11.**
- **On a Wayland session** (Hyprland, Sway, GNOME), bind the record commands in
  your compositor instead.
- **`banshee bind hyprland`** does this for Hyprland.
- **It asks for the key,** then whether you hold it while you speak or tap it
  to start and stop.
- **The default key** is `audio.hotkey`. A lone modifier falls back to `F9`,
  because Hyprland fires its binds only on the release.
- **It saves both answers** as `audio.hotkey` and `audio.hotkey_mode`.
- **`--yes`** skips the questions and binds the saved key and mode.
- **Bind owns only the lines** between `BEGIN BANSHEE MANAGED BLOCK` and
  `END BANSHEE MANAGED BLOCK`, and replaces only those.
- **A bind of your own** outside the markers stays as it is.
- **A config with no markers** gets the block at the end.
- **Bind names each older line** that runs `banshee record` outside the block,
  by file and line, before the diff. It removes none of them.
- **Sway and GNOME** have no connector. Bind the same commands by hand, in their
  own syntax.
- **On Omarchy** the block goes to `~/.config/hypr/bindings.lua`.
- **Elsewhere** it goes to `~/.config/hypr/hyprland.conf`.
- **`hyprctl reload`** runs after the write.
- **`banshee bind` alone** prints the block for you to paste, and names the
  file it would write.

```lua
-- BEGIN BANSHEE MANAGED BLOCK
o.bind("F9", "Banshee: hold to dictate", "banshee record start --dictate")
o.bind("F9", nil, "banshee record stop", { release = true })
o.bind("SHIFT + F9", "Banshee: hold to record", "banshee record start")
o.bind("SHIFT + F9", nil, "banshee record stop", { release = true })
-- END BANSHEE MANAGED BLOCK
```

```conf
# BEGIN BANSHEE MANAGED BLOCK
bind  = , F9, exec, banshee record start --dictate
bindr = , F9, exec, banshee record stop
bind  = SHIFT, F9, exec, banshee record start
bindr = SHIFT, F9, exec, banshee record stop
# END BANSHEE MANAGED BLOCK
```

- **Tap** binds one toggle for each key, with no release bind:

```lua
-- BEGIN BANSHEE MANAGED BLOCK
o.bind("F9", "Banshee: tap to dictate", "banshee record toggle --dictate")
o.bind("SHIFT + F9", "Banshee: tap to record", "banshee record toggle")
-- END BANSHEE MANAGED BLOCK
```

```conf
# BEGIN BANSHEE MANAGED BLOCK
bind = , F9, exec, banshee record toggle --dictate
bind = SHIFT, F9, exec, banshee record toggle
# END BANSHEE MANAGED BLOCK
```

- **Both release binds** in the hold block are there on purpose, in either config style.
- **Hyprland** matches modifiers exactly, and `Shift` may be released before
  `F9`.
- **Bind an ordinary key,** not a modifier.
- **A bare modifier's press action** waits, because the press might still become
  the start of a different chord.
- **Hyprland knows** only when the key is released. It then fires the press and
  release binds together, whatever the hold time.
- **A `Right Alt`-only bind** plays both earcons back to back, and captures no
  audio.
- **`F9` is not a modifier,** so it has no such ambiguity, and it dispatches its
  press bind at once.
- **`banshee record toggle --dictate`** is one key that starts and stops, for a
  compositor with no release bind.

## Typing into the focused app

- **`wtype`** types into the focused app on wlroots compositors.
- **`ydotool`** works anywhere, with its own daemon and uinput access.
- **Without one,** dictation reports an error, and the transcription stays in
  `banshee history`.
- **`banshee status`** tells you which one it found.

## Showing the state in a bar

- **`banshee tray`** puts the mark in any bar that hosts StatusNotifierItem.
- **`banshee watch --waybar`** feeds a Waybar module.
- **It emits** one custom-module object per line:

```json
{"text":"recording","alt":"recording","class":"recording","tooltip":"Banshee is recording. Microphone: Blue Yeti"}
```

- **`text`** shows.
- **`alt`** picks a `format-icons` entry.
- **`class`** is the CSS hook.
- **Put this** in your Waybar config:

```jsonc
"custom/banshee": {
    "exec": "banshee watch --waybar",
    "return-type": "json",
    "restart-interval": 5,
    "format": "{icon}",
    "format-icons": { "idle": "mic", "recording": "REC", "speaking": "spk" }
}
```

- **Style it** in your own CSS:

```css
#custom-banshee.recording { color: #e06c75; }
#custom-banshee.speaking  { color: #61afef; }
```

- **`restart-interval` matters:** the command exits when the daemon stops.
- **That exit** is how the module reconnects once the daemon comes back.
- **The same channel** is open to any client over `banshee.subscribe`.

## Building the desktop window

- **The window** is a Tauri app. It needs WebKitGTK, GTK 3 and Node 22.
- **The tray** needs GTK 3.
- **The daemon and the CLI** need none of them to run.
- **A build from source** still needs the GTK 3 headers, because one crate holds
  the daemon, the CLI and the tray.

```bash
# Arch
sudo pacman -S --needed webkit2gtk-4.1
# Debian and Ubuntu
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev
# Fedora (untested)
sudo dnf install webkit2gtk4.1-devel gtk3-devel librsvg2-devel
```

Then:

```bash
cargo install tauri-cli --version "^2" --locked
make install
```

- **`make install`** builds and installs the daemon, the CLI and the window.
- **It builds the window** only when `webkit2gtk-4.1`, `gtk+-3.0`, `npm` and the
  Tauri CLI are present. The window needs Node 22.
- **Without them** it installs the daemon and the CLI, and names what is missing.
  A machine with no GTK still runs them.
- **A plain `cargo build` of `banshee-app`** leaves out the UI. The window then
  loads `localhost:5173` and shows "connection refused".
- **It puts** `banshee-app` in `~/.local/bin`, a desktop entry in
  `~/.local/share/applications`, and icons in `~/.local/share/icons/hicolor`.
- **The launcher** opens the window with WM class `banshee-app`.
- **The install points** `banshee-app` at `target/release`.
- **A `cargo clean` or a moved clone** breaks the window's daemon control.
- **Run `make install` again** to put it back.
