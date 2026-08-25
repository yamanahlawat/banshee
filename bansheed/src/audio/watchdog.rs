use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use cpal::Stream;

use crate::audio::{already_serving, default_input_name, follows_os_default, open_capture, select};
use crate::state::{ConsumerCommand, DaemonState, RecordingError};

// Reading the liveness stamp is one atomic load, so this can be frequent
const TICK: Duration = Duration::from_millis(500);
// Enumeration costs about 176 ms and the default-name read about 17 ms, so
// neither runs on the bare tick: a device that stays missing is rescanned at
// this interval, and the OS default is read at it
const RETRY: Duration = Duration::from_secs(5);

/// True when this tick must try to rebuild: capture died, the setting moved,
/// or the OS default moved under a `default` binding, and the throttle allows
/// another attempt.
///
/// `None` for `opened_for` means the open stream satisfies no setting, so the
/// tick keeps asking. A substitution is that case.
pub fn should_attempt(
    stalled: bool,
    default_moved: bool,
    wanted: &str,
    opened_for: Option<&str>,
    wait: bool,
) -> bool {
    !wait && (stalled || default_moved || opened_for != Some(wanted))
}

/// True when the OS default is a device other than the open one, and the
/// setting is the one that follows the OS.
///
/// A named setting never follows the OS default, because that substitutes a
/// device the user chose by hand. With nothing open there is nothing to
/// compare, and an unsatisfied setting already rescans every `RETRY`.
fn os_default_moved(wanted: &str, open_device: Option<&str>, os_default: Option<&str>) -> bool {
    match (follows_os_default(wanted), open_device, os_default) {
        (true, Some(open), Some(os)) => os != open,
        _ => false,
    }
}

/// True when this tick must wait before it tries again. A setting that moved
/// clears the wait, because a person is waiting for the answer.
fn throttled(now: Instant, next_scan: Instant, setting_moved: bool) -> bool {
    !setting_moved && now < next_scan
}

/// `serving` and `fault` are the only writers of shared state, and `seeded`
/// reads none. An attempt writes every fact it concludes with, so none of them
/// can answer for an earlier attempt.
struct Binding {
    // `None` whenever the open stream satisfies no setting, which keeps the
    // tick asking until the named device comes back.
    opened_for: Option<String>,
    open_device: Option<String>,
}

impl Binding {
    /// What startup resolved, handed in rather than read back. A startup that
    /// substituted satisfies no setting, so the seed cannot disagree with the
    /// device startup opened.
    fn seeded(wanted: String, open: String, missing: Option<String>) -> Self {
        Self {
            opened_for: match missing {
                Some(_) => None,
                None => Some(wanted),
            },
            open_device: Some(open),
        }
    }

    fn serving(
        &mut self,
        state: &DaemonState,
        wanted: String,
        open: String,
        missing: Option<String>,
    ) {
        self.opened_for = match missing {
            Some(_) => None,
            None => Some(wanted),
        };
        self.open_device = Some(open);
        state.set_missing_device(missing);
        state.clear_recording_error();
    }

    /// Nothing records. `recording_error` carries the whole fault, so
    /// `missing_device` clears with the rest.
    fn fault(&mut self, state: &DaemonState, reason: String) {
        self.opened_for = None;
        self.open_device = None;
        state.set_audio_device(None);
        state.set_missing_device(None);
        state.set_recording_error(RecordingError::Microphone(reason));
    }

    /// Recording is unavailable only when the stream this tick holds is not
    /// delivering, so a live stream keeps every fact it has. The caller logs
    /// the case it reports.
    fn attempt_failed(&mut self, state: &DaemonState, stalled: bool, reason: &str) -> bool {
        if !stalled {
            return false;
        }
        self.fault(state, reason.to_string());
        true
    }
}

pub struct Handle {
    stop: mpsc::Sender<()>,
    thread: thread::JoinHandle<()>,
}

impl Handle {
    pub fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.thread.join();
    }
}

/// Takes ownership of the capture stream. Nothing else may hold one. `open` and
/// `missing` are what startup resolved, so the seed does not depend on when
/// `main` writes `missing_device`.
pub fn spawn(
    state: Arc<DaemonState>,
    stream: Stream,
    open: String,
    missing: Option<String>,
) -> Handle {
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        let mut stream = stream;
        let mut last_wanted = state.wanted_device();
        let mut binding = Binding::seeded(last_wanted.clone(), open, missing);
        let mut next_scan = Instant::now();
        let mut next_default_read = Instant::now() + RETRY;

        loop {
            match stop_rx.recv_timeout(TICK) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }

            let wanted_now = state.wanted_device();
            // One tick sees the move, so a failure that follows is paced again
            let setting_moved = wanted_now != last_wanted;
            last_wanted.clone_from(&wanted_now);
            let stalled = state.capture_is_stalled();
            // The 17 ms read carries its own deadline, because no attempt
            // happens while the daemon is healthy and the throttle is
            // therefore open on every tick. On the bare tick this read costs
            // 3.4 percent of a core, and at RETRY it costs 0.34 percent.
            let now = Instant::now();
            let os_default = if follows_os_default(&wanted_now) && now >= next_default_read {
                next_default_read = now + RETRY;
                default_input_name()
            } else {
                None
            };
            // Against the open device, because the setting is the string
            // `default` and would never differ from itself
            let default_moved = os_default_moved(
                &wanted_now,
                binding.open_device.as_deref(),
                os_default.as_deref(),
            );
            if !should_attempt(
                stalled,
                default_moved,
                &wanted_now,
                binding.opened_for.as_deref(),
                throttled(now, next_scan, setting_moved),
            ) {
                continue;
            }
            // Every attempt pays the throttle. A stream that opens and never
            // calls back stays stalled, so success alone is no reason to
            // enumerate and allocate a ring again on the next tick.
            next_scan = Instant::now() + RETRY;

            // One open path for both a wanted device and a substitute
            let selection = match select(&wanted_now) {
                Ok(selection) => selection,
                Err(reason) => {
                    if binding.attempt_failed(&state, stalled, &reason) {
                        eprintln!("Recording is unavailable: {reason}");
                    } else {
                        eprintln!("Capture keeps the device it has: {reason}");
                    }
                    continue;
                }
            };

            // A rescan while a named device stays absent lands here every
            // RETRY, so it opens nothing and logs nothing
            if already_serving(&selection.open, binding.open_device.as_deref(), stalled) {
                binding.serving(&state, wanted_now, selection.open, selection.missing);
                continue;
            }

            match open_capture(Arc::clone(&state), &selection) {
                Ok(capture) => {
                    // Sent only after play() succeeded, so the pipeline is
                    // never handed a ring whose stream failed to play
                    let _ = state.commands().send(ConsumerCommand::Rebind {
                        consumer: capture.consumer,
                        sample_rate: capture.sample_rate,
                    });
                    // The old stream dies here, after the new one is live
                    stream = capture.stream;
                    let opened = selection.open;
                    // Only a rebind logs, so a device that stays absent does
                    // not write a line every RETRY
                    match &selection.missing {
                        Some(name) => {
                            println!("Capture rebound to {opened}, still waiting for {name}")
                        }
                        None => println!("Capture rebound to {opened}"),
                    }
                    binding.serving(&state, wanted_now, opened, selection.missing);
                }
                Err(error) => {
                    let reason = error.to_string();
                    // A fault clears the device name too: open_capture names
                    // the device only after play() succeeds, so the old name
                    // would otherwise stand
                    if binding.attempt_failed(&state, stalled, &reason) {
                        eprintln!("Could not open {}: {reason}", selection.open);
                    } else {
                        eprintln!(
                            "Capture keeps the device it has, {} did not open: {reason}",
                            selection.open
                        );
                    }
                }
            }
        }
        drop(stream);
    });

    Handle {
        stop: stop_tx,
        thread,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TranscribeTarget;

    struct Tick {
        name: &'static str,
        stalled: bool,
        default_moved: bool,
        wanted: &'static str,
        opened_for: Option<&'static str>,
        wait: bool,
        attempt: bool,
    }

    #[test]
    fn the_tick_decides_whether_to_attempt() {
        let ticks = [
            Tick {
                name: "a healthy stream on the wanted device is left alone",
                stalled: false,
                default_moved: false,
                wanted: "default",
                opened_for: Some("default"),
                wait: false,
                attempt: false,
            },
            Tick {
                name: "a stalled stream rebuilds",
                stalled: true,
                default_moved: false,
                wanted: "default",
                opened_for: Some("default"),
                wait: false,
                attempt: true,
            },
            Tick {
                name: "a changed setting rebuilds a healthy stream",
                stalled: false,
                default_moved: false,
                wanted: "yeti",
                opened_for: Some("default"),
                wait: false,
                attempt: true,
            },
            Tick {
                name: "a throttled tick leaves a stalled stream alone",
                stalled: true,
                default_moved: false,
                wanted: "default",
                opened_for: Some("default"),
                wait: true,
                attempt: false,
            },
            Tick {
                name: "a throttled tick leaves a changed setting alone",
                stalled: false,
                default_moved: false,
                wanted: "yeti",
                opened_for: Some("default"),
                wait: true,
                attempt: false,
            },
            Tick {
                name: "a throttled tick on a healthy stream attempts nothing",
                stalled: false,
                default_moved: false,
                wanted: "default",
                opened_for: Some("default"),
                wait: true,
                attempt: false,
            },
            Tick {
                name: "a substitute satisfies nothing, so an open tick asks again",
                stalled: false,
                default_moved: false,
                wanted: "yeti",
                opened_for: None,
                wait: false,
                attempt: true,
            },
            Tick {
                name: "a substitute still waits for the throttle",
                stalled: false,
                default_moved: false,
                wanted: "yeti",
                opened_for: None,
                wait: true,
                attempt: false,
            },
            Tick {
                name: "nothing open on a healthy tick asks again",
                stalled: false,
                default_moved: false,
                wanted: "default",
                opened_for: None,
                wait: false,
                attempt: true,
            },
            Tick {
                name: "the OS default moved, so a healthy satisfied stream rebuilds",
                stalled: false,
                default_moved: true,
                wanted: "default",
                opened_for: Some("default"),
                wait: false,
                attempt: true,
            },
            Tick {
                name: "a throttled tick leaves a moved OS default alone",
                stalled: false,
                default_moved: true,
                wanted: "default",
                opened_for: Some("default"),
                wait: true,
                attempt: false,
            },
        ];

        for tick in ticks {
            assert_eq!(
                should_attempt(
                    tick.stalled,
                    tick.default_moved,
                    tick.wanted,
                    tick.opened_for,
                    tick.wait
                ),
                tick.attempt,
                "{}",
                tick.name
            );
        }
    }

    struct Moved {
        name: &'static str,
        wanted: &'static str,
        open_device: Option<&'static str>,
        os_default: Option<&'static str>,
        moved: bool,
    }

    #[test]
    fn only_a_default_binding_follows_the_os() {
        let cases = [
            Moved {
                name: "a default binding follows the OS onto the new device",
                wanted: "default",
                open_device: Some("MacBook Pro Microphone"),
                os_default: Some("OnePlus Buds 3"),
                moved: true,
            },
            Moved {
                name: "a blank setting follows the OS too",
                wanted: "",
                open_device: Some("MacBook Pro Microphone"),
                os_default: Some("OnePlus Buds 3"),
                moved: true,
            },
            Moved {
                name: "a default binding that holds the OS default stays put",
                wanted: "default",
                open_device: Some("OnePlus Buds 3"),
                os_default: Some("OnePlus Buds 3"),
                moved: false,
            },
            Moved {
                name: "a named device is never given up for the OS default",
                wanted: "yeti",
                open_device: Some("Blue Yeti Stereo Microphone"),
                os_default: Some("OnePlus Buds 3"),
                moved: false,
            },
            Moved {
                name: "a named device that is absent keeps its substitute",
                wanted: "yeti",
                open_device: Some("MacBook Pro Microphone"),
                os_default: Some("OnePlus Buds 3"),
                moved: false,
            },
            Moved {
                name: "no OS default at all is no reason to rebuild",
                wanted: "default",
                open_device: Some("MacBook Pro Microphone"),
                os_default: None,
                moved: false,
            },
            Moved {
                name: "nothing open is nothing to compare",
                wanted: "default",
                open_device: None,
                os_default: Some("OnePlus Buds 3"),
                moved: false,
            },
        ];

        for case in cases {
            assert_eq!(
                os_default_moved(case.wanted, case.open_device, case.os_default),
                case.moved,
                "{}",
                case.name
            );
        }
    }

    struct Wait {
        name: &'static str,
        next_scan: Instant,
        setting_moved: bool,
        waits: bool,
    }

    #[test]
    fn the_tick_decides_whether_to_wait() {
        let base = Instant::now();
        let now = base + RETRY;
        let passed = base;
        let pending = now + RETRY;

        let waits = [
            Wait {
                name: "before the deadline a tick waits",
                next_scan: pending,
                setting_moved: false,
                waits: true,
            },
            Wait {
                name: "after the deadline a tick tries",
                next_scan: passed,
                setting_moved: false,
                waits: false,
            },
            Wait {
                name: "at the deadline a tick tries",
                next_scan: now,
                setting_moved: false,
                waits: false,
            },
            Wait {
                name: "a moved setting does not wait, even before the deadline",
                next_scan: pending,
                setting_moved: true,
                waits: false,
            },
            Wait {
                name: "a moved setting after the deadline does not wait either",
                next_scan: passed,
                setting_moved: true,
                waits: false,
            },
        ];

        for wait in waits {
            assert_eq!(
                throttled(now, wait.next_scan, wait.setting_moved),
                wait.waits,
                "{}",
                wait.name
            );
        }
    }

    fn test_state() -> Arc<DaemonState> {
        crate::test_support::daemon_state(mpsc::channel().0)
    }

    // A substitution satisfies no setting, so a corrected setting reaches the
    // clear rather than latching on a comparison that already agrees.
    #[test]
    fn a_reverted_setting_still_clears_the_recording_error() {
        let state = test_state();
        let mut binding = Binding {
            opened_for: Some("default".to_string()),
            open_device: Some("MacBook Pro Microphone".to_string()),
        };

        // The setting becomes a device that is absent, so the tick substitutes
        assert!(should_attempt(
            false,
            false,
            "yeti",
            binding.opened_for.as_deref(),
            false
        ));
        binding.serving(
            &state,
            "yeti".to_string(),
            "MacBook Pro Microphone".to_string(),
            Some("yeti".to_string()),
        );
        assert_eq!(binding.opened_for, None, "a substitute satisfies nothing");
        assert_eq!(state.missing_device().as_deref(), Some("yeti"));

        // The setting is corrected. `opened_for` is None, so the tick runs
        // rather than latching on a comparison that already agrees.
        assert!(should_attempt(
            false,
            false,
            "default",
            binding.opened_for.as_deref(),
            false
        ));

        // A fault from an earlier attempt must not outlive this one
        state.set_recording_error(RecordingError::Microphone("gone".to_string()));
        // The corrected setting is served by the open device, so nothing opens
        assert!(already_serving(
            "MacBook Pro Microphone",
            binding.open_device.as_deref(),
            false
        ));
        binding.serving(
            &state,
            "default".to_string(),
            "MacBook Pro Microphone".to_string(),
            None,
        );
        assert!(
            state.recording_error().is_none(),
            "an attempt that concludes clears the fault"
        );
        assert_eq!(state.missing_device(), None);
        assert_eq!(binding.opened_for.as_deref(), Some("default"));

        // Satisfied at last, so the tick rests
        assert!(!should_attempt(
            false,
            false,
            "default",
            binding.opened_for.as_deref(),
            false
        ));
    }

    // The OS default moves under a healthy stream every RETRY, so a walk that
    // fails now reaches a failure arm while the microphone still records. A
    // failure to move is not a failure to record.
    #[test]
    fn a_tick_that_cannot_move_keeps_the_microphone_it_has() {
        let state = test_state();
        let mut binding = Binding {
            opened_for: Some("default".to_string()),
            open_device: Some("MacBook Pro Microphone".to_string()),
        };
        state.set_audio_device(Some("MacBook Pro Microphone".to_string()));

        // The OS default moved to a device the walk does not list
        assert!(os_default_moved(
            "default",
            binding.open_device.as_deref(),
            Some("OnePlus Buds 3")
        ));
        assert!(should_attempt(
            false,
            true,
            "default",
            binding.opened_for.as_deref(),
            false
        ));

        assert!(
            !binding.attempt_failed(&state, false, "no input device is available"),
            "a live stream is not a fault"
        );

        assert!(
            state.recording_error().is_none(),
            "the microphone works, so recording is possible"
        );
        assert!(
            state.record_start(TranscribeTarget::Mailbox),
            "a press must open a session, not answer with the error cue"
        );
        assert_eq!(
            state.audio_device().as_deref(),
            Some("MacBook Pro Microphone"),
            "audio_device names the device that really records"
        );
        assert_eq!(binding.opened_for.as_deref(), Some("default"));
        assert_eq!(
            binding.open_device.as_deref(),
            Some("MacBook Pro Microphone"),
            "the next default read finds the same move, so the tick tries again"
        );

        // The same failure with a dead stream loses capture
        assert!(
            binding.attempt_failed(&state, true, "no input device is available"),
            "a stalled stream that opens nothing is unavailable"
        );
        assert!(matches!(
            state.recording_error(),
            Some(RecordingError::Microphone(_))
        ));
        assert_eq!(state.audio_device(), None);
    }

    #[test]
    fn a_fault_leaves_no_fact_from_the_previous_attempt() {
        let state = test_state();
        let mut binding = Binding {
            opened_for: Some("yeti".to_string()),
            open_device: Some("Blue Yeti Stereo Microphone".to_string()),
        };
        state.set_audio_device(Some("Blue Yeti Stereo Microphone".to_string()));
        state.set_missing_device(Some("webcam".to_string()));

        binding.fault(&state, "no input device is available".to_string());

        assert_eq!(binding.opened_for, None);
        assert_eq!(binding.open_device, None);
        assert_eq!(state.audio_device(), None);
        // recording_error carries this case, so two fields cannot disagree
        assert_eq!(state.missing_device(), None);
        assert!(matches!(
            state.recording_error(),
            Some(RecordingError::Microphone(_))
        ));
    }

    // Startup can substitute, so the seed comes from what startup resolved.
    // A seed that claims the setting is satisfied latches the first tick, and
    // the named device is never taken back.
    #[test]
    fn a_substituted_startup_seeds_an_unsatisfied_setting() {
        let binding = Binding::seeded(
            "yeti".to_string(),
            "MacBook Pro Microphone".to_string(),
            Some("yeti".to_string()),
        );
        assert_eq!(binding.opened_for, None, "a substitute satisfies nothing");
        assert_eq!(
            binding.open_device.as_deref(),
            Some("MacBook Pro Microphone")
        );
        assert!(
            should_attempt(false, false, "yeti", binding.opened_for.as_deref(), false),
            "the first tick must rescan while the yeti is absent"
        );

        // Startup opened the wanted device, so the tick rests
        let binding = Binding::seeded("yeti".to_string(), "Yeti".to_string(), None);
        assert_eq!(binding.opened_for.as_deref(), Some("yeti"));
        assert_eq!(binding.open_device.as_deref(), Some("Yeti"));
        assert!(!should_attempt(
            false,
            false,
            "yeti",
            binding.opened_for.as_deref(),
            false
        ));
    }

    #[test]
    fn a_failed_attempt_retries_when_the_throttle_expires() {
        // A failure clears `opened_for`, so the decision stands
        let (wanted, opened_for) = ("yeti", None);
        assert!(should_attempt(false, false, wanted, opened_for, false));
        assert!(!should_attempt(false, false, wanted, opened_for, true));
        assert!(should_attempt(false, false, wanted, opened_for, false));
    }
}
