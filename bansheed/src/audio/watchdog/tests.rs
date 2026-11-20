use super::*;
use crate::audio::cues::{Cue, Cues};
use crate::hotkey::{Capture, CaptureSource};
use crate::state::{Pipeline, RecordingMode};
use crate::test_support::{Bench, BenchCall};

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
    crate::test_support::daemon_state(std::sync::mpsc::channel().0)
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
    state.set_mic(crate::state::Mic::Faulted(RecordingError::Microphone(
        "gone".to_string(),
    )));
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
        state.pipeline().fault().is_none(),
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

    assert_eq!(
        binding.attempt_failed(&state, false, "no input device is available"),
        AttemptFailure::KeptTheLiveStream,
        "a live stream is not a fault"
    );

    assert!(
        state.pipeline().fault().is_none(),
        "the microphone works, so recording is possible"
    );
    assert!(
        state.record_start(),
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
    assert_eq!(
        binding.attempt_failed(&state, true, "no input device is available"),
        AttemptFailure::NewFault,
        "a stalled stream that opens nothing is unavailable"
    );
    assert!(matches!(
        state.pipeline().fault(),
        Some(RecordingError::Microphone(_))
    ));
    assert_eq!(state.audio_device(), None);
}

// A microphone that stays gone fails on every RETRY. The log needs the fault
// once, and again only when its reason changes.
#[test]
fn a_fault_is_new_only_when_its_reason_is() {
    let state = test_state();
    let mut binding = Binding {
        opened_for: Some("default".to_string()),
        open_device: Some("PipeWire Sound Server".to_string()),
    };

    let attempts = [
        ("Host is down", AttemptFailure::NewFault),
        ("Host is down", AttemptFailure::SameFault),
        ("No such file or directory", AttemptFailure::NewFault),
    ];
    for (reason, expected) in attempts {
        assert_eq!(
            binding.attempt_failed(&state, true, reason),
            expected,
            "{reason}"
        );
    }
}

// A person who picks another microphone while capture is down waits for its
// answer. The same OS error on another device is news.
#[test]
fn another_device_that_fails_the_same_way_is_a_new_fault() {
    let state = test_state();
    let mut binding = Binding {
        opened_for: None,
        open_device: None,
    };
    let yeti = crate::audio::open_failure("Blue Yeti Stereo Microphone", "Host is down");
    let buds = crate::audio::open_failure("OnePlus Buds 3", "Host is down");

    assert_eq!(
        binding.attempt_failed(&state, true, &yeti),
        AttemptFailure::NewFault
    );
    assert_eq!(
        binding.attempt_failed(&state, true, &buds),
        AttemptFailure::NewFault
    );
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
        state.pipeline().fault(),
        Some(RecordingError::Microphone(_))
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

const BUILT_IN: &str = "MacBook Pro Microphone";
const BUDS: &str = "OnePlus Buds 3";
const WAIT: Duration = Duration::from_secs(5);

fn select() -> BenchCall {
    BenchCall::Select("default".to_string())
}

fn open(name: &str) -> BenchCall {
    BenchCall::Open(name.to_string())
}

fn close(name: &str) -> BenchCall {
    BenchCall::Close(name.to_string())
}

fn owner_of(state: &Arc<DaemonState>, bench: &Bench) -> (Owner<Bench>, Arc<Capture>) {
    let capture = Arc::new(Capture::new(CaptureSource::unopened()));
    let owner = Owner::new(
        Arc::clone(state),
        Arc::clone(&capture),
        bench.clone(),
        Instant::now(),
    );
    (owner, capture)
}

#[test]
fn a_press_opens_capture_and_its_release_closes_it() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();

    assert_eq!(owner.step(now), None, "an idle daemon waits for a use");
    assert!(bench.asked().is_empty(), "and opens nothing");

    assert!(state.record_start());
    owner.step(now);
    assert_eq!(bench.asked(), [select(), open(BUILT_IN)]);
    assert_eq!(state.mic(), Mic::Open);
    assert_eq!(state.audio_device().as_deref(), Some(BUILT_IN));

    state.record_stop();
    assert_eq!(owner.step(now), None);
    assert_eq!(bench.asked(), [select(), open(BUILT_IN), close(BUILT_IN)]);
    assert_eq!(state.mic(), Mic::Closed);
    assert_eq!(
        state.pipeline(),
        Pipeline::Open,
        "a closed microphone is ready for the next press"
    );
}

// The default hotkey is AltGr held alone, so every AltGr chord typed opens the
// microphone on its press. The chord's release cancels, and that must close.
#[test]
fn an_altgr_chord_closes_the_capture_its_press_opened() {
    use crate::binding::{Hotkey, HotkeyTracker};
    use rdev::{EventType, Key};

    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();
    let mut tracker = HotkeyTracker::new(Hotkey::default(), crate::config::HotkeyMode::Hold);
    let mut owned = false;
    let mut press = |event: EventType| {
        if let Some(action) = tracker.on_event(&event) {
            crate::hotkey::act(&state, action, &mut owned);
        }
    };

    press(EventType::KeyPress(Key::AltGr));
    owner.step(now);
    assert_eq!(bench.asked(), [select(), open(BUILT_IN)]);

    press(EventType::KeyPress(Key::KeyE));
    press(EventType::KeyRelease(Key::KeyE));
    press(EventType::KeyRelease(Key::AltGr));
    owner.step(now);
    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    assert_eq!(bench.asked(), [select(), open(BUILT_IN), close(BUILT_IN)]);
}

#[test]
fn a_question_opens_capture_once_it_has_played_and_its_end_closes_it() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();

    let session = state.arm_for_ask().expect("an idle daemon arms");
    owner.step(now);
    assert!(
        bench.asked().is_empty(),
        "the question is still playing, so nothing listens yet"
    );

    assert!(state.hear_answer(session));
    owner.step(now);
    assert_eq!(bench.asked(), [select(), open(BUILT_IN)]);

    state.disarm(session);
    owner.step(now);
    assert_eq!(bench.asked(), [select(), open(BUILT_IN), close(BUILT_IN)]);
}

// A closed stream never calls back, so its liveness stamp goes stale, and the
// setting and the OS default can move. Closed capture acts on none of them:
// the next open reads the devices afresh.
#[test]
fn closed_capture_reads_no_device_however_long_it_waits() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN, BUDS], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();
    assert!(state.record_start());
    owner.step(now);
    state.record_stop();
    owner.step(now);
    let asked = bench.asked();

    bench.set_default(Some(BUDS));
    state.set_wanted_device(BUDS.to_string());
    for later in [TICK, RETRY, RETRY * 10] {
        assert_eq!(owner.step(now + later), None);
    }
    assert_eq!(bench.asked(), asked);
}

#[test]
fn the_next_use_opens_the_device_that_is_the_default_by_then() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN, BUDS], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();
    assert!(state.record_start());
    owner.step(now);
    state.record_stop();
    owner.step(now);

    // The headset connects between uses, and the OS makes it the default
    bench.set_default(Some(BUDS));
    assert!(state.record_start());
    owner.step(now);

    assert_eq!(
        bench.asked(),
        [
            select(),
            open(BUILT_IN),
            close(BUILT_IN),
            select(),
            open(BUDS)
        ]
    );
    assert_eq!(state.audio_device().as_deref(), Some(BUDS));
}

#[test]
fn a_device_that_will_not_open_fails_the_press_with_its_name() {
    let (cues, sounds) = Cues::recording();
    let state = crate::test_support::daemon_state_with_cues(cues);
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    bench.refuse(Some(BUILT_IN));
    let (mut owner, capture) = owner_of(&state, &bench);
    let now = Instant::now();

    assert!(state.record_start());
    owner.step(now);

    assert_eq!(
        state.recording_mode(),
        RecordingMode::Idle,
        "the press ends"
    );
    assert_eq!(sounds.try_recv().ok(), Some(Cue::Error));
    let why = state.last_error().expect("the failure is kept for status");
    assert!(why.contains(BUILT_IN), "{why}");
    assert!(
        why.starts_with("the microphone would not open: ")
            && why.ends_with(
                ". Connect the microphone, or choose another in Listening. The next use tries again."
            ),
        "status shows the cause, then the fix: {why}"
    );
    assert_eq!(
        capture.failure().as_deref(),
        Some(why.as_str()),
        "an answer reads the same reason and fix"
    );
    assert_eq!(
        state.pipeline(),
        Pipeline::Open,
        "the next press tries again"
    );
    assert_eq!(owner.step(now), None, "and nothing retries until it does");

    bench.refuse(None);
    assert!(state.record_start());
    owner.step(now);
    assert_eq!(state.mic(), Mic::Open);
    assert_eq!(capture.failure(), None);
}

// The grant is read, so a refused microphone is never walked or opened, and
// the press fails with the one fix that lifts it.
#[test]
fn a_refused_grant_fails_the_press_with_its_fix_and_opens_nothing() {
    let (cues, sounds) = Cues::recording();
    let state = crate::test_support::daemon_state_with_cues(cues);
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    bench.refuse_access(Some(Refusal::Denied));
    let (mut owner, capture) = owner_of(&state, &bench);
    let now = Instant::now();

    assert!(state.record_start());
    owner.step(now);

    assert!(
        bench.asked().is_empty(),
        "nothing walked or opened: {:?}",
        bench.asked()
    );
    assert_eq!(
        state.recording_mode(),
        RecordingMode::Idle,
        "the press ends"
    );
    assert_eq!(sounds.try_recv().ok(), Some(Cue::Error));
    let why = state.last_error().expect("the failure is kept for status");
    assert_eq!(
        why,
        "the microphone would not open: macOS does not allow Banshee to use it. \
         Allow Banshee in System Settings > Privacy & Security > Microphone."
    );
    assert_eq!(capture.failure().as_deref(), Some(why.as_str()));
    assert_eq!(state.microphone_refusal(), Some(Refusal::Denied));
    assert_eq!(
        state.pipeline(),
        Pipeline::Open,
        "the next press reads again"
    );

    bench.refuse_access(None);
    assert!(state.record_start());
    owner.step(now);
    assert_eq!(state.mic(), Mic::Open);
    assert_eq!(state.microphone_refusal(), None);
}

#[test]
fn a_restricted_grant_names_the_administrator() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    bench.refuse_access(Some(Refusal::Restricted));
    let (mut owner, _) = owner_of(&state, &bench);

    assert!(state.record_start());
    owner.step(Instant::now());

    assert_eq!(
        state.last_error().as_deref(),
        Some(
            "the microphone would not open: this Mac restricts it. \
             Ask the administrator of this Mac for microphone access."
        )
    );
}

// Status and the tray tell a refused microphone from an idle one before any
// press, so the start reads the grant once.
#[test]
fn the_start_reads_the_grant_without_opening() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    bench.refuse_access(Some(Refusal::Denied));
    let (mut owner, _) = owner_of(&state, &bench);

    owner.seed();

    assert_eq!(state.microphone_refusal(), Some(Refusal::Denied));
    assert!(
        !bench
            .asked()
            .contains(&BenchCall::Open(BUILT_IN.to_string()))
    );
}

// Held open, the fault carries the recovery fix, and the grant blocker names
// the refusal beside it.
#[test]
fn a_held_open_microphone_that_macos_refuses_is_a_fault() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    bench.refuse_access(Some(Refusal::Denied));
    let (mut owner, _) = owner_of(&state, &bench);

    state.set_keep_open(true);
    owner.step(Instant::now());

    assert_eq!(
        state.pipeline(),
        Pipeline::Broken(RecordingError::Microphone(
            "macOS does not allow Banshee to use it".to_string()
        ))
    );
}

#[test]
fn no_microphone_at_all_fails_the_press_and_says_so() {
    let state = test_state();
    let bench = Bench::with(&[], None);
    let (mut owner, _) = owner_of(&state, &bench);

    assert!(state.record_start());
    owner.step(Instant::now());

    assert_eq!(state.recording_mode(), RecordingMode::Idle);
    let why = state.last_error().expect("the failure is kept for status");
    assert!(why.contains("no input device"), "{why}");
}

#[test]
fn keep_open_holds_capture_open_between_uses_and_switches_live() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();

    state.set_keep_open(true);
    owner.step(now);
    assert_eq!(
        bench.asked(),
        [select(), open(BUILT_IN)],
        "open with no use"
    );

    assert!(state.record_start());
    owner.step(now);
    state.record_stop();
    owner.step(now);
    assert_eq!(
        bench.asked(),
        [select(), open(BUILT_IN)],
        "a use changes nothing"
    );

    state.set_keep_open(false);
    owner.step(now);
    assert_eq!(state.mic(), Mic::Closed);

    state.set_keep_open(true);
    owner.step(now);
    assert_eq!(
        bench.asked(),
        [
            select(),
            open(BUILT_IN),
            close(BUILT_IN),
            select(),
            open(BUILT_IN)
        ]
    );
}

#[test]
fn keep_open_switched_during_a_press_takes_effect_when_the_press_ends() {
    for keep_open_before in [true, false] {
        let state = test_state();
        let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
        let (mut owner, _) = owner_of(&state, &bench);
        let now = Instant::now();
        state.set_keep_open(keep_open_before);
        assert!(state.record_start());
        owner.step(now);

        state.set_keep_open(!keep_open_before);
        owner.step(now);
        assert_eq!(state.mic(), Mic::Open, "the press still records");

        state.record_stop();
        owner.step(now);
        let closed = state.mic() == Mic::Closed;
        assert_eq!(
            closed, keep_open_before,
            "keep_open now {}",
            !keep_open_before
        );
    }
}

#[test]
fn a_held_open_microphone_that_will_not_open_is_a_fault_until_keep_open_goes() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    bench.refuse(Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();

    state.set_keep_open(true);
    assert_eq!(owner.step(now), Some(now + RETRY), "it tries again later");
    assert!(matches!(
        state.pipeline().fault(),
        Some(RecordingError::Microphone(why)) if why.contains(BUILT_IN)
    ));
    owner.step(now + TICK);
    assert_eq!(bench.asked().len(), 1, "not before the throttle allows");

    state.set_keep_open(false);
    assert_eq!(owner.step(now + TICK), None);
    assert_eq!(state.pipeline(), Pipeline::Open, "a press may try again");
}

// The press opens capture whatever the voice is doing, and barge-in stops
// the voice.
#[test]
fn a_press_while_the_voice_speaks_opens_capture_at_once() {
    for (barge_in, stops) in [
        (crate::config::BargeInMode::Stop, true),
        (crate::config::BargeInMode::None, false),
    ] {
        let held = crate::test_support::daemon_state_holding_speech(
            Duration::from_secs(60),
            std::sync::mpsc::channel().0,
        );
        let state = held.state;
        state.set_barge_in(barge_in);
        let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
        let (mut owner, _) = owner_of(&state, &bench);
        state
            .speech()
            .speak("a long reply", false, None)
            .expect("the line starts");

        assert!(state.record_start());
        owner.step(Instant::now());

        assert_eq!(
            held.cut_short.load(std::sync::atomic::Ordering::SeqCst),
            stops,
            "{barge_in:?}"
        );
        assert_eq!(state.mic(), Mic::Open, "{barge_in:?}");
    }
}

#[test]
fn a_press_while_the_voice_loads_opens_capture_and_waits_on_nothing() {
    use crate::text_to_speech::{TtsBackend, lock};
    for (barge_in, still_speaking) in [
        (crate::config::BargeInMode::Stop, false),
        (crate::config::BargeInMode::None, true),
    ] {
        let (entered_to, entered) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let released = std::sync::Mutex::new(released);
        let asked = crate::test_support::Asked::default();
        let heard = Arc::clone(&asked);
        let loaded = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = Arc::clone(&loaded);
        let voice = crate::test_support::on_demand(
            &crate::test_support::companion_models(),
            move |_voice: &str| {
                let _ = entered_to.send(());
                let _ = lock(&released).recv_timeout(Duration::from_secs(5));
                finished.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(Box::new(crate::test_support::Listening(Arc::clone(&heard)))
                    as Box<dyn TtsBackend>)
            },
            crate::test_support::ignored_faults(),
        );
        let state = crate::test_support::daemon_state_speaking_through(
            crate::text_to_speech::SpeechPlayer::new(Box::new(voice)),
        );
        state.set_barge_in(barge_in);
        let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
        let (mut owner, _) = owner_of(&state, &bench);
        state
            .speech()
            .speak("a reply after the voice idled out", false, None)
            .expect("the line starts while its voice loads");
        entered
            .recv_timeout(Duration::from_secs(5))
            .expect("the voice is loading");

        assert!(state.record_start(), "{barge_in:?}");
        owner.step(Instant::now());
        assert!(
            !loaded.load(std::sync::atomic::Ordering::SeqCst),
            "{barge_in:?}: the press came back before the voice loaded"
        );

        assert_eq!(state.mic(), Mic::Open, "{barge_in:?}");
        assert_eq!(state.speech().is_speaking(), still_speaking, "{barge_in:?}");
        release.send(()).unwrap();
        if still_speaking {
            let deadline = Instant::now() + Duration::from_secs(5);
            while lock(&asked).is_empty() {
                assert!(Instant::now() < deadline, "the reply plays once loaded");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

// A press while the consumer is still busy with the last dictation opens
// capture again before that dictation is read.
#[test]
fn a_released_dictation_left_unread_survives_the_next_open() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let (mut owner, capture) = owner_of(&state, &bench);
    let now = Instant::now();
    assert!(state.record_start());
    owner.step(now);
    bench.play(&[0.25; 160]);
    state.record_stop();
    owner.step(now);

    assert!(state.record_start());
    owner.step(now);
    bench.play(&[0.5; 160]);

    let held = capture.drain();
    assert_eq!(held.len(), 320, "{} samples", held.len());
    assert!(held[..160].iter().all(|&sample| sample == 0.25));
}

#[test]
fn a_seed_names_the_device_the_first_use_opens_and_opens_nothing() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);

    owner.seed();

    assert_eq!(bench.asked(), [select()]);
    assert_eq!(state.audio_device().as_deref(), Some(BUILT_IN));
    assert_eq!(state.mic(), Mic::Closed);
}

#[test]
fn the_thread_opens_for_a_press_cues_its_first_sound_and_closes_after() {
    let (cues, sounds) = Cues::recording();
    let state = crate::test_support::daemon_state_with_cues(cues);
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let told = bench.tell();
    let capture = Arc::new(Capture::new(CaptureSource::unopened()));
    let owner = spawn(Arc::clone(&state), capture, bench.clone());
    assert_eq!(told.recv_timeout(WAIT), Ok(select()), "the seed");

    assert!(state.record_start());
    assert_eq!(told.recv_timeout(WAIT), Ok(select()));
    assert_eq!(told.recv_timeout(WAIT), Ok(open(BUILT_IN)));
    bench.play(&[0.0; 160]);
    bench.play(&[0.25; 160]);
    assert_eq!(sounds.recv_timeout(WAIT), Ok(Cue::RecordStart));

    state.record_stop();
    assert_eq!(told.recv_timeout(WAIT), Ok(close(BUILT_IN)));
    owner.stop();
}

#[test]
fn a_named_device_gone_between_uses_is_named_while_the_default_records() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN, "Yeti"], Some(BUILT_IN));
    state.set_wanted_device("Yeti".to_string());
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();
    assert!(state.record_start());
    owner.step(now);
    state.record_stop();
    owner.step(now);

    bench.unplug("Yeti");
    assert!(state.record_start());
    owner.step(now);

    let yeti = BenchCall::Select("Yeti".to_string());
    assert_eq!(
        bench.asked(),
        [
            yeti.clone(),
            open("Yeti"),
            close("Yeti"),
            yeti,
            open(BUILT_IN)
        ]
    );
    assert_eq!(state.missing_device().as_deref(), Some("Yeti"));
    assert_eq!(state.mic(), Mic::Open);
}

// A headset's first callback came 163 ms after the build in measurement. A
// stream that has not called back by the first tick is not dead yet.
#[test]
fn a_stream_that_has_not_called_back_by_the_first_tick_is_kept() {
    let state = test_state();
    let bench = Bench::with(&[BUILT_IN], Some(BUILT_IN));
    let (mut owner, _) = owner_of(&state, &bench);
    let now = Instant::now();
    assert!(state.record_start());
    owner.step(now);

    owner.step(now + TICK);

    assert_eq!(bench.asked(), [select(), open(BUILT_IN)]);
}
