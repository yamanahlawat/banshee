//! The `banshee` command on PATH. A cask writes its own wrapper there, and a
//! shell installer places its own copy, so both routes arrive with the command
//! already working. An app bundle unpacked by hand places neither.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

// The only directory this writes to. It is first in `/etc/paths`, so a link
// there answers in every login shell. Homebrew's prefix is left alone. The
// `banshee` it holds is a wrapper that offers to clear the quarantine flag,
// and a link would take that away.
const TARGET: &str = "/usr/local/bin";

const NAME: &str = "banshee";

/// What `ensure` found, or did.
#[derive(Debug, PartialEq, Eq)]
pub enum Linked {
    /// A `banshee` already answers, so nothing was written.
    Already(PathBuf),
    Made(PathBuf),
    /// Nothing was written. The string is the command that writes it.
    Advised(String),
}

/// `None` when this copy is not installed.
pub fn ensure(exe: &Path, path: impl FnOnce() -> OsString) -> Option<Linked> {
    // A build tree would get a link to a binary that moves, and an uninstall
    // from the installed copy would leave it.
    crate::uninstall::bundle_of(exe)?;
    Some(ensure_in(exe, path, Path::new(TARGET)))
}

fn ensure_in(exe: &Path, path: impl FnOnce() -> OsString, target: &Path) -> Linked {
    // A link of ours answers with two filesystem calls. Reading the PATH asks
    // a login shell, which blocks for up to `SHELL_WAIT`, so it comes last.
    if let Some(link) = placed_in(exe, target) {
        return Linked::Already(link);
    }
    if let Some(found) = crate::status::resolve(NAME, &path()) {
        return Linked::Already(found);
    }
    let link = target.join(NAME);
    // The write decides, not the PATH in hand. A service manager hands the
    // daemon four directories, and that says nothing about where a terminal
    // looks, so a PATH without the target would refuse a link that works.
    match place(exe, &link) {
        Ok(()) => Linked::Made(link),
        Err(_) => Linked::Advised(advice(exe, target, &link)),
    }
}

// A clean macOS has no `/usr/local/bin`, and its parent belongs to `root`, so
// the directory is part of what the person has to make. No `-f`, so a file
// that is not ours stays and `ln` says why.
fn advice(exe: &Path, target: &Path, link: &Path) -> String {
    let linking = format!("sudo ln -s {} {}", shell_word(exe), shell_word(link));
    if target.is_dir() {
        linking
    } else {
        format!("sudo mkdir -p {} && {linking}", shell_word(target))
    }
}

fn shell_word(path: &Path) -> String {
    crate::tell::quoted(&path.display().to_string())
}

fn place(exe: &Path, link: &Path) -> std::io::Result<()> {
    // A link left by a deleted install holds the name and answers nothing.
    if is_link(link) {
        std::fs::remove_file(link)?;
    }
    std::os::unix::fs::symlink(exe, link)
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// The link this install owns, which `uninstall` removes with the rest. A
/// wrapper, or a binary someone else placed, is no link of ours and stays.
pub fn placed(exe: Option<&Path>) -> Option<PathBuf> {
    placed_in(exe?, Path::new(TARGET))
}

fn placed_in(exe: &Path, target: &Path) -> Option<PathBuf> {
    let link = target.join(NAME);
    if !is_link(&link) {
        return None;
    }
    // The name that reached this command is usually the link itself, so both
    // sides are followed before they are compared.
    let installed_in = crate::uninstall::bundle_of(exe)?;
    let points_at = std::fs::canonicalize(&link).ok()?;
    points_at.starts_with(installed_in).then_some(link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        crate::test_support::scratch(&format!("on-path-{name}"))
    }

    /// An installed copy: the command inside an app bundle, as every macOS
    /// route lays it out.
    fn installed(dir: &Path) -> PathBuf {
        let inside = dir.join("Banshee.app/Contents/MacOS");
        std::fs::create_dir_all(&inside).unwrap();
        let exe = inside.join(NAME);
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    #[test]
    fn a_copy_that_is_not_installed_links_nothing() {
        let dir = scratch("loose");
        let loose = dir.join(NAME);
        std::fs::write(&loose, "#!/bin/sh\n").unwrap();
        assert_eq!(ensure(&loose, || OsString::from("/usr/bin")), None);
    }

    #[test]
    fn a_command_that_already_answers_is_left_alone() {
        let dir = scratch("already");
        let exe = installed(&dir);
        let reachable = exe.parent().unwrap().as_os_str().to_os_string();
        assert_eq!(
            ensure_in(&exe, || reachable, Path::new("/nowhere")),
            Linked::Already(exe)
        );
    }

    #[test]
    fn a_link_lands_in_the_target_directory() {
        let install = scratch("made-install");
        let target = scratch("made-target");
        let exe = installed(&install);

        let made = ensure_in(&exe, OsString::new, &target);
        assert_eq!(made, Linked::Made(target.join(NAME)));
        assert_eq!(std::fs::read_link(target.join(NAME)).unwrap(), exe);
    }

    #[test]
    fn a_dangling_link_from_a_deleted_install_is_replaced() {
        let install = scratch("dangling-install");
        let target = scratch("dangling-target");
        let exe = installed(&install);
        std::os::unix::fs::symlink(install.join("gone"), target.join(NAME)).unwrap();

        assert_eq!(
            ensure_in(&exe, OsString::new, &target),
            Linked::Made(target.join(NAME))
        );
        assert_eq!(std::fs::read_link(target.join(NAME)).unwrap(), exe);
    }

    #[test]
    fn an_unwritable_target_is_advised_never_written() {
        let install = scratch("unwritable-install");
        let target = scratch("unwritable-target");
        let exe = installed(&install);
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o555)).unwrap();

        let advised = ensure_in(&exe, OsString::new, &target);
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

        let Linked::Advised(command) = advised else {
            panic!("a directory that refuses a write takes no link");
        };
        assert!(command.starts_with("sudo ln -s "), "{command}");
        assert!(
            !command.contains("mkdir"),
            "the directory is there, so making it is noise: {command}"
        );
        assert!(!target.join(NAME).exists(), "nothing was written");
    }

    #[test]
    fn a_missing_target_is_advised_with_the_directory_made() {
        let install = scratch("mkdir-install");
        let exe = installed(&install);
        let target = install.join("absent");

        let Linked::Advised(command) = ensure_in(&exe, OsString::new, &target) else {
            panic!("a directory that is not there takes no link");
        };
        assert!(command.starts_with("sudo mkdir -p "), "{command}");
        assert!(command.contains(" && sudo ln -s "), "{command}");
    }

    // The tarball unpacks wherever the person put it, and a Downloads folder
    // holds spaces.
    #[test]
    fn the_advice_quotes_a_path_that_holds_a_space() {
        let exe = Path::new("/Users/someone/Banshee 0.15/Banshee.app/Contents/MacOS/banshee");
        let target = Path::new("/usr/local/bin");
        let command = advice(exe, target, &target.join(NAME));
        assert!(
            command.contains("'/Users/someone/Banshee 0.15/"),
            "{command}"
        );
    }

    // `current_exe` answers the name that ran, which is the link itself once
    // the link exists, so an uninstall through it has to recognise its own.
    #[test]
    fn uninstall_takes_the_link_even_when_the_link_is_what_ran() {
        let install = scratch("through-install");
        let target = scratch("through-target");
        let exe = installed(&install);
        let link = target.join(NAME);
        std::os::unix::fs::symlink(&exe, &link).unwrap();

        assert_eq!(placed_in(&link, &target), Some(link));
    }

    #[test]
    fn uninstall_takes_a_link_into_this_install() {
        let install = scratch("owned-install");
        let target = scratch("owned-target");
        let exe = installed(&install);
        std::os::unix::fs::symlink(&exe, target.join(NAME)).unwrap();

        assert_eq!(placed_in(&exe, &target), Some(target.join(NAME)));
    }

    #[test]
    fn uninstall_leaves_a_wrapper_it_did_not_write() {
        let install = scratch("wrapper-install");
        let target = scratch("wrapper-target");
        let exe = installed(&install);
        // What the cask writes: a file, not a link, and not ours to remove.
        std::fs::write(target.join(NAME), "#!/bin/sh\nexec ...\n").unwrap();

        assert_eq!(placed_in(&exe, &target), None);
    }

    #[test]
    fn uninstall_leaves_a_link_into_another_install() {
        let install = scratch("other-install");
        let target = scratch("other-target");
        let exe = installed(&install);
        let elsewhere = scratch("other-elsewhere");
        std::os::unix::fs::symlink(installed(&elsewhere), target.join(NAME)).unwrap();

        assert_eq!(placed_in(&exe, &target), None);
    }
}
