// A TCC grant applies only to processes started after it lands, so the daemon
// has to restart to pick one up. No equivalent outside macOS.

use banshee_common::Blocker;
#[cfg(target_os = "macos")]
use banshee_common::BlockerKind;

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq)]
pub enum Access {
    Granted,
    Denied,
    /// Never asked. The first event tap turns this into granted or denied.
    Undetermined,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
pub enum Grant {
    Accessibility,
    InputMonitoring,
}

#[cfg(target_os = "macos")]
impl Grant {
    pub const REQUIRED: [Grant; 2] = [Grant::Accessibility, Grant::InputMonitoring];

    pub fn name(self) -> &'static str {
        match self {
            Grant::Accessibility => "Accessibility",
            Grant::InputMonitoring => "Input Monitoring",
        }
    }

    /// Stable across renames of `name`, so a client can switch on it.
    pub fn id(self) -> &'static str {
        match self {
            Grant::Accessibility => "accessibility",
            Grant::InputMonitoring => "input_monitoring",
        }
    }

    pub fn consequence(self) -> &'static str {
        match self {
            Grant::Accessibility => "dictation cannot type and the hotkey stays inert",
            Grant::InputMonitoring => "the hotkey receives no key presses, with no error anywhere",
        }
    }

    pub fn fix(self) -> &'static str {
        match self {
            Grant::Accessibility => {
                "grant it in System Settings > Privacy & Security > Accessibility"
            }
            Grant::InputMonitoring => {
                "grant it in System Settings > Privacy & Security > Input Monitoring"
            }
        }
    }

    pub fn access(self) -> Access {
        match self {
            Grant::Accessibility => {
                #[link(name = "ApplicationServices", kind = "framework")]
                unsafe extern "C" {
                    fn AXIsProcessTrusted() -> u8;
                }
                // C booleans are unsigned char, so u8 keeps the call sound
                if unsafe { AXIsProcessTrusted() != 0 } {
                    Access::Granted
                } else {
                    Access::Denied
                }
            }
            Grant::InputMonitoring => {
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
        }
    }

    /// Anchors are the `revealElementKeyName` values macOS publishes in
    /// `Security.prefPane/Contents/Resources/PrivacyTCCServices.plist`.
    pub fn open_settings(self) {
        let anchor = match self {
            Grant::Accessibility => "Privacy_Accessibility",
            Grant::InputMonitoring => "Privacy_ListenEvent",
        };
        let _ = std::process::Command::new("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{anchor}"
            ))
            .status();
    }

    pub fn missing() -> Vec<Grant> {
        Grant::REQUIRED
            .into_iter()
            .filter(|grant| grant.access() != Access::Granted)
            .collect()
    }
}

pub fn blockers() -> Vec<Blocker> {
    #[cfg(target_os = "macos")]
    {
        Grant::missing()
            .into_iter()
            .map(|grant| Blocker {
                kind: BlockerKind::Permission,
                id: grant.id().to_string(),
                name: grant.name().to_string(),
                consequence: grant.consequence().to_string(),
                fix: grant.fix().to_string(),
            })
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// Names what is missing and opens the pane for the first one. True when a
/// grant is missing.
pub fn guide_missing() -> bool {
    #[cfg(target_os = "macos")]
    {
        let missing = Grant::missing();
        let Some(first) = missing.first() else {
            return false;
        };
        println!();
        for grant in &missing {
            println!("{} is not granted: {}.", grant.name(), grant.consequence());
        }
        println!(
            "Opening System Settings at {}. Every switch is in the same Privacy & Security list.",
            first.name()
        );
        println!("The daemon restarts itself as each one lands.");
        first.open_settings();
        true
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Exit once a missing grant lands, so the supervisor restarts us with it:
/// launchd re-runs on nonzero exit (`KeepAlive`/`SuccessfulExit`), systemd on
/// `Restart=on-failure`.
pub fn restart_when_granted() {
    #[cfg(target_os = "macos")]
    {
        use std::thread;
        use std::time::Duration;

        const POLL: Duration = Duration::from_secs(2);

        let waiting = Grant::missing().len();
        if waiting == 0 {
            return;
        }
        tracing::warn!("a hotkey permission is missing; the hotkey is inert until it lands");
        // Any one landing is worth a restart: each grant buys back its own feature
        thread::spawn(move || {
            loop {
                thread::sleep(POLL);
                if Grant::missing().len() < waiting {
                    tracing::info!("a hotkey permission landed, restarting to pick it up");
                    std::process::exit(1);
                }
            }
        });
    }
}
