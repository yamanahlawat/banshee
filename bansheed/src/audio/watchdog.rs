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
mod tests;
