pub mod cues;
pub mod utils;
pub mod watchdog;
use banshee_common::{InputDevice, error::BansheeError};

use std::sync::Arc;

use cpal::{
    Stream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use ringbuf::{
    HeapCons, HeapRb,
    traits::{Producer, Split},
};

use crate::state::DaemonState;

pub const RING_SECS: usize = 120;

// The name [audio] input_device carries to mean "whatever the OS is set to"
pub const DEFAULT_INPUT_DEVICE: &str = "default";

/// A blank setting means the OS default. Without this an empty config value
/// matches the first device by substring.
fn wanted_or_default(setting: &str) -> &str {
    let setting = setting.trim();
    if setting.is_empty() {
        DEFAULT_INPUT_DEVICE
    } else {
        setting
    }
}

pub fn follows_os_default(setting: &str) -> bool {
    wanted_or_default(setting) == DEFAULT_INPUT_DEVICE
}

/// The name the OS calls its default input device, and nothing else. It costs
/// about 17 ms.
///
/// This is not the walk below, and it must not become it. The walk costs about
/// 176 ms, which no timer can pay. Asking "is the OS default still the open
/// device?" needs this call alone, so a timer can ask it every few seconds.
pub fn default_input_name() -> Option<String> {
    cpal::default_host()
        .default_input_device()
        .and_then(|device| device.description().ok())
        .map(|description| description.name().to_string())
}

/// One walk of the input devices, with the name the OS calls its default. It
/// costs about 176 ms, so one open pays for it once.
///
/// A device that will not describe itself is left out, because no caller can
/// name it.
fn walk_input_devices() -> (Vec<(String, cpal::Device)>, Option<String>) {
    let default = default_input_name();
    let Ok(devices) = cpal::default_host().input_devices() else {
        return (Vec::new(), default);
    };
    let devices = devices
        .filter_map(|device| {
            let name = device.description().ok()?.name().to_string();
            Some((name, device))
        })
        .collect();
    (devices, default)
}

/// Names only: opening each device to prove it records would steal the
/// microphone from a running daemon.
pub fn input_devices() -> Vec<InputDevice> {
    let (devices, default) = walk_input_devices();
    devices
        .into_iter()
        .map(|(name, _)| InputDevice {
            default: Some(&name) == default.as_ref(),
            name,
        })
        .collect()
}

/// Which of the present devices the wanted name opens.
///
/// An exact name wins, so a full name that a longer name contains still opens
/// its own device. A substring falls back, so a config can say "yeti" not
/// "Blue Yeti Stereo Microphone". Case insensitive.
fn match_input_device(present: &[String], wanted: &str) -> Option<usize> {
    let wanted = wanted.to_lowercase();
    let mut partial = None;
    for (index, name) in present.iter().enumerate() {
        let name = name.to_lowercase();
        if name == wanted {
            return Some(index);
        }
        if partial.is_none() && name.contains(&wanted) {
            partial = Some(index);
        }
    }
    partial
}

// "none" rather than an empty tail, so the message reads as a sentence
fn available(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// Which device a rebind should open.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// The wanted device, or the OS default when the config asks for it.
    Device(String),
    /// The wanted device is absent, so this one records in its place.
    Substitute { open: String, missing: String },
    /// Nothing at all can be opened; the reason goes to the client.
    Unavailable(String),
}

/// An absent named device falls back to the OS default so dictation keeps
/// working; Substitute names what is missing.
fn choose(wanted: &str, present: &[String], default: Option<&str>) -> Target {
    let wanted = wanted_or_default(wanted);
    if wanted == DEFAULT_INPUT_DEVICE {
        return match default {
            Some(name) => Target::Device(name.to_string()),
            None => Target::Unavailable("no input device is available".to_string()),
        };
    }

    if let Some(index) = match_input_device(present, wanted) {
        return Target::Device(present[index].clone());
    }
    match default {
        Some(name) => Target::Substitute {
            open: name.to_string(),
            missing: wanted.to_string(),
        },
        None => Target::Unavailable(format!(
            "no input device matching \"{wanted}\"; available: {}",
            available(present)
        )),
    }
}

pub struct Selection {
    pub device: cpal::Device,
    pub open: String,
    pub missing: Option<String>,
}

/// Where in the list the choice lands, so no caller resolves the name a second
/// time.
fn pick(
    names: &[String],
    wanted: &str,
    default: Option<&str>,
) -> Result<(usize, Option<String>), String> {
    let (open, missing) = match choose(wanted, names, default) {
        Target::Device(open) => (open, None),
        Target::Substitute { open, missing } => (open, Some(missing)),
        Target::Unavailable(reason) => return Err(reason),
    };
    // Exact, and no fallback: the OS default can be absent from the list, and
    // a looser match here opens a device that the choice did not name.
    let index = names.iter().position(|name| *name == open).ok_or_else(|| {
        format!(
            "the default input device \"{open}\" is not listed; available: {}",
            available(names)
        )
    })?;
    Ok((index, missing))
}

/// Capture's only device-list read.
pub fn select(wanted: &str) -> Result<Selection, String> {
    let (mut devices, default) = walk_input_devices();
    let names: Vec<String> = devices.iter().map(|(name, _)| name.clone()).collect();
    let (index, missing) = pick(&names, wanted, default.as_deref())?;
    let (open, device) = devices.swap_remove(index);
    Ok(Selection {
        device,
        open,
        missing,
    })
}

/// True when the open stream already serves the target, so the tick does
/// nothing. A rescan that reopens the held device allocates a new ring every
/// retry, and a substitution rescans forever while the named device is absent.
pub fn already_serving(target: &str, open_device: Option<&str>, stalled: bool) -> bool {
    !stalled && open_device == Some(target)
}

// Shared so the probe fails wherever capture would, down to the sample format.
fn build_and_play<D>(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    data: D,
) -> Result<Stream, BansheeError>
where
    D: FnMut(&[f32], &cpal::InputCallbackInfo) + Send + 'static,
{
    let stream = device
        .build_input_stream(
            &config.into(),
            data,
            |error| eprintln!("Audio Error: {error}"),
            None,
        )
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    stream
        .play()
        .map_err(|e| BansheeError::Other(e.to_string()))?;
    Ok(stream)
}

/// Enumeration is not proof: a device can list itself and still fail
/// `hw_params` when opened.
///
/// The error is the reason text alone. The checklist prints it to a user, and an
/// absent microphone is not an internal error.
pub fn probe_input_device(input_device: &str) -> Result<(String, Option<String>), String> {
    let selection = select(input_device)?;
    let config = selection
        .device
        .default_input_config()
        .map_err(|e| e.to_string())?;
    // Dropped at once: opening and starting it is the whole proof
    let stream = build_and_play(&selection.device, config, |_, _| {}).map_err(|e| e.to_string())?;
    drop(stream);
    Ok((selection.open, selection.missing))
}

pub struct Capture {
    pub stream: Stream,
    pub consumer: HeapCons<f32>,
    pub sample_rate: u32,
}

/// Opens the device `select` resolved. It takes the whole selection, so the
/// device that records and the name that is reported cannot disagree.
pub fn open_capture(
    daemon_state: Arc<DaemonState>,
    selection: &Selection,
) -> Result<Capture, BansheeError> {
    let device = &selection.device;
    let config = device
        .default_input_config()
        .map_err(|e| BansheeError::Other(e.to_string()))?;

    let sample_rate = config.sample_rate();
    let channels = config.channels();

    let ring_capacity = sample_rate as usize * RING_SECS;
    let (mut producer, consumer) = HeapRb::<f32>::new(ring_capacity).split();

    let capture_state = Arc::clone(&daemon_state);
    // Runs on the real-time audio thread, so it must not allocate: downmixing
    // through an iterator keeps the mono copy out of the heap
    let stream = build_and_play(device, config, move |data: &[f32], _| {
        capture_state.mark_capture_alive();
        if capture_state.is_recording() {
            if channels > 1 {
                producer.push_iter(
                    data.chunks(channels as usize)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32),
                );
            } else {
                producer.push_slice(data);
            }
        }
    })?;

    // Set after play() succeeds, so status never names a mic that failed to open
    daemon_state.set_audio_device(Some(selection.open.clone()));

    Ok(Capture {
        stream,
        consumer,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn default_follows_the_os() {
        let present = devices(&["MacBook Pro Microphone", "OnePlus Buds 3"]);
        assert_eq!(
            choose("default", &present, Some("OnePlus Buds 3")),
            Target::Device("OnePlus Buds 3".to_string())
        );
        // The headset goes, so the OS moves the default and capture follows it
        let present = devices(&["MacBook Pro Microphone"]);
        assert_eq!(
            choose("default", &present, Some("MacBook Pro Microphone")),
            Target::Device("MacBook Pro Microphone".to_string())
        );
    }

    #[test]
    fn default_with_no_device_at_all_is_unavailable() {
        assert!(matches!(
            choose("default", &[], None),
            Target::Unavailable(_)
        ));
    }

    #[test]
    fn a_named_device_is_found_by_substring_and_ignores_case() {
        let present = devices(&["MacBook Pro Microphone", "Blue Yeti Stereo Microphone"]);
        assert_eq!(
            choose("yeti", &present, Some("MacBook Pro Microphone")),
            Target::Device("Blue Yeti Stereo Microphone".to_string())
        );
        assert_eq!(
            choose("YETI", &present, Some("MacBook Pro Microphone")),
            Target::Device("Blue Yeti Stereo Microphone".to_string())
        );
    }

    #[test]
    fn a_named_device_that_is_gone_falls_back_to_the_default() {
        let present = devices(&["MacBook Pro Microphone"]);
        assert_eq!(
            choose("yeti", &present, Some("MacBook Pro Microphone")),
            Target::Substitute {
                open: "MacBook Pro Microphone".to_string(),
                missing: "yeti".to_string(),
            }
        );
    }

    #[test]
    fn a_named_device_that_is_gone_with_no_default_is_unavailable() {
        assert!(matches!(choose("yeti", &[], None), Target::Unavailable(_)));
    }

    #[test]
    fn an_exact_name_wins_over_a_longer_name_that_contains_it() {
        let present = devices(&["Blue Yeti Pro", "Yeti"]);
        assert_eq!(match_input_device(&present, "Yeti"), Some(1));
        assert_eq!(match_input_device(&present, "yeti"), Some(1));
        // choose shares that rule, so it cannot drift from the open path
        assert_eq!(
            choose("Yeti", &present, Some("Blue Yeti Pro")),
            Target::Device("Yeti".to_string())
        );
    }

    #[test]
    fn a_partial_name_matches_the_first_device_that_contains_it() {
        let present = devices(&["MacBook Pro Microphone", "Blue Yeti Stereo Microphone"]);
        assert_eq!(match_input_device(&present, "yeti"), Some(1));
    }

    #[test]
    fn a_name_that_matches_no_device_matches_nothing() {
        let present = devices(&["MacBook Pro Microphone"]);
        assert_eq!(match_input_device(&present, "yeti"), None);
    }

    #[test]
    fn a_blank_setting_is_the_default() {
        assert_eq!(wanted_or_default(""), DEFAULT_INPUT_DEVICE);
        assert_eq!(wanted_or_default("   "), DEFAULT_INPUT_DEVICE);
        assert_eq!(wanted_or_default("\t\n"), DEFAULT_INPUT_DEVICE);
        assert_eq!(wanted_or_default("yeti"), "yeti");
        assert_eq!(wanted_or_default(" yeti "), "yeti");
    }

    // The gate on the OS default check: a named device is never given up for
    // the device the OS chose.
    #[test]
    fn only_the_default_setting_follows_the_os() {
        assert!(follows_os_default(DEFAULT_INPUT_DEVICE));
        assert!(follows_os_default(""));
        assert!(follows_os_default("   "));
        assert!(!follows_os_default("yeti"));
        assert!(!follows_os_default("MacBook Pro Microphone"));
    }

    #[test]
    fn a_blank_setting_does_not_match_the_first_device() {
        let present = devices(&["MacBook Pro Microphone", "Blue Yeti Stereo Microphone"]);
        let default = Some("Blue Yeti Stereo Microphone");
        let expected = choose(DEFAULT_INPUT_DEVICE, &present, default);
        assert_eq!(choose("", &present, default), expected);
        assert_eq!(choose("   ", &present, default), expected);
        assert_eq!(
            expected,
            Target::Device("Blue Yeti Stereo Microphone".to_string())
        );
    }

    struct Held {
        name: &'static str,
        target: &'static str,
        open_device: Option<&'static str>,
        stalled: bool,
        serving: bool,
    }

    // A rescan that reopens the device it already holds rebuilds a 23 MB ring
    #[test]
    fn an_open_device_is_reopened_only_when_it_must_be() {
        let cases = [
            Held {
                name: "the same device on a healthy stream is left alone",
                target: "MacBook Pro Microphone",
                open_device: Some("MacBook Pro Microphone"),
                stalled: false,
                serving: true,
            },
            Held {
                name: "the same device on a stalled stream rebuilds",
                target: "MacBook Pro Microphone",
                open_device: Some("MacBook Pro Microphone"),
                stalled: true,
                serving: false,
            },
            Held {
                name: "a different device rebuilds",
                target: "Blue Yeti Stereo Microphone",
                open_device: Some("MacBook Pro Microphone"),
                stalled: false,
                serving: false,
            },
            Held {
                name: "nothing open rebuilds",
                target: "MacBook Pro Microphone",
                open_device: None,
                stalled: false,
                serving: false,
            },
            Held {
                name: "nothing open and stalled rebuilds",
                target: "MacBook Pro Microphone",
                open_device: None,
                stalled: true,
                serving: false,
            },
        ];

        for case in cases {
            assert_eq!(
                already_serving(case.target, case.open_device, case.stalled),
                case.serving,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn the_open_takes_the_device_the_choice_named() {
        // Two names contain the wanted one, and the answer sits between them:
        // resolving the choice a second time is how capture lands on another
        // device, and the position is pinned as well as the name
        let present = devices(&["Blue Yeti Pro", "Yeti", "Yeti Nano"]);
        let (index, missing) = pick(&present, "Yeti", Some("Blue Yeti Pro")).expect("a device");
        assert_eq!(index, 1, "the choice and the open must agree");
        assert_eq!(missing, None);
    }

    #[test]
    fn a_substitute_carries_the_device_that_records() {
        let present = devices(&["MacBook Pro Microphone"]);
        let (index, missing) =
            pick(&present, "yeti", Some("MacBook Pro Microphone")).expect("a device");
        assert_eq!(index, 0);
        assert_eq!(missing.as_deref(), Some("yeti"));
    }

    // The OS default can be absent from the listing. Nothing opens, because a
    // looser match records from a device the user never asked for.
    #[test]
    fn a_choice_the_list_does_not_hold_opens_nothing() {
        let present = devices(&["Blue Yeti Pro"]);
        let reason = pick(&present, "default", Some("Yeti")).expect_err("no device");
        assert!(reason.contains("Yeti"), "the reason names the device");
        assert!(reason.contains("Blue Yeti Pro"), "and what is there");
    }

    #[test]
    fn nothing_to_open_carries_the_reason() {
        let reason =
            pick(&devices(&["MacBook Pro Microphone"]), "yeti", None).expect_err("no device");
        assert!(reason.contains("yeti"));
        assert!(reason.contains("MacBook Pro Microphone"));
    }

    #[test]
    fn the_unavailable_reason_lists_what_is_there() {
        let present = devices(&["MacBook Pro Microphone"]);
        let Target::Unavailable(reason) = choose("yeti", &present, None) else {
            panic!("expected unavailable");
        };
        assert!(reason.contains("MacBook Pro Microphone"));
        assert!(
            reason.contains("yeti"),
            "the reason must name what was asked"
        );
    }
}
