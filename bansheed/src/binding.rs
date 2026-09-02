//! The hotkey binding: what `audio.hotkey` may say, and how raw key events
//! become recording actions. Pure data and logic; the listener thread and the
//! audio pipeline live in `hotkey.rs`.

use rdev::{EventType, Key};

use crate::config::HotkeyMode;
use crate::state::TranscribeTarget;

/// A parsed `audio.hotkey`. The parser constructs it, so a binding the
/// listener cannot match fails the config load instead of sitting silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "String", into = "String")]
pub enum Hotkey {
    Key {
        ctrl: bool,
        alt: bool,
        cmd: bool,
        key: Key,
    },
    /// A modifier pressed alone. Another key or a click during the hold makes
    /// it a chord, and the release then cancels the session it opened.
    Modifier(Key),
}

impl Default for Hotkey {
    // AltGr is the one right-hand modifier rdev maps on both platforms
    fn default() -> Self {
        Hotkey::Modifier(Key::AltGr)
    }
}

impl TryFrom<String> for Hotkey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        parse_hotkey(&value)
            .map_err(|reason| format!("hotkey \"{value}\": {reason}. Legal: {}", legal_forms()))
    }
}

const F_KEYS: [Key; 12] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
];

const TYPED_KEYS: [(char, Key); 36] = [
    ('A', Key::KeyA),
    ('B', Key::KeyB),
    ('C', Key::KeyC),
    ('D', Key::KeyD),
    ('E', Key::KeyE),
    ('F', Key::KeyF),
    ('G', Key::KeyG),
    ('H', Key::KeyH),
    ('I', Key::KeyI),
    ('J', Key::KeyJ),
    ('K', Key::KeyK),
    ('L', Key::KeyL),
    ('M', Key::KeyM),
    ('N', Key::KeyN),
    ('O', Key::KeyO),
    ('P', Key::KeyP),
    ('Q', Key::KeyQ),
    ('R', Key::KeyR),
    ('S', Key::KeyS),
    ('T', Key::KeyT),
    ('U', Key::KeyU),
    ('V', Key::KeyV),
    ('W', Key::KeyW),
    ('X', Key::KeyX),
    ('Y', Key::KeyY),
    ('Z', Key::KeyZ),
    ('0', Key::Num0),
    ('1', Key::Num1),
    ('2', Key::Num2),
    ('3', Key::Num3),
    ('4', Key::Num4),
    ('5', Key::Num5),
    ('6', Key::Num6),
    ('7', Key::Num7),
    ('8', Key::Num8),
    ('9', Key::Num9),
];

const SHIFT_RESERVED: &str = "Shift is the mailbox modifier: Shift + the hotkey \
     sends speech to `banshee listen` instead of typing it";

struct Modifier {
    name: &'static str,
    key: Key,
    // Some(reason) when this platform can never deliver the key
    refused: Option<&'static str>,
}

/// The one table behind the parser, `Display`, and the legal-forms message.
/// Rows the parser always refuses sit last, so a reverse lookup by key finds
/// the bindable name first.
const MODIFIERS: [Modifier; 11] = [
    Modifier {
        name: "RightOption",
        key: Key::AltGr,
        refused: None,
    },
    Modifier {
        name: "LeftOption",
        key: Key::Alt,
        refused: None,
    },
    Modifier {
        name: "LeftControl",
        key: Key::ControlLeft,
        refused: None,
    },
    Modifier {
        name: "LeftCommand",
        key: Key::MetaLeft,
        refused: None,
    },
    Modifier {
        name: "RightCommand",
        key: Key::MetaRight,
        refused: if cfg!(target_os = "macos") {
            None
        } else {
            Some("rdev has no MetaRight on Linux, so it can never match")
        },
    },
    Modifier {
        name: "RightControl",
        key: Key::ControlRight,
        refused: if cfg!(target_os = "macos") {
            // rdev's key_from_code has no arm for keycode 62, and a live
            // press produced no event at all
            Some("rdev never maps Right Control on macOS")
        } else {
            None
        },
    },
    Modifier {
        name: "Fn",
        key: Key::Function,
        refused: if cfg!(target_os = "macos") {
            None
        } else {
            Some("no Fn key reaches rdev on Linux, so it can never match")
        },
    },
    Modifier {
        name: "Shift",
        key: Key::ShiftLeft,
        refused: Some(SHIFT_RESERVED),
    },
    Modifier {
        name: "LeftShift",
        key: Key::ShiftLeft,
        refused: Some(SHIFT_RESERVED),
    },
    Modifier {
        name: "RightShift",
        key: Key::ShiftRight,
        refused: Some(SHIFT_RESERVED),
    },
    Modifier {
        name: "CapsLock",
        key: Key::CapsLock,
        refused: Some(
            "a bound CapsLock flips the lock state on every press, \
             so dictation toggles caps as you speak",
        ),
    },
];

fn legal_forms() -> String {
    let bindable: Vec<&str> = MODIFIERS
        .iter()
        .filter(|modifier| modifier.refused.is_none())
        .map(|modifier| modifier.name)
        .collect();
    format!(
        "an F-key (F1-F12), a modifier alone ({}), or modifiers and a key, as in Ctrl+Alt+D",
        bindable.join(", ")
    )
}

enum Main {
    FKey(Key),
    Typed(Key),
    Modifier(Key),
}

fn resolve(name: &str) -> Result<Main, String> {
    if let Some(number) = name
        .strip_prefix(['f', 'F'])
        .and_then(|digits| digits.parse::<u16>().ok())
    {
        return F_KEYS
            .get((number as usize).wrapping_sub(1))
            .copied()
            .map(Main::FKey)
            .ok_or_else(|| format!("rdev maps F1 to F12 only, so F{number} can never arrive"));
    }
    if let Some(modifier) = MODIFIERS
        .iter()
        .find(|modifier| name.eq_ignore_ascii_case(modifier.name))
    {
        return match modifier.refused {
            None => Ok(Main::Modifier(modifier.key)),
            Some(reason) => Err(reason.to_string()),
        };
    }
    let wanted = name.chars().next().map(|c| c.to_ascii_uppercase());
    if name.chars().count() == 1
        && let Some(&(_, key)) = TYPED_KEYS.iter().find(|(c, _)| Some(*c) == wanted)
    {
        return Ok(Main::Typed(key));
    }
    Err(format!("\"{name}\" is not a key the parser knows"))
}

fn parse_hotkey(value: &str) -> Result<Hotkey, String> {
    let parts: Vec<&str> = value.split('+').map(str::trim).collect();
    let (main, chord) = parts.split_last().expect("split yields at least one part");

    let (mut ctrl, mut alt, mut cmd) = (false, false, false);
    for part in chord {
        let is = |canonical: &str| part.eq_ignore_ascii_case(canonical);
        if is("Ctrl") || is("Control") {
            ctrl = true;
        } else if is("Alt") || is("Option") {
            alt = true;
        } else if is("Cmd") || is("Command") {
            cmd = true;
        } else if is("Shift") {
            return Err(SHIFT_RESERVED.to_string());
        } else {
            return Err(format!(
                "\"{part}\" is not a chord modifier; use Ctrl, Alt or Cmd"
            ));
        }
    }

    match (chord.is_empty(), resolve(main)?) {
        (_, Main::FKey(key)) => Ok(Hotkey::Key {
            ctrl,
            alt,
            cmd,
            key,
        }),
        (false, Main::Typed(key)) => Ok(Hotkey::Key {
            ctrl,
            alt,
            cmd,
            key,
        }),
        (true, Main::Modifier(key)) => Ok(Hotkey::Modifier(key)),
        (true, Main::Typed(_)) => Err(format!(
            "\"{main}\" alone fires on ordinary typing, and rdev cannot \
             suppress the keystroke; add Ctrl, Alt or Cmd"
        )),
        (false, Main::Modifier(_)) => Err(format!(
            "\"{main}\" is a modifier, and a chord ends in a key"
        )),
    }
}

impl From<Hotkey> for String {
    fn from(hotkey: Hotkey) -> String {
        hotkey.to_string()
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Hotkey::Key {
                ctrl,
                alt,
                cmd,
                key,
            } => {
                if *ctrl {
                    write!(f, "Ctrl+")?;
                }
                if *alt {
                    write!(f, "Alt+")?;
                }
                if *cmd {
                    write!(f, "Cmd+")?;
                }
                if let Some(position) = F_KEYS.iter().position(|k| k == key) {
                    return write!(f, "F{}", position + 1);
                }
                if let Some((c, _)) = TYPED_KEYS.iter().find(|(_, k)| k == key) {
                    return write!(f, "{c}");
                }
                write!(f, "{key:?}")
            }
            Hotkey::Modifier(key) => {
                let name = MODIFIERS
                    .iter()
                    .find(|modifier| modifier.key == *key)
                    .map_or("?", |modifier| modifier.name);
                write!(f, "{name}")
            }
        }
    }
}

/// What one keyboard event asks the daemon to do. `Toggle` is resolved by
/// `DaemonState::record_toggle`, where the recording modes live: the tracker
/// deciding start-versus-stop itself would race a mode change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Start(TranscribeTarget),
    Toggle(TranscribeTarget),
    Stop,
    Cancel,
}

/// Free of I/O, so every rule is testable without a keyboard.
pub struct HotkeyTracker {
    hotkey: Hotkey,
    mode: HotkeyMode,
    shift: bool,
    // Left and right copies of each chord modifier, tracked apart: one bool
    // per pair would read a released twin as the whole pair released
    ctrl: (bool, bool),
    alt: (bool, bool),
    cmd: (bool, bool),
    // Held keys auto-repeat their press; unguarded, a press would fire again
    held: bool,
    // This hold produced a Start, so its release owes a Stop
    started: bool,
    // Another key or a click arrived while a lone-modifier hotkey was held
    combo: bool,
}

impl HotkeyTracker {
    pub fn new(hotkey: Hotkey, mode: HotkeyMode) -> Self {
        Self {
            hotkey,
            mode,
            shift: false,
            ctrl: (false, false),
            alt: (false, false),
            cmd: (false, false),
            held: false,
            started: false,
            combo: false,
        }
    }

    pub fn on_event(&mut self, event: &EventType) -> Option<HotkeyAction> {
        let (key, pressed) = match event {
            EventType::KeyPress(key) => (*key, true),
            EventType::KeyRelease(key) => (*key, false),
            EventType::ButtonPress(_) => {
                // Option+click is a modifier gesture like Option+E
                if self.held {
                    self.combo = true;
                }
                return None;
            }
            _ => return None,
        };
        match key {
            // Shift is sampled at the action instant, so dictation takes the
            // unmodified key: a near simultaneous press cannot misroute it.
            // It routes to the mailbox, so it never counts as a combo.
            Key::ShiftLeft | Key::ShiftRight => {
                self.shift = pressed;
                return None;
            }
            Key::ControlLeft => self.ctrl.0 = pressed,
            Key::ControlRight => self.ctrl.1 = pressed,
            Key::Alt => self.alt.0 = pressed,
            Key::AltGr => self.alt.1 = pressed,
            Key::MetaLeft => self.cmd.0 = pressed,
            Key::MetaRight => self.cmd.1 = pressed,
            _ => {}
        }
        match self.hotkey {
            Hotkey::Key { key: main, .. } => self.on_key(main, key, pressed),
            Hotkey::Modifier(main) => self.on_modifier(main, key, pressed),
        }
    }

    fn on_key(&mut self, main: Key, key: Key, pressed: bool) -> Option<HotkeyAction> {
        if key != main {
            return None;
        }
        if !pressed {
            self.held = false;
            if self.started {
                self.started = false;
                return Some(HotkeyAction::Stop);
            }
            return None;
        }
        if self.held {
            return None;
        }
        self.held = true;
        if !self.chord_down() {
            // The bare key is ordinary typing; it neither starts nor stops
            return None;
        }
        match self.mode {
            HotkeyMode::Hold => {
                self.started = true;
                Some(HotkeyAction::Start(self.target()))
            }
            HotkeyMode::Toggle => Some(HotkeyAction::Toggle(self.target())),
        }
    }

    fn on_modifier(&mut self, main: Key, key: Key, pressed: bool) -> Option<HotkeyAction> {
        if key != main {
            if pressed && self.held {
                self.combo = true;
            }
            return None;
        }
        if pressed {
            if self.held {
                return None;
            }
            self.held = true;
            self.combo = false;
            // Toggle waits for the release: the press may yet become a chord
            if matches!(self.mode, HotkeyMode::Hold) {
                self.started = true;
                return Some(HotkeyAction::Start(self.target()));
            }
            return None;
        }
        let was_held = self.held;
        self.held = false;
        self.started = false;
        // A release only completes a press this tracker saw: the listener drops
        // events while the daemon types, so a press can go missing
        if !was_held {
            return None;
        }
        match (self.mode, self.combo) {
            (HotkeyMode::Hold, true) => Some(HotkeyAction::Cancel),
            (HotkeyMode::Hold, false) => Some(HotkeyAction::Stop),
            (HotkeyMode::Toggle, true) => None,
            (HotkeyMode::Toggle, false) => Some(HotkeyAction::Toggle(self.target())),
        }
    }

    fn chord_down(&self) -> bool {
        let Hotkey::Key { ctrl, alt, cmd, .. } = self.hotkey else {
            return true;
        };
        let down = |pair: (bool, bool)| pair.0 || pair.1;
        (!ctrl || down(self.ctrl)) && (!alt || down(self.alt)) && (!cmd || down(self.cmd))
    }

    fn target(&self) -> TranscribeTarget {
        if self.shift {
            TranscribeTarget::Mailbox
        } else {
            TranscribeTarget::Dictate
        }
    }
}

#[cfg(test)]
pub fn hotkey(text: &str) -> Result<Hotkey, String> {
    Hotkey::try_from(text.to_string())
}

#[cfg(test)]
mod parse_tests;

#[cfg(test)]
mod tracker_tests;
