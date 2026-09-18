//! What `banshee uninstall` may remove, and what it must leave to the tool that
//! put it there. Homebrew keeps its own records of what it placed; deleting
//! those files behind its back leaves the records pointing at nothing.

use std::path::{Path, PathBuf};

/// Who installed this copy.
#[derive(Debug, PartialEq, Eq)]
pub enum Owner {
    /// Homebrew, with the command that removes it.
    Homebrew(&'static str),
    /// The shell installer, which wrote a receipt naming what it placed.
    Installer,
    /// An app bundle nobody else claims: unpacked from the tarball, or built
    /// from source.
    Bundle,
    /// Nothing to go on. The login entries still go; nothing else is touched.
    Unknown,
}

/// The first of these that is present owns the copy. A cask install has an app
/// bundle too, so Homebrew is asked about first.
pub fn owner(cask: bool, formula: bool, receipt: bool, bundle: bool) -> Owner {
    if cask {
        Owner::Homebrew("brew uninstall --cask banshee")
    } else if formula {
        Owner::Homebrew("brew uninstall --formula banshee")
    } else if receipt {
        Owner::Installer
    } else if bundle {
        Owner::Bundle
    } else {
        Owner::Unknown
    }
}

/// Everything the command will do, worked out before it does any of it, so it
/// can be shown to a person who has not agreed to it yet.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Deleted by this command.
    pub remove: Vec<PathBuf>,
    /// Left to its owner, with the command that does it.
    pub leave_to: Option<&'static str>,
}

/// `software` is what the owner placed: the binaries a receipt names, or the
/// app bundle. `data` is `~/.banshee` when the person asked for it.
pub fn plan(owner: &Owner, software: Vec<PathBuf>, data: Option<PathBuf>) -> Plan {
    let mut remove = match owner {
        // Its files are Homebrew's to remove, but the data directory is nobody's
        // but the person's, and `--zap` is easy to miss.
        Owner::Homebrew(_) => Vec::new(),
        Owner::Installer | Owner::Bundle => software,
        Owner::Unknown => Vec::new(),
    };
    remove.extend(data);
    Plan {
        remove,
        leave_to: match owner {
            Owner::Homebrew(command) => Some(command),
            _ => None,
        },
    }
}

/// The receipt the shell installer writes, which names the binaries it placed.
pub fn receipt_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))?;
    Some(config.join("banshee").join("banshee-receipt.json"))
}

/// The binaries a receipt names, under the prefix it recorded.
pub fn receipt_binaries(receipt: &str) -> Vec<PathBuf> {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(receipt) else {
        return Vec::new();
    };
    let Some(prefix) = parsed["install_prefix"].as_str() else {
        return Vec::new();
    };
    let dir = binaries_dir(
        prefix,
        parsed["install_layout"].as_str().unwrap_or_default(),
    );
    parsed["binaries"]
        .as_array()
        .map(|binaries| {
            binaries
                .iter()
                .filter_map(|binary| binary.as_str())
                .map(|binary| dir.join(binary))
                .collect()
        })
        .unwrap_or_default()
}

/// Where under the prefix the installer put the binaries. A cargo home and the
/// hierarchical layout both keep them in `bin`; the rest are the prefix itself.
fn binaries_dir(prefix: &str, layout: &str) -> PathBuf {
    match layout {
        "cargo-home" | "hierarchical" => Path::new(prefix).join("bin"),
        _ => PathBuf::from(prefix),
    }
}

/// The `.app` this binary runs from, if it runs from one at all. What a person
/// runs is the link on their PATH, so the link is followed first: the bundle is
/// where the binary lives, not where the name that reached it lives.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    real.ancestors()
        .find(|path| path.extension().is_some_and(|kind| kind == "app"))
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A cask install has an app bundle as well, so the order of these questions
    // is what keeps the answer from being "delete Homebrew's files".
    #[test]
    fn homebrew_owns_a_copy_it_placed_even_though_a_bundle_is_there() {
        assert_eq!(
            owner(true, false, false, true),
            Owner::Homebrew("brew uninstall --cask banshee")
        );
        assert_eq!(
            owner(false, true, false, false),
            Owner::Homebrew("brew uninstall --formula banshee")
        );
    }

    #[test]
    fn a_receipt_names_the_shell_installer_and_a_lone_bundle_names_itself() {
        assert_eq!(owner(false, false, true, false), Owner::Installer);
        assert_eq!(owner(false, false, false, true), Owner::Bundle);
        assert_eq!(owner(false, false, false, false), Owner::Unknown);
    }

    // Removing what Homebrew placed leaves its records pointing at nothing, so
    // this command removes none of it. The data directory is not Homebrew's.
    #[test]
    fn nothing_homebrew_placed_is_removed_here() {
        let plan = plan(
            &Owner::Homebrew("brew uninstall --cask banshee"),
            vec![PathBuf::from("/Applications/Banshee.app")],
            Some(PathBuf::from("/Users/someone/.banshee")),
        );
        assert_eq!(plan.remove, vec![PathBuf::from("/Users/someone/.banshee")]);
        assert_eq!(plan.leave_to, Some("brew uninstall --cask banshee"));
    }

    #[test]
    fn what_the_installer_placed_is_removed_here() {
        let binaries = vec![PathBuf::from("/home/someone/.cargo/bin/banshee")];
        let plan = plan(&Owner::Installer, binaries.clone(), None);
        assert_eq!(plan.remove, binaries);
        assert_eq!(plan.leave_to, None, "nobody else has to be asked");
    }

    #[test]
    fn data_goes_only_when_it_is_given() {
        let without = plan(&Owner::Bundle, Vec::new(), None);
        assert!(without.remove.is_empty());

        let with = plan(
            &Owner::Bundle,
            Vec::new(),
            Some(PathBuf::from("/Users/someone/.banshee")),
        );
        assert_eq!(with.remove, vec![PathBuf::from("/Users/someone/.banshee")]);
    }

    // The layout says where under the prefix they went. A cargo home keeps them
    // in `bin`, and reading the prefix alone names files that are not there.
    #[test]
    fn a_receipt_names_its_binaries_where_its_layout_put_them() {
        let cargo_home = r#"{"binaries":["banshee","banshee-tray"],
            "install_layout":"cargo-home",
            "install_prefix":"/home/someone/.cargo","version":"0.14.0"}"#;
        assert_eq!(
            receipt_binaries(cargo_home),
            vec![
                PathBuf::from("/home/someone/.cargo/bin/banshee"),
                PathBuf::from("/home/someone/.cargo/bin/banshee-tray"),
            ]
        );

        let flat = r#"{"binaries":["banshee"],"install_layout":"flat",
            "install_prefix":"/home/someone/.local/bin","version":"0.14.0"}"#;
        assert_eq!(
            receipt_binaries(flat),
            vec![PathBuf::from("/home/someone/.local/bin/banshee")]
        );
    }

    #[test]
    fn a_receipt_that_says_nothing_useful_names_no_files_to_delete() {
        assert!(receipt_binaries("not json").is_empty());
        assert!(receipt_binaries(r#"{"version":"0.14.0"}"#).is_empty());
    }

    // What a person runs is the link on their PATH, not the binary inside the
    // bundle. A link that is not followed finds no app, and the command then
    // offers to remove nothing at all.
    #[test]
    fn a_link_on_the_path_still_finds_the_app_it_points_into() {
        let root = std::env::temp_dir().join(format!("banshee-link-{}", std::process::id()));
        let inside = root.join("Probe.app/Contents/MacOS");
        std::fs::create_dir_all(&inside).expect("a bundle to point at");
        let real = inside.join("banshee");
        std::fs::write(&real, b"").expect("the binary");
        let link = root.join("banshee");
        std::os::unix::fs::symlink(&real, &link).expect("the link");

        // `/tmp` is itself a link on macOS, so the answer is compared against
        // the resolved root rather than the one the test wrote to.
        let resolved = std::fs::canonicalize(&root).expect("the root resolves");
        assert_eq!(
            bundle_of(&link),
            Some(resolved.join("Probe.app")),
            "the link was not followed"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_bundle_is_the_app_a_binary_runs_from() {
        assert_eq!(
            bundle_of(Path::new(
                "/Applications/Banshee.app/Contents/MacOS/banshee"
            )),
            Some(PathBuf::from("/Applications/Banshee.app"))
        );
        assert_eq!(
            bundle_of(Path::new("/home/someone/.cargo/bin/banshee")),
            None
        );
    }
}
