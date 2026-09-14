//! The compositor key binding: which Hyprland config a machine reads, and the
//! block `banshee bind` appends to it.
use std::path::{Path, PathBuf};

use banshee_common::error::BansheeError;

use crate::connect::{Change, Env, apply_plan};

const LUA_BLOCK: &str = r#"
-- Banshee
o.bind("F9", "Banshee: hold to dictate", "banshee record start --dictate")
o.bind("F9", nil, "banshee record stop", { release = true })
o.bind("SHIFT + F9", "Banshee: hold to record", "banshee record start")
o.bind("SHIFT + F9", nil, "banshee record stop", { release = true })
"#;

const CONF_BLOCK: &str = r#"
# Banshee
bind  = , F9, exec, banshee record start --dictate
bindr = , F9, exec, banshee record stop
bind  = SHIFT, F9, exec, banshee record start
bindr = SHIFT, F9, exec, banshee record stop
"#;

/// The file Hyprland reads on this machine, and the syntax it takes.
struct Layout {
    path: PathBuf,
    block: &'static str,
    marker: &'static str,
}

impl Layout {
    fn detect(hypr_dir: &Path) -> Result<Layout, BansheeError> {
        let lua = hypr_dir.join("bindings.lua");
        if lua.is_file() {
            // Omarchy
            return Ok(Layout {
                path: lua,
                block: LUA_BLOCK,
                marker: "-- Banshee\n",
            });
        }
        let conf = hypr_dir.join("hyprland.conf");
        if conf.is_file() {
            return Ok(Layout {
                path: conf,
                block: CONF_BLOCK,
                marker: "# Banshee\n",
            });
        }
        Err(BansheeError::Rejected(format!(
            "no Hyprland config under {}: looked for bindings.lua and hyprland.conf",
            hypr_dir.display()
        )))
    }
}

/// The edits that bind the key: the appended block, then a reload. Empty when
/// the block is already there.
pub fn plan(hypr_dir: &Path) -> Result<Vec<Change>, BansheeError> {
    let layout = Layout::detect(hypr_dir)?;
    let before = std::fs::read_to_string(&layout.path)?;
    if before.contains(layout.marker) {
        return Ok(Vec::new());
    }
    let mut after = before.clone();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after.push_str(layout.block);
    Ok(vec![
        Change::WriteFile {
            path: layout.path,
            before: Some(before),
            after,
            executable: false,
        },
        Change::Run {
            argv: vec!["hyprctl".to_string(), "reload".to_string()],
        },
    ])
}

fn hypr_dir(home: &Path) -> PathBuf {
    home.join(".config/hypr")
}

pub fn run(name: Option<crate::args::CompositorName>, yes: bool) -> Result<(), BansheeError> {
    if cfg!(target_os = "macos") {
        return Err(BansheeError::Rejected(
            "on macOS the daemon binds the key itself; change it with: banshee config set audio.hotkey <key>"
                .to_string(),
        ));
    }
    if name.is_none() {
        let home = crate::service::home_dir()?;
        let dir = hypr_dir(&home);
        let layout = Layout::detect(&dir)?;
        println!("{}", layout.block.trim_start_matches('\n'));
        println!(
            "Add it to {}, or run: banshee bind hyprland",
            layout.path.display()
        );
        return Ok(());
    }
    let env = Env::from_machine()?;
    let dir = hypr_dir(&env.home);
    let changes = plan(&dir)?;
    if changes.is_empty() {
        println!("Hyprland is already bound: F9 dictates, Shift+F9 records.");
        return Ok(());
    }
    apply_plan(
        &changes,
        &env.path,
        yes,
        "Hyprland is bound: hold F9 and speak.",
    )
}

#[cfg(test)]
mod tests;
