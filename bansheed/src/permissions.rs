// The key listener is built at start, so a daemon that started without
// Accessibility restarts once it lands. No equivalent outside macOS.

use std::sync::Arc;

use banshee_common::error::BansheeError;
use banshee_common::{Blocker, BlockerKind};

use crate::state::DaemonState;
#[cfg(target_os = "macos")]
use crate::state::Refusal;

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
    name: "Accessibility is not granted",
    anchor: "Privacy_Accessibility",
    consequence: "dictation cannot type and the hotkey stays inert",
    fix: "grant it in System Settings > Privacy & Security > Accessibility. \
If Banshee is already listed and switched on, remove it with the minus button and add it back",
};

/// Recording asks for this one at its first open, through macOS's own prompt,
/// so only a refusal is a blocker. The fix is a denied grant's; a restricted
/// one carries its own.
#[cfg(target_os = "macos")]
pub const MICROPHONE: Grant = Grant {
    id: "microphone",
    name: "Microphone access is not allowed",
    anchor: "Privacy_Microphone",
    consequence: "dictation and spoken answers fail",
    fix: Refusal::Denied.fix(),
};

/// What macOS refuses about the microphone, read without opening it. `None`
/// when it allows it or has yet to ask. A read from the CLI answers for the
/// terminal, as `accessibility_granted` does.
#[cfg(target_os = "macos")]
pub fn microphone_access() -> Option<Refusal> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_avf_audio::{AVAudioApplication, AVAudioApplicationRecordPermission};

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: &'static AnyObject;
    }
    let device = AnyClass::get(c"AVCaptureDevice")?;
    // The daemon's threads run no run loop to drain anything the call
    // autoreleases.
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: a class method that takes the framework's own media type
        // constant, which lives for the process, and returns an NSInteger.
        let status: isize =
            unsafe { msg_send![device, authorizationStatusForMediaType: AVMediaTypeAudio] };
        // The class does not exist before macOS 14.
        let record = if objc2::available!(macos = 14.0) {
            // SAFETY: plain getters on a class that exists on this macOS.
            let permission = unsafe { AVAudioApplication::sharedInstance().recordPermission() };
            match permission {
                AVAudioApplicationRecordPermission::Undetermined => Some(Record::Undetermined),
                AVAudioApplicationRecordPermission::Denied => Some(Record::Denied),
                AVAudioApplicationRecordPermission::Granted => Some(Record::Granted),
                _ => None,
            }
        } else {
            None
        };
        refusal_from(record, status)
    })
}

/// AVAudioApplication's record permission. It follows the System Settings
/// switch while the process runs; `authorizationStatus` does not.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum Record {
    Undetermined,
    Denied,
    Granted,
}

/// The live record permission decides, and the old status only names a
/// restriction, as the live read has no restricted value. Without a live read
/// (macOS 13) the status stands alone.
#[cfg(target_os = "macos")]
fn refusal_from(record: Option<Record>, status: isize) -> Option<Refusal> {
    let old = refusal_of(status);
    match record {
        Some(Record::Granted) => None,
        _ if old == Some(Refusal::Restricted) => old,
        Some(Record::Denied) => Some(Refusal::Denied),
        Some(Record::Undetermined) => None,
        None => old,
    }
}

/// AVAuthorizationStatus, in header order.
#[cfg(target_os = "macos")]
fn refusal_of(status: isize) -> Option<Refusal> {
    match status {
        1 => Some(Refusal::Restricted),
        2 => Some(Refusal::Denied),
        _ => None,
    }
}

/// Reads the grant into the state, so a move reaches every client.
#[cfg(target_os = "macos")]
pub fn read_microphone(state: &DaemonState) -> Option<Refusal> {
    let refusal = state.microphone_access();
    state.set_microphone_refusal(refusal);
    refusal
}

/// The window server grants an event tap only while the switch is on, and
/// answers afresh each time. Measured on macOS 26.7.1 in the launchd daemon,
/// where `AXIsProcessTrusted`, `CGPreflightPostEventAccess` and
/// `IOHIDCheckAccess` keep their first answer. A listen-only tap can stand on
/// Input Monitoring alone, so this one filters; it watches a rare event and is
/// gone before this returns. TCC credits it to the process macOS holds
/// responsible, so from a CLI arm it answers for the terminal. Every caller
/// runs inside the daemon, and nothing but review keeps it so.
#[cfg(target_os = "macos")]
pub fn accessibility_granted() -> bool {
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        CallbackResult,
    };

    CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![CGEventType::OtherMouseDragged],
        |_, _, _| CallbackResult::Keep,
    )
    .is_ok()
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

#[cfg(target_os = "macos")]
pub fn pane_anchor(id: &str) -> Option<&'static str> {
    [ACCESSIBILITY, MICROPHONE]
        .into_iter()
        .find(|grant| grant.id == id)
        .map(|grant| grant.anchor)
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
pub fn blockers(state: &DaemonState) -> Vec<Blocker> {
    grant_blockers(state.noted_accessibility(), read_microphone(state))
}

#[cfg(target_os = "macos")]
pub fn blockers_as_read(state: &DaemonState) -> Vec<Blocker> {
    grant_blockers(state.noted_accessibility(), state.microphone_refusal())
}

// Nothing here is a grant read into the state, so a read writes nothing.
#[cfg(not(target_os = "macos"))]
pub fn blockers_as_read(state: &DaemonState) -> Vec<Blocker> {
    blockers(state)
}

/// Split from the live reads, which answer only on the machine they run on.
#[cfg(target_os = "macos")]
fn grant_blockers(accessibility: bool, microphone: Option<Refusal>) -> Vec<Blocker> {
    let mut missing = Vec::new();
    if !accessibility {
        missing.push(blocker(&ACCESSIBILITY));
    }
    if let Some(refusal) = microphone {
        missing.push(Blocker {
            fix: refusal.fix().to_string(),
            ..blocker(&MICROPHONE)
        });
    }
    missing
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

#[cfg(all(unix, not(target_os = "macos")))]
pub fn blockers(state: &DaemonState) -> Vec<Blocker> {
    state.typer_blockers()
}

// X11 types through enigo directly, so only a Wayland session needs a typer.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn live_typer_blockers() -> Vec<Blocker> {
    let path = crate::connect::resolved_path();
    blockers_for(
        crate::dictation::is_wayland(),
        crate::dictation::resolve_wayland_typer(&path).is_some(),
    )
}

// Neither macOS nor a Wayland-capable unix: no client of this build ever sees
// a typer blocker.
#[cfg(not(any(target_os = "macos", unix)))]
pub fn blockers(_: &DaemonState) -> Vec<Blocker> {
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
        name: "No typing tool is installed".to_string(),
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
            "Accessibility is the one grant Banshee needs: without it, {}.",
            ACCESSIBILITY.consequence
        );
        println!(
            "If macOS asks for it, approve the prompt and the daemon restarts itself as it lands. \
             Otherwise {}.",
            ACCESSIBILITY.fix
        );
        println!("`banshee status` says whether it is granted.");
    }
}

/// A daemon that started with the grant never restarts: its key listener
/// keeps firing through a revoke, measured.
#[cfg(target_os = "macos")]
pub fn restarts_for_the_grant(started_granted: bool, now: bool, busy: bool) -> bool {
    !started_granted && now && !busy
}

/// A restart kills whatever is in flight, and a download starts its file over.
#[cfg(target_os = "macos")]
fn busy(state: &DaemonState) -> bool {
    state.is_downloading()
        || state.is_recording()
        || state.is_transcribing()
        || state.is_typing()
        || state.speech().is_speaking()
}

/// Polls the grant so a move reaches every client. A daemon that started
/// without it leaves once it lands, so the supervisor starts it again with
/// it: launchd re-runs on nonzero exit (`KeepAlive`/`SuccessfulExit`).
/// `leave_to_restart` ends the daemon the way a signal does, rather than by
/// `exit`, which would leave the socket file behind for the next run to call
/// a crash.
#[cfg(target_os = "macos")]
pub fn follow_accessibility(state: Arc<DaemonState>) {
    use std::thread;
    use std::time::Duration;

    const POLL: Duration = Duration::from_secs(2);
    // A denied tap keeps about 0.7 KB in this process, measured, so a missing
    // grant is read less often.
    const POLL_WHILE_DENIED: Duration = Duration::from_secs(10);

    let started_granted = state.noted_accessibility();
    if !started_granted {
        log::warn!("the Accessibility grant is missing; the hotkey is inert until it lands");
    }
    thread::spawn(move || {
        loop {
            thread::sleep(if state.noted_accessibility() {
                POLL
            } else {
                POLL_WHILE_DENIED
            });
            let now = state.read_accessibility();
            if state.note_accessibility(now) {
                if now {
                    log::info!("the Accessibility grant is back");
                } else {
                    log::warn!(
                        "the Accessibility grant was switched off; dictation is copied until it is back"
                    );
                }
            }
            if restarts_for_the_grant(started_granted, now, busy(&state)) {
                log::info!("the Accessibility grant landed, restarting to pick it up");
                state.leave_to_restart();
                return;
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn follow_accessibility(_: Arc<DaemonState>) {}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn a_grant_restarts_only_a_daemon_that_started_without_it_and_only_when_idle() {
        assert!(restarts_for_the_grant(false, true, false));
        assert!(!restarts_for_the_grant(false, true, true), "busy waits");
        assert!(!restarts_for_the_grant(false, false, false));
        assert!(
            !restarts_for_the_grant(true, true, false),
            "the listener survived, so a re-grant needs no restart"
        );
        assert!(
            !restarts_for_the_grant(true, false, false),
            "a revoke never restarts"
        );
    }

    #[test]
    fn a_restart_waits_for_each_thing_in_flight() {
        use crate::state::RecordingMode;
        use std::time::{Duration, Instant};

        let held = crate::test_support::daemon_state_holding_speech(
            Duration::from_secs(5),
            std::sync::mpsc::channel().0,
        );
        let state = &held.state;
        assert!(!busy(state));

        state.set_typing(true);
        assert!(busy(state), "typing");
        state.set_typing(false);

        state.set_transcribing(true);
        assert!(busy(state), "transcribing");
        state.set_transcribing(false);

        state.set_recording_mode(RecordingMode::Armed);
        assert!(busy(state), "an armed question");
        state.set_recording_mode(RecordingMode::Idle);

        let download = state.start_downloading().unwrap();
        assert!(busy(state), "downloading");
        drop(download);
        assert!(!busy(state));

        state.speech().speak("a line", false, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !state.speech().is_speaking() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(busy(state), "speaking");
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

    // Before any press: macOS prompts at the first open, so only a refusal
    // stops anything, and status names it with what lifts it.
    #[test]
    fn a_refused_microphone_is_a_grant_blocker_and_an_unasked_one_is_not() {
        let ids = |accessibility, microphone| {
            grant_blockers(accessibility, microphone)
                .into_iter()
                .map(|blocker| blocker.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(true, None), Vec::<String>::new());
        assert_eq!(ids(true, Some(Refusal::Denied)), ["microphone"]);
        assert_eq!(
            ids(false, Some(Refusal::Denied)),
            ["accessibility", "microphone"]
        );

        let [refused] = &grant_blockers(true, Some(Refusal::Denied))[..] else {
            panic!("one blocker");
        };
        assert_eq!(refused.kind, BlockerKind::Permission);
        assert_eq!(refused.name, "Microphone access is not allowed");
        assert_eq!(refused.consequence, "dictation and spoken answers fail");
        assert_eq!(refused.remedy, Some(banshee_common::Remedy::Grant));
        assert_eq!(
            refused.fix,
            "allow Banshee in System Settings > Privacy & Security > Microphone"
        );
    }

    // Nobody on the Mac can allow a restricted microphone, so its fix names
    // who can.
    #[test]
    fn a_restricted_microphone_names_the_administrator_rather_than_a_switch() {
        assert_eq!(refusal_of(0), None);
        assert_eq!(refusal_of(1), Some(Refusal::Restricted));
        assert_eq!(refusal_of(2), Some(Refusal::Denied));
        assert_eq!(refusal_of(3), None);
        let [restricted] = &grant_blockers(true, Some(Refusal::Restricted))[..] else {
            panic!("one blocker");
        };
        assert_eq!(
            restricted.fix,
            "ask the administrator of this Mac for microphone access"
        );
    }

    #[test]
    fn the_live_read_wins_and_the_old_one_only_names_a_restriction() {
        // daemon started granted, then revoked
        assert_eq!(refusal_from(Some(Record::Denied), 3), Some(Refusal::Denied));
        // daemon started denied, then granted
        assert_eq!(refusal_from(Some(Record::Granted), 2), None);
        // permission reset while running
        assert_eq!(refusal_from(Some(Record::Undetermined), 2), None);
        assert_eq!(
            refusal_from(Some(Record::Denied), 1),
            Some(Refusal::Restricted)
        );
        assert_eq!(refusal_from(Some(Record::Granted), 1), None);
        assert_eq!(
            refusal_from(Some(Record::Undetermined), 1),
            Some(Refusal::Restricted)
        );
        // macOS 13: no live read
        assert_eq!(refusal_from(None, 1), Some(Refusal::Restricted));
        assert_eq!(refusal_from(None, 2), Some(Refusal::Denied));
        assert_eq!(refusal_from(None, 3), None);
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
        assert_eq!(blocker.name, "Accessibility is not granted");
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
        assert_eq!(found[0].name, "No typing tool is installed");
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
