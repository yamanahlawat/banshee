// macOS gates the hotkey behind two TCC grants, and each one applies only to
// processes that started after it landed. No equivalent elsewhere, so this
// whole surface is a no-op on other platforms.

/// What macOS reports for a grant it has been asked about.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq)]
pub enum Access {
    Granted,
    Denied,
    /// Never decided. The first event tap turns this into granted or denied.
    Undetermined,
}

/// A permission the hotkey needs. Both are silent when missing, so each one
/// carries what breaks, how to fix it, and the pane that grants it.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
pub enum Grant {
    Accessibility,
    InputMonitoring,
}

#[cfg(target_os = "macos")]
impl Grant {
    /// Every grant the daemon needs. One list, so a check and a prompt cannot
    /// disagree about what "permitted" means.
    pub const REQUIRED: [Grant; 2] = [Grant::Accessibility, Grant::InputMonitoring];

    pub fn name(self) -> &'static str {
        match self {
            Grant::Accessibility => "Accessibility",
            Grant::InputMonitoring => "Input Monitoring",
        }
    }

    /// What the user loses while it is missing.
    pub fn consequence(self) -> &'static str {
        match self {
            Grant::Accessibility => "dictation cannot type and the hotkey stays inert",
            Grant::InputMonitoring => "the hotkey receives no key presses, with no error anywhere",
        }
    }

    pub fn fix(self) -> &'static str {
        match self {
            Grant::Accessibility => {
                "grant it in System Settings > Privacy & Security > \
                 Accessibility. unsigned debug builds lose the grant on every rebuild"
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

    /// Opens the pane that grants it, rather than describing where it lives.
    /// The anchors are the `revealElementKeyName` values macOS publishes in
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

/// Names every missing grant and opens the pane for the first one, so someone
/// who never opens a terminal still lands on the switch that has to be flipped.
/// Only macOS gates the hotkey this way, so elsewhere there is nothing to say.
pub fn guide_missing() {
    #[cfg(target_os = "macos")]
    {
        let missing = Grant::missing();
        let Some(first) = missing.first() else {
            return;
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
    }
}

/// Exit once a missing grant lands, so the supervisor restarts us with it.
/// launchd re-runs on nonzero exit (`KeepAlive`/`SuccessfulExit`) and so does
/// systemd (`Restart=on-failure`), and the next start reclaims the stale
/// socket. The event tap is built once at startup, so a grant that arrives
/// later does nothing until the process comes back. Only macOS gates the hotkey
/// this way, so elsewhere there is nothing to wait for.
pub fn restart_when_granted() {
    #[cfg(target_os = "macos")]
    {
        use std::thread;
        use std::time::Duration;

        // How often we look for a grant that landed while we were running.
        const POLL: Duration = Duration::from_secs(2);

        let waiting = Grant::missing().len();
        if waiting == 0 {
            return;
        }
        tracing::warn!("a hotkey permission is missing; the hotkey is inert until it lands");
        // Restart as soon as any one lands, not once they all have: each grant
        // buys back its own feature, and the next start re-arms for the rest
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
