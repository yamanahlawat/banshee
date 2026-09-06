# Linux

The daemon, the CLI and the agent voice work the same as on macOS. Two things
differ, both below. The desktop window is macOS only today.

## The hotkey on Wayland

The global hotkey needs X11, so on a Wayland session (Hyprland, Sway, GNOME)
bind the record commands in your compositor instead. For push-to-talk on `F5`,
put these in `~/.config/hypr/hyprland.conf` (or `bindings.conf` on Omarchy),
then run `hyprctl reload`:

```conf
bind  = , F5, exec, banshee record start --dictate
bindr = , F5, exec, banshee record stop           # bindr fires on release
bind  = SHIFT, F5, exec, banshee record start
bindr = SHIFT, F5, exec, banshee record stop
```

Both release binds are there on purpose: Hyprland matches modifiers exactly,
and `Shift` may be released before `F5`.

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
