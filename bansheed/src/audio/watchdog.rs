use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use ringbuf::{HeapRb, traits::Split};

use crate::audio::{CaptureSink, RING_SECS, Selection, already_serving, follows_os_default};
use crate::hotkey::{Capture, CaptureSource};
use crate::state::{DaemonState, Mic, RecordingError, Refusal};

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

/// `serving` and `fault` are the only writers of shared state. An attempt
/// writes every fact it concludes with, so none of them can answer for an
/// earlier attempt.
#[derive(Default)]
struct Binding {
    // `None` whenever the open stream satisfies no setting, which keeps the
    // tick asking until the named device comes back.
    opened_for: Option<String>,
    open_device: Option<String>,
}

impl Binding {
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
        state.set_audio_device(Some(open.clone()));
        self.open_device = Some(open);
        state.set_missing_device(missing);
        state.set_mic(Mic::Open);
    }

    /// Nothing records. `recording_error` carries the whole fault, so
    /// `missing_device` clears with the rest.
    fn fault(&mut self, state: &DaemonState, reason: String) {
        self.opened_for = None;
        self.open_device = None;
        state.set_audio_device(None);
        state.set_missing_device(None);
        state.set_mic(Mic::Faulted(RecordingError::Microphone(reason)));
    }

    /// Recording is unavailable only when the stream this tick holds is not
    /// delivering, so a live stream keeps every fact it has. The caller logs
    /// the case it reports.
    fn attempt_failed(
        &mut self,
        state: &DaemonState,
        stalled: bool,
        reason: &str,
    ) -> AttemptFailure {
        if !stalled {
            return AttemptFailure::KeptTheLiveStream;
        }
        let reported = matches!(
            state.pipeline().fault(),
            Some(RecordingError::Microphone(before)) if before == reason
        );
        self.fault(state, reason.to_string());
        if reported {
            AttemptFailure::SameFault
        } else {
            AttemptFailure::NewFault
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AttemptFailure {
    KeptTheLiveStream,
    /// Recording is unavailable, for a reason the log has not seen.
    NewFault,
    /// Recording is still unavailable, for the reason already reported.
    SameFault,
}

impl AttemptFailure {
    fn log(&self, reason: &str) {
        match self {
            AttemptFailure::NewFault => log::error!("Recording is unavailable: {reason}"),
            AttemptFailure::KeptTheLiveStream => {
                log::warn!("Capture keeps the device it has: {reason}")
            }
            AttemptFailure::SameFault => {}
        }
    }
}

/// The machine's microphones, as the thread that owns capture sees them.
pub trait Devices: Send + 'static {
    type Device;
    type Stream: Send;

    /// One walk of the devices, resolved against the setting.
    fn select(&mut self, wanted: &str) -> Result<Selection<Self::Device>, String>;

    /// Builds and plays a stream on the device. `sink` gives the sink for the
    /// device's rate and channel count. The error names the device.
    fn open(
        &mut self,
        selection: &Selection<Self::Device>,
        sink: &mut dyn FnMut(u32, usize) -> CaptureSink,
    ) -> Result<Self::Stream, String>;

    /// The name the OS gives its default input.
    fn default_name(&mut self) -> Option<String>;

    /// What the platform refuses about the microphone, read from its grant.
    /// Nothing is opened to find out.
    fn refusal(&mut self) -> Option<Refusal>;
}

struct Held<S> {
    stream: S,
    binding: Binding,
}

/// Opens capture for each use and closes it after, or holds it open under
/// `audio.keep_open`. While it is open, it rebinds a stream that stalls, a
/// moved OS default and a changed setting. While it is closed it does nothing:
/// the next open reads the devices afresh.
struct Owner<D: Devices> {
    state: Arc<DaemonState>,
    capture: Arc<Capture>,
    devices: D,
    held: Option<Held<D::Stream>>,
    last_wanted: String,
    next_tick: Instant,
    next_scan: Instant,
    next_default_read: Instant,
}

impl<D: Devices> Owner<D> {
    fn new(state: Arc<DaemonState>, capture: Arc<Capture>, devices: D, now: Instant) -> Self {
        let last_wanted = state.wanted_device();
        Self {
            state,
            capture,
            devices,
            held: None,
            last_wanted,
            next_tick: now,
            next_scan: now,
            next_default_read: now,
        }
    }

    /// Reads the grant, and names the device the first use will open, without
    /// opening either.
    fn seed(&mut self) {
        self.state.set_microphone_refusal(self.devices.refusal());
        if self.state.wants_mic() {
            return;
        }
        match self.devices.select(&self.state.wanted_device()) {
            Ok(selection) => {
                self.state.set_audio_device(Some(selection.open));
                self.state.set_missing_device(selection.missing);
            }
            Err(reason) => log::warn!("No microphone to open yet: {reason}"),
        }
    }

    /// One pass. Answers when the next pass is due, or `None` when only a
    /// wake brings one.
    fn step(&mut self, now: Instant) -> Option<Instant> {
        self.state.announce_sound();
        match (self.held.is_some(), self.state.wants_mic()) {
            // A use opens at once. Only a held-open microphone that failed
            // waits to try again, since a failed use wants nothing more.
            (false, true) if !self.state.keep_open() || now >= self.next_scan => self.open(now),
            (false, true) => {}
            (true, false) => self.close(),
            (true, true) if now >= self.next_tick => {
                self.next_tick = now + TICK;
                self.tick(now);
            }
            (true, true) => {}
            // A held-open microphone that failed and is no longer held open
            // leaves no fault: the next use tries the device again.
            (false, false) => self.state.set_mic(Mic::Closed),
        }
        match (self.held.is_some(), self.state.wants_mic()) {
            (true, _) => Some(self.next_tick),
            (false, true) => Some(self.next_scan),
            (false, false) => None,
        }
    }

    fn open(&mut self, now: Instant) {
        let wanted = self.state.wanted_device();
        self.last_wanted.clone_from(&wanted);
        // The silence limit counts from the open, as it does after a stall.
        self.state.mark_capture_alive();
        let refusal = self.devices.refusal();
        self.state.set_microphone_refusal(refusal);
        let opened = match refusal {
            Some(refusal) => Err(RecordingError::MicrophoneRefused(refusal)),
            None => self
                .devices
                .select(&wanted)
                .and_then(|selection| self.play(&selection).map(|stream| (selection, stream)))
                .map_err(RecordingError::Microphone),
        };
        match opened {
            Ok((selection, stream)) => {
                match &selection.missing {
                    Some(name) => log::info!(
                        "Capture opened {}, still waiting for {name}",
                        selection.open
                    ),
                    None => log::info!("Capture opened {}", selection.open),
                }
                let mut binding = Binding::default();
                binding.serving(&self.state, wanted, selection.open, selection.missing);
                self.held = Some(Held { stream, binding });
                self.next_tick = now + TICK;
                self.next_default_read = now + RETRY;
            }
            // Held open, a failure is a fault with the recovery fix, whatever
            // its cause; the grant blocker names a refusal on its own.
            Err(fault) if self.state.keep_open() => {
                let reason = match fault {
                    RecordingError::MicrophoneRefused(refusal) => refusal.reason().to_string(),
                    RecordingError::Microphone(reason) => reason,
                    other => other.to_string(),
                };
                self.next_scan = now + RETRY;
                Binding::default()
                    .attempt_failed(&self.state, true, &reason)
                    .log(&reason);
            }
            Err(fault) => {
                log::error!("{fault}");
                self.capture.fail(fault.failed_use());
                self.state.capture_failed(fault);
            }
        }
    }

    /// Plays into the ring capture holds when it fits, or into a new one. A
    /// new ring reaches readers only once its stream plays.
    fn play(&mut self, selection: &Selection<D::Device>) -> Result<D::Stream, String> {
        let state = &self.state;
        let capture = &self.capture;
        let mut fresh = None;
        let stream = self.devices.open(selection, &mut |rate, channels| {
            let producer = capture.reuse(rate).unwrap_or_else(|| {
                let (producer, consumer) = HeapRb::new(rate as usize * RING_SECS).split();
                fresh = Some(CaptureSource {
                    consumer,
                    sample_rate: rate,
                });
                producer
            });
            CaptureSink::new(Arc::clone(state), producer, channels)
        })?;
        if let Some(source) = fresh {
            // Written rather than sent: a question that is already listening
            // holds the consumer thread, and a command would wait for it.
            self.capture.swap(source);
        }
        self.capture.forget_failure();
        Ok(stream)
    }

    fn close(&mut self) {
        self.held = None;
        log::debug!("Capture closed");
        self.state.set_mic(Mic::Closed);
    }

    fn tick(&mut self, now: Instant) {
        let Some(held) = self.held.as_ref() else {
            return;
        };
        let open_device = held.binding.open_device.clone();
        let opened_for = held.binding.opened_for.clone();
        let wanted_now = self.state.wanted_device();
        // One tick sees the move, so a failure that follows is paced again
        let setting_moved = wanted_now != self.last_wanted;
        self.last_wanted.clone_from(&wanted_now);
        let stalled = self.state.capture_is_stalled();
        // The 17 ms read carries its own deadline, because no attempt
        // happens while the stream is healthy and the throttle is therefore
        // open on every tick. On the bare tick this read costs 3.4 percent of
        // a core, and at RETRY it costs 0.34 percent.
        let os_default = if follows_os_default(&wanted_now) && now >= self.next_default_read {
            self.next_default_read = now + RETRY;
            self.devices.default_name()
        } else {
            None
        };
        // Against the open device, because the setting is the string
        // `default` and would never differ from itself
        let default_moved =
            os_default_moved(&wanted_now, open_device.as_deref(), os_default.as_deref());
        if !should_attempt(
            stalled,
            default_moved,
            &wanted_now,
            opened_for.as_deref(),
            throttled(now, self.next_scan, setting_moved),
        ) {
            return;
        }
        // Every attempt pays the throttle. A stream that opens and never
        // calls back stays stalled, so success alone is no reason to
        // enumerate and allocate a ring again on the next tick.
        self.next_scan = now + RETRY;

        // One open path for both a wanted device and a substitute
        let attempt = match self.devices.select(&wanted_now) {
            Ok(selection) => {
                // A rescan while a named device stays absent lands here every
                // RETRY, so it opens nothing and logs nothing
                if already_serving(&selection.open, open_device.as_deref(), stalled) {
                    Ok((selection, None))
                } else {
                    self.play(&selection)
                        .map(|stream| (selection, Some(stream)))
                }
            }
            Err(reason) => Err(reason),
        };
        let Some(held) = self.held.as_mut() else {
            return;
        };
        match attempt {
            Ok((selection, stream)) => {
                if let Some(stream) = stream {
                    // The old stream dies here, after the new one is live
                    held.stream = stream;
                    // Only a rebind logs, so a device that stays absent does
                    // not write a line every RETRY
                    match &selection.missing {
                        Some(name) => log::info!(
                            "Capture rebound to {}, still waiting for {name}",
                            selection.open
                        ),
                        None => log::info!("Capture rebound to {}", selection.open),
                    }
                }
                held.binding
                    .serving(&self.state, wanted_now, selection.open, selection.missing);
            }
            Err(reason) => held
                .binding
                .attempt_failed(&self.state, stalled, &reason)
                .log(&reason),
        }
    }
}

pub struct Handle {
    stop: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

impl Handle {
    /// Closes capture and ends the thread.
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.thread().unpark();
        let _ = self.thread.join();
    }
}

/// The thread that owns capture. Nothing else holds a stream.
pub fn spawn<D: Devices>(state: Arc<DaemonState>, capture: Arc<Capture>, devices: D) -> Handle {
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        // Before the first pass, so a change that pass misses still wakes it
        state.set_mic_owner(thread::current());
        let mut owner = Owner::new(state, capture, devices, Instant::now());
        owner.seed();
        while !stopping.load(Ordering::Relaxed) {
            match owner.step(Instant::now()) {
                Some(due) => thread::park_timeout(due.saturating_duration_since(Instant::now())),
                None => thread::park(),
            }
        }
    });
    Handle { stop, thread }
}

#[cfg(test)]
mod tests;
