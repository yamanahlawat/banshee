// A TCC grant applies only to processes started after it lands, so the daemon
// has to restart to pick one up. No equivalent outside macOS.

use banshee_common::Blocker;
#[cfg(target_os = "macos")]
use banshee_common::BlockerKind;
use banshee_common::error::BansheeError;

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
    fix: "grant it in System Settings > Privacy & Security > Accessibility",
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
    // C booleans are unsigned char, so u8 keeps the call sound
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
        let prompt = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
        let options = CFDictionary::from_CFType_pairs(&[(
            prompt.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        // Called for the prompt; the answer is the read `accessibility_granted` makes.
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
    // IOHIDAccessType, in header order
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
fn open_anchor(anchor: &str) -> Result<(), BansheeError> {
    let status = std::process::Command::new("open")
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

#[cfg(not(target_os = "macos"))]
pub fn blockers() -> Vec<Blocker> {
    Vec::new()
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

/// Exit once the grant lands, so the supervisor restarts us with it: launchd
/// re-runs on nonzero exit (`KeepAlive`/`SuccessfulExit`), systemd on
/// `Restart=on-failure`.
pub fn restart_when_granted() {
    #[cfg(target_os = "macos")]
    {
        use std::thread;
        use std::time::Duration;

        const POLL: Duration = Duration::from_secs(2);

        if accessibility_granted() {
            return;
        }
        eprintln!("the Accessibility grant is missing; the hotkey is inert until it lands");
        thread::spawn(|| {
            loop {
                thread::sleep(POLL);
                if accessibility_granted() {
                    eprintln!("the Accessibility grant landed, restarting to pick it up");
                    std::process::exit(1);
                }
            }
        });
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

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
