// A TCC grant applies only to processes started after it lands, so the daemon
// has to restart to pick one up. No equivalent outside macOS.

use banshee_common::error::BansheeError;
use banshee_common::{Blocker, BlockerKind};

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
pub enum Access {
    Granted,
    Denied,
    Undetermined,
}

#[cfg(target_os = "macos")]
impl Access {
    /// The word this answer takes on the wire, so a rename is a protocol change.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Granted => "granted",
            Access::Denied => "denied",
            Access::Undetermined => "undetermined",
        }
    }

    pub fn from_wire(word: &str) -> Option<Access> {
        [Access::Granted, Access::Denied, Access::Undetermined]
            .into_iter()
            .find(|access| access.as_str() == word)
    }
}

#[cfg(target_os = "macos")]
pub struct Grant {
    /// Stable across renames of `name`, so a client can switch on it.
    pub id: &'static str,
    pub name: &'static str,
    /// The `revealElementKeyName` macOS publishes for this grant's pane, from
    /// `Security.prefPane/Contents/Resources/PrivacyTCCServices.plist`.
    pub anchor: &'static str,
    pub consequence: &'static str,
    pub fix: &'static str,
}

/// The only grant Banshee needs. Input Monitoring is absent on purpose:
/// Accessibility alone carries the event tap, measured, and nobody can grant one
/// macOS was never asked about.
#[cfg(target_os = "macos")]
pub const ACCESSIBILITY: Grant = Grant {
    id: "accessibility",
    name: "Accessibility",
    anchor: "Privacy_Accessibility",
    consequence: "dictation cannot type and the hotkey stays inert",
    fix: "grant it in System Settings > Privacy & Security > Accessibility. \
If Banshee is already listed and switched on, remove it with the minus button and add it back",
};

/// TCC credits this read to the process macOS holds responsible, so from a CLI
/// arm it answers for the terminal. Every caller runs inside the daemon, and
/// nothing but review keeps it so.
#[cfg(target_os = "macos")]
pub fn accessibility_granted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    // SAFETY: a plain call with no arguments into a system framework. C booleans
    // are unsigned char, so u8 keeps the call sound.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Asking registers the process with TCC and draws the prompt; a read does
/// neither. The daemon asks, never the window: the grant attaches to whoever
/// asks, and the daemon owns the event tap. The dialog is macOS's own, so this
/// returns before a person answers.
pub fn ask_for_accessibility() {
    #[cfg(target_os = "macos")]
    {
        use core_foundation::base::TCFType;
        use core_foundation::boolean::CFBoolean;
        use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
        use core_foundation::string::{CFString, CFStringRef};

        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            static kAXTrustedCheckOptionPrompt: CFStringRef;
            fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
        }
        // SAFETY: the constant is a CFString the framework owns for the life of the
        // process, and `get_rule` retains it rather than taking that ownership.
        let prompt = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
        let options = CFDictionary::from_CFType_pairs(&[(
            prompt.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        // SAFETY: `options` is a live CFDictionary for the whole call, and the call
        // reads it without keeping it. Called for the prompt; the answer is the read
        // `accessibility_granted` makes.
        unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) };
    }
}

/// Whether key events reach this process, not the Input Monitoring grant:
/// measured from the daemon with no Input Monitoring entry, this answers granted
/// while Accessibility is granted and denied once it is revoked.
#[cfg(target_os = "macos")]
pub fn key_presses_reach_us() -> Access {
    // IOHIDRequestType, in header order: PostEvent is 0, ListenEvent 1
    const LISTEN_EVENT: i32 = 1;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOHIDCheckAccess(request: i32) -> i32;
    }
    // SAFETY: a call by value into a system framework, with no pointer to keep
    // valid. IOHIDAccessType, in header order.
    match unsafe { IOHIDCheckAccess(LISTEN_EVENT) } {
        0 => Access::Granted,
        1 => Access::Denied,
        _ => Access::Undetermined,
    }
}

/// The microphone has a pane but no `Grant`: recording reports its own failure.
#[cfg(target_os = "macos")]
pub fn pane_anchor(id: &str) -> Option<&'static str> {
    if id == ACCESSIBILITY.id {
        return Some(ACCESSIBILITY.anchor);
    }
    if id == "microphone" {
        return Some("Privacy_Microphone");
    }
    None
}

#[cfg(target_os = "macos")]
const OPEN: &str = "/usr/bin/open";

#[cfg(target_os = "macos")]
fn open_anchor(anchor: &str) -> Result<(), BansheeError> {
    let status = std::process::Command::new(OPEN)
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{anchor}"
        ))
        .status()
        .map_err(|error| BansheeError::Other(format!("could not run open: {error}")))?;
    if !status.success() {
        return Err(BansheeError::Other(format!(
            "open refused the {anchor} pane"
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn open_pane(id: &str) -> Result<(), BansheeError> {
    let anchor = pane_anchor(id)
        .ok_or_else(|| BansheeError::Rejected(format!("'{id}' is not a settings pane")))?;
    open_anchor(anchor)
}

#[cfg(not(target_os = "macos"))]
pub fn open_pane(id: &str) -> Result<(), BansheeError> {
    Err(BansheeError::Rejected(format!(
        "'{id}': settings panes are a macOS feature"
    )))
}

#[cfg(target_os = "macos")]
pub fn blockers() -> Vec<Blocker> {
    if accessibility_granted() {
        return Vec::new();
    }
    vec![blocker(&ACCESSIBILITY)]
}

/// Split from the live read, which answers only on the machine it runs on.
#[cfg(target_os = "macos")]
fn blocker(grant: &Grant) -> Blocker {
    Blocker {
        role: None,
        remedy: Some(banshee_common::Remedy::Grant),
        kind: BlockerKind::Permission,
        id: grant.id.to_string(),
        name: grant.name.to_string(),
        consequence: grant.consequence.to_string(),
        fix: grant.fix.to_string(),
        // A grant is a switch in System Settings, not a command.
        command: None,
    }
}

// X11 types through enigo directly, so only a Wayland session needs a typer.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn blockers() -> Vec<Blocker> {
    let path = crate::connect::resolved_path();
    blockers_for(
        crate::dictation::is_wayland(),
        crate::dictation::resolve_wayland_typer(&path).is_some(),
    )
}

// Neither macOS nor a Wayland-capable unix: no client of this build ever sees
// a typer blocker.
#[cfg(not(any(target_os = "macos", unix)))]
pub fn blockers() -> Vec<Blocker> {
    Vec::new()
}

/// Split from the live reads, which answer only on the machine and session
/// this runs in.
#[cfg(all(unix, not(target_os = "macos")))]
fn blockers_for(wayland: bool, typer_resolved: bool) -> Vec<Blocker> {
    if wayland && !typer_resolved {
        vec![wayland_typer_blocker()]
    } else {
        Vec::new()
    }
}

// No client reads `command` on a pipeline blocker, so the fix carries the
// tool names in prose instead of a distribution-specific install line.
#[cfg(all(unix, not(target_os = "macos")))]
fn wayland_typer_blocker() -> Blocker {
    Blocker {
        role: None,
        remedy: None,
        kind: BlockerKind::Pipeline,
        id: "wayland_typer".to_string(),
        name: "Typing tool".to_string(),
        consequence: "dictation cannot type anywhere".to_string(),
        fix: "install 'wtype' (or 'ydotool')".to_string(),
        command: None,
    }
}

/// States what macOS asks for and checks nothing. It cannot read the grant (see
/// `accessibility_granted`), nor ask the daemon `banshee start` just launched,
/// which answers nothing until its models load. It opens no pane either: the
/// daemon's own prompt does that, and two routes to one switch fight for focus.
pub fn grant_note() {
    #[cfg(target_os = "macos")]
    {
        println!();
        println!(
            "{} is the one grant Banshee needs: without it, {}.",
            ACCESSIBILITY.name, ACCESSIBILITY.consequence
        );
        println!(
            "If macOS asks for it, approve the prompt and the daemon restarts itself as it lands. \
             Otherwise {}.",
            ACCESSIBILITY.fix
        );
        println!("`banshee status` says whether it is granted.");
    }
}

/// Whether the daemon goes now. It has to go for the grant to reach it, and a
/// download in flight dies with the process and starts that file over, while
/// waiting for one costs no more than an inert hotkey for a few minutes.
#[cfg(target_os = "macos")]
pub fn leaves_for_the_grant(granted: bool, downloading: bool) -> bool {
    granted && !downloading
}

/// Leaves once the grant lands, so the supervisor starts us again with it:
/// launchd re-runs on nonzero exit (`KeepAlive`/`SuccessfulExit`), systemd on
/// `Restart=on-failure`. `leave` ends the daemon the way a signal does, rather
/// than by `exit`, which would leave the socket file behind for the next run
/// to call a crash.
pub fn restart_when_granted(
    downloading: impl Fn() -> bool + Send + 'static,
    leave: impl FnOnce() + Send + 'static,
) {
    #[cfg(target_os = "macos")]
    {
        use std::thread;
        use std::time::Duration;

        const POLL: Duration = Duration::from_secs(2);

        if accessibility_granted() {
            return;
        }
        log::warn!("the Accessibility grant is missing; the hotkey is inert until it lands");
        thread::spawn(move || {
            loop {
                thread::sleep(POLL);
                if leaves_for_the_grant(accessibility_granted(), downloading()) {
                    log::info!("the Accessibility grant landed, restarting to pick it up");
                    leave();
                    return;
                }
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (downloading, leave);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    // The grant reaches only a process started after it lands, so the daemon
    // has to go. A download dies with it and the next run starts that file
    // over, while waiting costs no more than an inert hotkey.
    #[test]
    fn the_daemon_leaves_for_a_grant_but_not_through_a_download() {
        assert!(leaves_for_the_grant(true, false));
        assert!(!leaves_for_the_grant(true, true), "a download outranks it");
        assert!(!leaves_for_the_grant(false, false));
        assert!(!leaves_for_the_grant(false, true));
    }

    #[test]
    fn each_pane_id_has_an_anchor_and_an_unknown_one_is_refused() {
        assert_eq!(pane_anchor("accessibility"), Some("Privacy_Accessibility"));
        assert_eq!(pane_anchor("microphone"), Some("Privacy_Microphone"));
        assert_eq!(pane_anchor("everything"), None);
    }

    #[test]
    fn every_access_answer_survives_the_wire() {
        for access in [Access::Granted, Access::Denied, Access::Undetermined] {
            let word = access.as_str();
            assert_eq!(
                Access::from_wire(word).map(Access::as_str),
                Some(word),
                "'{word}' did not read back as itself"
            );
        }
        assert_eq!(Access::from_wire("moonbeam").map(Access::as_str), None);
    }

    #[test]
    fn input_monitoring_is_not_a_pane_a_client_can_open() {
        assert_eq!(pane_anchor("input_monitoring"), None);
    }

    // A row can be listed and switched on while macOS no longer matches it to
    // this build, so the grant does nothing and no prompt appears. Naming only
    // the switch sends the reader back to a switch that is already on.
    #[test]
    fn the_grant_fix_names_the_repair_for_an_entry_that_is_already_there() {
        let fix = blocker(&ACCESSIBILITY).fix;
        assert!(fix.contains("System Settings"), "{fix}");
        assert!(
            fix.contains("remove it") && fix.contains("add it back"),
            "the switch alone is not the repair: {fix}"
        );
    }

    // The one blocker the daemon sends for a grant. A client routes on `remedy`
    // and opens the pane by `id`, and offers a command when there is one.
    #[test]
    fn a_grant_reaches_a_client_as_a_switch_to_open_and_never_a_command() {
        let blocker = blocker(&ACCESSIBILITY);

        assert_eq!(blocker.kind, BlockerKind::Permission);
        assert_eq!(blocker.remedy, Some(banshee_common::Remedy::Grant));
        assert_eq!(blocker.command, None);
        assert_eq!(pane_anchor(&blocker.id), Some(ACCESSIBILITY.anchor));
        assert_eq!(blocker.name, "Accessibility");
        assert!(
            blocker.consequence.contains("hotkey"),
            "the consequence says what breaks: {}",
            blocker.consequence
        );
        assert!(
            blocker.fix.contains("System Settings"),
            "the fix is a settings path, not a command: {}",
            blocker.fix
        );
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod wayland_typer_tests {
    use super::*;

    #[test]
    fn wayland_with_no_typer_gets_one_pipeline_blocker() {
        let found = blockers_for(true, false);
        assert_eq!(found.len(), 1, "a missing typer on wayland must block");
        assert_eq!(found[0].kind, BlockerKind::Pipeline);
        assert_eq!(found[0].id, "wayland_typer");
        assert_eq!(found[0].command, None);
        assert!(
            found[0].fix.contains("wtype") && found[0].fix.contains("ydotool"),
            "the fix must name both tools: {}",
            found[0].fix
        );
    }

    #[test]
    fn wayland_with_a_resolved_typer_has_no_blocker() {
        assert!(blockers_for(true, true).is_empty());
    }

    #[test]
    fn x11_never_blocks_on_a_typer() {
        assert!(blockers_for(false, false).is_empty());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tool_tests {
    #[test]
    fn the_settings_opener_is_where_a_supervised_daemon_finds_it() {
        crate::test_support::tool_is_installed(super::OPEN);
    }
}
