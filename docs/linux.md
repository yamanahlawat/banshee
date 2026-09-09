# Linux

The daemon, the CLI and the agent voice work the same as on macOS. The
sections below cover what is different: the desktop window, the Wayland
hotkey, and the Waybar module.

## Building the desktop window

The window is a Tauri app. It needs WebKitGTK, GTK 3 and Node 22. The daemon
and the CLI need none of these.

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
make install-window
```

`make install` builds and installs the daemon and the CLI. A machine with no
GTK still runs it.

`make install-window` depends on `install`, so it installs the daemon, the
CLI and the window together. It needs GTK, WebKitGTK and Node 22. It puts
`banshee-app` in `~/.local/bin`, a desktop entry in
`~/.local/share/applications`, and icons in
`~/.local/share/icons/hicolor`. The launcher then opens the window with WM
class `banshee-app`.

The install points `banshee-app` at `target/release`, so a `cargo clean` or a
moved clone breaks the window's daemon control. Run `make install-window`
again to put it back.

There is no tray icon here yet. `banshee-tray` still exits on Linux.
`banshee watch --waybar` reports the state instead.

## The hotkey on Wayland

The global hotkey needs X11, so on a Wayland session (Hyprland, Sway, GNOME)
bind the record commands in your compositor instead. For push-to-talk on
`F5`:

**Hyprland**, in `~/.config/hypr/hyprland.conf` or `bindings.conf`, then run
`hyprctl reload`:

```conf
bind  = , F5, exec, banshee record start --dictate
bindr = , F5, exec, banshee record stop
bind  = SHIFT, F5, exec, banshee record start
bindr = SHIFT, F5, exec, banshee record stop
```

**Omarchy** configures Hyprland through Lua, not `hyprland.conf`. There is
no `bindings.conf`. Put these in `~/.config/hypr/bindings.lua` instead, then
run `hyprctl reload`:

```lua
o.bind("F5", "Banshee: start dictation", "banshee record start --dictate")
o.bind("F5", nil, "banshee record stop", { release = true })
o.bind("SHIFT + F5", "Banshee: start recording", "banshee record start")
o.bind("SHIFT + F5", nil, "banshee record stop", { release = true })
```

Both release binds are there on purpose, in either config style: Hyprland
matches modifiers exactly, and `Shift` may be released before `F5`.

Bind an ordinary key, not a modifier. Hyprland cannot dispatch a bare
modifier's press action immediately, because the press might still become
the start of a different chord. It only knows once the key is released, so
it fires the press and release binds together at that point, regardless of
how long the key was actually held. A `Right Alt`-only bind plays both
earcons back to back and never captures audio. `F5` is not a modifier, so it
has no such ambiguity and dispatches its press bind immediately.

Typing into the focused app needs **`wtype`** (wlroots compositors) or
**`ydotool`** (anywhere, with its own daemon and uinput access). Without one,
dictation reports an error and the transcription is kept in `banshee history`.
`banshee status` tells you which one it found.

## Showing the state in a Waybar module

`banshee watch --waybar` emits one Waybar custom-module object per line:

```json
{"text":"recording","alt":"recording","class":"recording","tooltip":"Banshee is recording. Microphone: Blue Yeti"}
```

`text` shows, `alt` picks a `format-icons` entry, and `class` is the CSS hook.
Put this in your Waybar config:

```jsonc
"custom/banshee": {
    "exec": "banshee watch --waybar",
    "return-type": "json",
    "restart-interval": 5,
    "format": "{icon}",
    "format-icons": { "idle": "mic", "recording": "REC", "speaking": "spk" }
}
```

and style it in your own CSS:

```css
#custom-banshee.recording { color: #e06c75; }
#custom-banshee.speaking  { color: #61afef; }
```

`restart-interval` matters: the command exits when the daemon stops, and that
is how the module reconnects once it comes back. The same channel is open to
any client over `banshee.subscribe`.
