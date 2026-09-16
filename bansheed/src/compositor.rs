//! The compositor key binding: which Hyprland config a machine reads, and the
//! block `banshee bind` writes to it.
use std::path::{Path, PathBuf};

use banshee_common::error::BansheeError;

use crate::binding::{Hotkey, key_name};
use crate::cli::ask_line;
use crate::config::HotkeyMode;
use crate::connect::{Change, Env, apply, apply_plan, split_between};

const FALLBACK_KEY: Hotkey = Hotkey::Key {
    ctrl: false,
    alt: false,
    cmd: false,
    key: rdev::Key::F9,
};

const BLOCK_START: &str = "BEGIN BANSHEE MANAGED BLOCK";
const BLOCK_END: &str = "END BANSHEE MANAGED BLOCK";

#[derive(Clone, Copy)]
enum Syntax {
    Lua,
    Conf,
}

struct Layout {
    path: PathBuf,
    syntax: Syntax,
}

impl Layout {
    fn detect(hypr_dir: &Path) -> Result<Layout, BansheeError> {
        let lua = hypr_dir.join("bindings.lua");
        if lua.is_file() {
            // Omarchy
            return Ok(Layout {
                path: lua,
                syntax: Syntax::Lua,
            });
        }
        let conf = hypr_dir.join("hyprland.conf");
        if conf.is_file() {
            return Ok(Layout {
                path: conf,
                syntax: Syntax::Conf,
            });
        }
        Err(BansheeError::Rejected(format!(
            "no Hyprland config under {}: looked for bindings.lua and hyprland.conf",
            hypr_dir.display()
        )))
    }

    fn comment(&self) -> &'static str {
        match self.syntax {
            Syntax::Lua => "--",
            Syntax::Conf => "#",
        }
    }

    fn start(&self) -> String {
        format!("{} {BLOCK_START}", self.comment())
    }

    fn end(&self) -> String {
        format!("{} {BLOCK_END}", self.comment())
    }

    fn block(&self, hotkey: Hotkey, mode: HotkeyMode) -> String {
        let (binds, _) = self.binds(hotkey, mode);
        format!("{}\n{}{}\n", self.start(), binds, self.end())
    }

    fn binds(&self, hotkey: Hotkey, mode: HotkeyMode) -> (String, Option<String>) {
        let Hotkey::Key {
            ctrl,
            alt,
            cmd,
            key,
        } = hotkey
        else {
            unreachable!("bindable keeps a lone modifier out of a Hyprland block")
        };
        let name = key_name(key);
        let key = name.as_str();
        let mods: Vec<&str> = [(ctrl, "CTRL"), (alt, "ALT"), (cmd, "SUPER")]
            .into_iter()
            .filter_map(|(held, modifier)| held.then_some(modifier))
            .collect();
        let shifted: Vec<&str> = std::iter::once("SHIFT")
            .chain(mods.iter().copied())
            .collect();
        // SHIFT is taken by the record bind, so the search starts at SUPER.
        let telling: Option<Vec<&str>> = ["SUPER", "CTRL", "ALT"]
            .into_iter()
            .find(|modifier| !mods.contains(modifier))
            .map(|modifier| {
                std::iter::once(modifier)
                    .chain(mods.iter().copied())
                    .collect()
            });
        let tell_chord = telling
            .as_ref()
            .map(|parts| [parts.as_slice(), &[key]].concat().join(" + "));
        let binds = match self.syntax {
            Syntax::Lua => {
                let plain = [mods.as_slice(), &[key]].concat().join(" + ");
                let shift = [shifted.as_slice(), &[key]].concat().join(" + ");
                let tell_hold = tell_chord.as_ref().map_or(String::new(), |chord| {
                    format!(
                        r#"o.bind("{chord}", "Banshee: hold to tell the agent", "banshee record start --tell")
o.bind("{chord}", nil, "banshee record stop", {{ release = true }})
"#
                    )
                });
                let tell_tap = tell_chord.as_ref().map_or(String::new(), |chord| {
                    format!(
                        r#"o.bind("{chord}", "Banshee: tap to tell the agent", "banshee record toggle --tell")
"#
                    )
                });
                match mode {
                    HotkeyMode::Hold => format!(
                        r#"o.bind("{plain}", "Banshee: hold to dictate", "banshee record start --dictate")
o.bind("{plain}", nil, "banshee record stop", {{ release = true }})
o.bind("{shift}", "Banshee: hold to record", "banshee record start")
o.bind("{shift}", nil, "banshee record stop", {{ release = true }})
{tell_hold}"#
                    ),
                    HotkeyMode::Toggle => format!(
                        r#"o.bind("{plain}", "Banshee: tap to dictate", "banshee record toggle --dictate")
o.bind("{shift}", "Banshee: tap to record", "banshee record toggle")
{tell_tap}"#
                    ),
                }
            }
            Syntax::Conf => {
                let plain = mods.join(" ");
                let shift = shifted.join(" ");
                let tell_mods = telling.as_ref().map(|parts| parts.join(" "));
                let tell_hold = tell_mods.as_ref().map_or(String::new(), |chord| {
                    format!(
                        "bind  = {chord}, {key}, exec, banshee record start --tell
bindr = {chord}, {key}, exec, banshee record stop
"
                    )
                });
                let tell_tap = tell_mods.as_ref().map_or(String::new(), |chord| {
                    format!("bind = {chord}, {key}, exec, banshee record toggle --tell\n")
                });
                match mode {
                    HotkeyMode::Hold => format!(
                        "bind  = {plain}, {key}, exec, banshee record start --dictate
bindr = {plain}, {key}, exec, banshee record stop
bind  = {shift}, {key}, exec, banshee record start
bindr = {shift}, {key}, exec, banshee record stop
{tell_hold}"
                    ),
                    HotkeyMode::Toggle => format!(
                        "bind = {plain}, {key}, exec, banshee record toggle --dictate
bind = {shift}, {key}, exec, banshee record toggle
{tell_tap}"
                    ),
                }
            }
        };
        (binds, tell_chord)
    }
}

pub struct Rebind {
    pub path: PathBuf,
    /// Empty when the file already reads that way.
    pub changes: Vec<Change>,
    /// Lines, counted in the file as written, that run `banshee record` outside the markers.
    pub strays: Vec<usize>,
    /// The chord that tells the agent. `None` when the dictate key holds every
    /// modifier and nothing is left to add.
    pub tell: Option<String>,
}

pub fn plan(hypr_dir: &Path, hotkey: Hotkey, mode: HotkeyMode) -> Result<Rebind, BansheeError> {
    let layout = Layout::detect(hypr_dir)?;
    let before = std::fs::read_to_string(&layout.path)?;
    let (start, end) = (layout.start(), layout.end());
    let (binds, tell) = layout.binds(hotkey, mode);
    let after = match split_between(&before, &start, &end) {
        Some((head, _, tail)) => {
            format!("{head}{start}\n{binds}{end}{tail}")
        }
        None if before.contains(BLOCK_START) => {
            return Err(BansheeError::Rejected(format!(
                "{} has {start} with no {end} after it; remove that block, then run: \
                 banshee bind hyprland",
                layout.path.display()
            )));
        }
        None => appended(&before, &layout.block(hotkey, mode)),
    };
    let strays = strays(&after, &start, &end);
    let changes = if after == before {
        Vec::new()
    } else {
        vec![Change::WriteFile {
            path: layout.path.clone(),
            before: Some(before),
            after,
            executable: false,
        }]
    };
    Ok(Rebind {
        path: layout.path,
        changes,
        strays,
        tell,
    })
}

fn appended(before: &str, block: &str) -> String {
    let separator = if before.is_empty() || before.ends_with("\n\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{before}{separator}{block}")
}

fn strays(text: &str, start: &str, end: &str) -> Vec<usize> {
    let mut inside = false;
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim() == start {
            inside = true;
        } else if line.trim() == end {
            inside = false;
        } else if !inside && line.contains("banshee record") {
            lines.push(index + 1);
        }
    }
    lines
}

fn bindable(hotkey: Hotkey) -> Option<Hotkey> {
    match hotkey {
        Hotkey::Modifier(_) => None,
        key => Some(key),
    }
}

fn default_key(saved: Hotkey) -> Hotkey {
    bindable(saved).unwrap_or(FALLBACK_KEY)
}

fn key_from_answer(answer: &str) -> Result<Hotkey, BansheeError> {
    let hotkey = Hotkey::try_from(answer.to_string()).map_err(BansheeError::Rejected)?;
    bindable(hotkey).ok_or_else(|| {
        BansheeError::Rejected(format!(
            "Hyprland fires both binds of {answer} together when you release it, so it records \
             nothing; use an F-key or a chord such as Ctrl+Alt+D"
        ))
    })
}

fn press_word(mode: HotkeyMode) -> &'static str {
    match mode {
        HotkeyMode::Hold => "hold",
        HotkeyMode::Toggle => "tap",
    }
}

fn mode_from_answer(answer: &str) -> Result<HotkeyMode, BansheeError> {
    [HotkeyMode::Hold, HotkeyMode::Toggle]
        .into_iter()
        .find(|mode| answer.eq_ignore_ascii_case(press_word(*mode)))
        .ok_or_else(|| {
            BansheeError::Rejected(format!(
                "answer {} or {}, not '{answer}'",
                press_word(HotkeyMode::Hold),
                press_word(HotkeyMode::Toggle)
            ))
        })
}

fn hypr_dir(home: &Path) -> PathBuf {
    home.join(".config/hypr")
}

/// Binds the key and answers the key and mode it bound, which the caller saves.
pub fn run(
    name: Option<crate::args::CompositorName>,
    yes: bool,
    saved_key: Hotkey,
    saved_mode: HotkeyMode,
) -> Result<(Hotkey, HotkeyMode), BansheeError> {
    if cfg!(target_os = "macos") {
        return Err(BansheeError::Rejected(
            "on macOS the daemon binds the key itself; change it with: banshee config set audio.hotkey <key>"
                .to_string(),
        ));
    }
    let default = default_key(saved_key);
    if name.is_none() {
        let home = crate::service::home_dir()?;
        let dir = hypr_dir(&home);
        let layout = Layout::detect(&dir)?;
        println!("{}", layout.block(default, saved_mode));
        println!(
            "Add it to {}, or run: banshee bind hyprland",
            layout.path.display()
        );
        return Ok((saved_key, saved_mode));
    }
    let (hotkey, mode) = if yes {
        (default, saved_mode)
    } else {
        let hotkey = match ask_line(&format!("Which key? [{default}] "))? {
            Some(answer) => key_from_answer(&answer)?,
            None => default,
        };
        let prompt = format!(
            "Hold {hotkey} while you speak, or tap it to start and stop? hold/tap [{}] ",
            press_word(saved_mode)
        );
        let mode = match ask_line(&prompt)? {
            Some(answer) => mode_from_answer(&answer)?,
            None => saved_mode,
        };
        (hotkey, mode)
    };
    let env = Env::from_machine()?;
    let dir = hypr_dir(&env.home);
    let rebind = plan(&dir, hotkey, mode)?;
    for line in &rebind.strays {
        eprintln!(
            "{}:{line} runs banshee record outside the Banshee block; remove it if the block \
             replaces it",
            rebind.path.display()
        );
    }
    if rebind.changes.is_empty() {
        let tell = match &rebind.tell {
            Some(chord) => format!(", {chord} to tell your agent"),
            None => ", no modifier left to tell your agent".to_string(),
        };
        println!(
            "Hyprland is already bound: {} {hotkey} to dictate, Shift+{hotkey} to record{tell}.",
            press_word(mode)
        );
        return Ok((hotkey, mode));
    }
    let tell = match &rebind.tell {
        Some(chord) => format!(" {chord} tells your agent."),
        None => " Your key holds every modifier, so nothing is left for the tell key.".to_string(),
    };
    let done = match mode {
        HotkeyMode::Hold => format!("Hyprland is bound: hold {hotkey} and speak.{tell}"),
        HotkeyMode::Toggle => {
            format!("Hyprland is bound: tap {hotkey}, speak, and tap it again to stop.{tell}")
        }
    };
    apply_plan(&rebind.changes, &env.path, yes, &done)?;
    let reload = Change::Run {
        argv: vec!["hyprctl".to_string(), "reload".to_string()],
    };
    // The file is written, so a failed reload must not stop the key and mode being saved
    if let Err(error) = apply(&reload, &env.path) {
        eprintln!(
            "{} is written, but hyprctl reload failed: {error}; run hyprctl reload in your \
             Hyprland session",
            rebind.path.display()
        );
    }
    Ok((hotkey, mode))
}

#[cfg(test)]
mod tests;
