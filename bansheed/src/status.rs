use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use banshee_common::{Blocker, BlockerKind, KokoroTTSConfig, error::BansheeError, utils};

use crate::config::{BargeInMode, Config, HotkeyMode, STTPreset, TTSFallback};
#[cfg(target_os = "macos")]
use crate::permissions::Access;

// Reports and names the fix, never mutates; true when every required check passed
pub async fn run(config: Result<Config, BansheeError>) -> bool {
    let mut healthy = true;

    let config = match config {
        Ok(config) => {
            let exists = utils::get_config_path().is_some_and(|p| p.exists());
            if exists {
                pass("config.toml parsed");
            } else {
                pass("no config.toml, using defaults");
            }
            config
        }
        Err(e) => {
            let path = utils::get_config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "config.toml".to_string());
            // First line only: toml's diagnostic runs five lines with a caret,
            // and `main` has already printed it in full
            let reason = e.to_string();
            let reason = reason.lines().next().unwrap_or("could not be read");
            healthy &= fail(
                &format!("config.toml invalid: {reason}"),
                &format!("edit {path}, or delete it to use defaults"),
            );
            Config::default()
        }
    };

    // Probed first: a live daemon knows a vad_threshold that was never written
    let daemon = probe_daemon().await;
    report_settings(&config, &daemon);

    let Some(models_dir) = utils::get_models_path() else {
        fail("home directory not found", "set $HOME");
        return false;
    };
    for name in crate::models::required(&config) {
        healthy &= check_model(&models_dir, name);
    }
    let kokoro = KokoroTTSConfig::new(&config.tts.voice);
    let kokoro_present = models_dir.join(&kokoro.model_name).exists()
        && models_dir.join(&kokoro.voice_name).exists();
    match (kokoro_present, &config.tts.fallback) {
        (true, _) => {
            pass(&format!(
                "kokoro tts model present (voice {})",
                config.tts.voice
            ));
        }
        (false, TTSFallback::System) => {
            note("kokoro tts model missing, will fall back to system voice (run: banshee setup)")
        }
        (false, TTSFallback::None) => {
            healthy &= fail(
                "kokoro tts model missing and [tts] fallback = \"none\"",
                "run: banshee setup",
            );
        }
    }

    check_espeak();

    // Before the checks that depend on it: a dead daemon causes the microphone
    // and permission failures below, and read after them it looks like a footnote
    healthy &= report_daemon(&daemon);

    healthy &= check_recording(&daemon, &config.audio.input_device);

    #[cfg(target_os = "macos")]
    {
        healthy &= check_permissions(&daemon);
    }

    // wtype/ydotool restore typing, but nothing restores rdev's global hotkey.
    // Same cfg as `is_wayland`, so every unix target that loses it explains why.
    #[cfg(all(unix, not(target_os = "macos")))]
    if crate::dictation::is_wayland() {
        // The table dictation actually runs, so the checklist cannot name a stale tool
        let path = crate::connect::resolved_path();
        let typer = crate::dictation::WAYLAND_TYPERS
            .into_iter()
            .map(|(binary, _)| binary)
            .find(|binary| resolve(binary, &path).is_some());
        match typer {
            Some(tool) => {
                pass(&format!("wayland session: dictation types via {tool}"));
            }
            None => note(
                "wayland session: install 'wtype' (or 'ydotool') or dictation cannot type anywhere",
            ),
        }
        note(&format!("wayland: {}", crate::hotkey::WAYLAND_HOTKEY_HINT));
    } else {
        match std::env::var("XDG_SESSION_TYPE").as_deref() {
            Ok(session) => {
                pass(&format!("session type: {session}"));
            }
            Err(_) => note("XDG_SESSION_TYPE unset; the global hotkey needs X11"),
        }
    }

    if let Some(service) = crate::service::service_file_path() {
        if service.exists() {
            note("start-at-login service installed");
        } else {
            note("no start-at-login service (install with: banshee start)");
        }
        #[cfg(target_os = "macos")]
        if let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize) {
            note(&format!("running from {}", install_shape(&exe)));
        }
    }

    if healthy {
        println!("All checks passed.");
    } else {
        // Causal order: a later failure is often an earlier one's symptom
        println!("Problems found. Work down from the top.");
    }
    healthy
}

// Optional dependency, so it reports but never fails the health check.
fn check_espeak() {
    if crate::text_to_speech::oov::OovFallback::available() {
        pass("espeak-ng present (pronounces unknown words)");
    } else {
        note(&format!(
            "espeak-ng not installed; unknown words are spelled out. install: {}",
            espeak_install_hint()
        ));
    }
}

fn espeak_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        return "brew install espeak-ng";
    }
    for (mgr, cmd) in [
        ("apt", "sudo apt install espeak-ng"),
        ("dnf", "sudo dnf install espeak-ng"),
        ("pacman", "sudo pacman -S espeak-ng"),
        ("zypper", "sudo zypper install espeak-ng"),
        ("apk", "sudo apk add espeak-ng"),
    ] {
        if runs(mgr) {
            return cmd;
        }
    }
    "your package manager's espeak-ng package"
}

fn runs(bin: &str) -> bool {
    std::process::Command::new(bin)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

// Walks the given PATH directly: `which` is its own package on minimal systems.
// Answers with the file, because a caller that has to run it cannot resolve the
// name again: a supervised process is handed a different PATH.
pub(crate) fn resolve(bin: &str, path: &OsStr) -> Option<PathBuf> {
    crate::connect::path_dirs(path)
        .map(|dir| dir.join(bin))
        .find(|candidate| runnable(candidate))
}

// A bare name let the kernel walk past a file it could not execute. An absolute
// path cannot, so the walk skips one here.
fn runnable(candidate: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(candidate)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    candidate.is_file()
}

fn check_model(models_dir: &Path, name: &str) -> bool {
    if models_dir.join(name).exists() {
        pass(&format!("model present: {name}"))
    } else {
        fail(&format!("model missing: {name}"), "run: banshee setup")
    }
}

// What the socket says about a daemon. Read once, because the microphone check
// and the version check both depend on it.
pub enum Daemon {
    Running {
        status: serde_json::Value,
        blockers: Vec<Blocker>,
    },
    /// Answered, but from a build before it reported blockers
    Legacy(serde_json::Value),
    Silent(String),
    Stale,
    Missing,
}

/// An absent blockers field is an older daemon; one that will not decode is a
/// daemon this build cannot read, which is not the same answer.
fn classify(status: serde_json::Value) -> Daemon {
    let Some(listed) = status.get("blockers") else {
        return Daemon::Legacy(status);
    };
    match serde_json::from_value::<Vec<Blocker>>(listed.clone()) {
        Ok(blockers) => Daemon::Running { status, blockers },
        Err(error) => Daemon::Silent(format!("its blockers could not be read: {error}")),
    }
}

fn unreported(what: &str) -> bool {
    note(&format!(
        "{what} unchecked: this daemon is older than this command and does not report it"
    ));
    true
}

#[cfg(target_os = "macos")]
fn check_permissions(daemon: &Daemon) -> bool {
    let healthy = check_grants(daemon);
    report_key_presses(daemon);
    healthy
}

/// The daemon measures this, for the same reason it speaks for its own grants.
#[cfg(target_os = "macos")]
fn report_key_presses(daemon: &Daemon) {
    let reported = match daemon {
        Daemon::Running { status, .. } => banshee_common::key_press_access(status),
        Daemon::Legacy(_) => None,
        // A read from here answers for the terminal, so with no daemon the
        // checklist says nothing rather than saying it under Banshee's name.
        _ => return,
    };
    let Some(word) = reported else {
        unreported("key press access");
        return;
    };
    match Access::from_wire(word) {
        Some(access) => note(key_press_line(access)),
        None => note(&format!(
            "the daemon reports key press access as '{word}', which this command cannot read"
        )),
    }
}

/// A note and never a fix: no pane in System Settings offers this one. A macOS
/// that starts to want the grant of its own flips the line.
#[cfg(target_os = "macos")]
fn key_press_line(access: Access) -> &'static str {
    match access {
        Access::Granted => "the daemon can receive key presses",
        Access::Denied => "the daemon cannot receive key presses",
        Access::Undetermined => "macOS has not decided whether the daemon can receive key presses",
    }
}

#[cfg(target_os = "macos")]
fn check_grants(daemon: &Daemon) -> bool {
    let denied: Vec<(&str, &str, &str)> = match daemon {
        Daemon::Running { blockers, .. } => {
            let denied: Vec<_> = blockers
                .iter()
                .filter(|blocker| blocker.kind == BlockerKind::Permission)
                .map(|blocker| {
                    (
                        blocker.name.as_str(),
                        blocker.consequence.as_str(),
                        blocker.fix.as_str(),
                    )
                })
                .collect();
            if denied.is_empty() {
                return pass("permissions granted to the daemon");
            }
            denied
        }
        Daemon::Legacy(_) => return unreported("permissions"),
        // A read from here answers for the terminal that ran the command.
        _ => {
            note("a grant cannot be read from here: only the daemon speaks for the daemon");
            return true;
        }
    };

    let mut healthy = true;
    for (name, consequence, fix) in denied {
        healthy &= fail(
            &format!("{name} missing for the daemon: {consequence}"),
            fix,
        );
    }
    healthy
}

// status fields are optional by protocol, so every read needs a fallback
fn field<'a>(status: &'a serde_json::Value, key: &str, fallback: &'a str) -> &'a str {
    status.get(key).and_then(|v| v.as_str()).unwrap_or(fallback)
}

pub async fn probe_daemon() -> Daemon {
    match utils::get_socket_path() {
        Some(socket) if socket.exists() => {
            if !crate::daemon::socket_answers(&socket) {
                return Daemon::Stale;
            }
            match tokio::time::timeout(
                Duration::from_secs(2),
                utils::call_daemon(banshee_common::BANSHEE_STATUS, serde_json::json!({})),
            )
            .await
            {
                Ok(Ok(status)) => classify(status),
                Ok(Err(e)) => Daemon::Silent(e.to_string()),
                Err(_) => Daemon::Silent("no answer within 2s".to_string()),
            }
        }
        _ => Daemon::Missing,
    }
}

// Core Audio does not say which of these it was: the one failure measured here
// returned kAudioHardwareBadObjectError, which a denied grant and a vanished
// device can both produce. All three are named because none can be ruled out.
#[cfg(target_os = "macos")]
const MICROPHONE_FIX: &str = "check Microphone is allowed in System Settings > \
     Privacy & Security, that the device is connected, and that [audio] input_device \
     names it";
#[cfg(not(target_os = "macos"))]
const MICROPHONE_FIX: &str = "connect a microphone, or fix [audio] input_device in config.toml";

// One line for both paths, so a substitution reads the same whether the daemon
// reports it or the checklist opens the device itself.
fn microphone_line(lead: &str, open: Option<&str>, missing: Option<&str>) -> String {
    format!(
        "{lead}: {}",
        banshee_common::microphone_label(open, missing)
    )
}

// Split out so the decision is testable without a microphone: a substitute
// records, so it passes, and only a device that will not open fails.
fn report_probe(probed: Result<(String, Option<String>), String>) -> bool {
    match probed {
        Ok((open, missing)) => pass(&microphone_line(
            "microphone opens for capture",
            Some(&open),
            missing.as_deref(),
        )),
        Err(e) => fail(&format!("microphone will not open: {e}"), MICROPHONE_FIX),
    }
}

// Opening a second stream fails on backends that allow only one, which would
// report a broken microphone on a healthy machine, so ask the daemon instead.
// `Silent` counts as live: something answered the socket, so something owns the
// device. Missing models suppress the pipeline blocker, so no blocker proves
// capture opened, not that recording works. A substitute records correctly, so
// it stays a pass.
fn check_recording(daemon: &Daemon, input_device: &str) -> bool {
    match daemon {
        Daemon::Running { status, blockers } => match blockers
            .iter()
            .find(|blocker| blocker.kind == BlockerKind::Pipeline)
        {
            None => pass(&microphone_line(
                "daemon has the microphone",
                banshee_common::audio_device(status),
                banshee_common::missing_device(status),
            )),
            Some(blocker) => fail(
                &format!("the daemon cannot record: {}", blocker.consequence),
                &blocker.fix,
            ),
        },
        Daemon::Legacy(_) => unreported("recording"),
        Daemon::Silent(_) => {
            note("microphone unchecked: a daemon holds it but did not answer status");
            true
        }
        // Nothing holds the device, so open it here: enumeration is not proof
        Daemon::Stale | Daemon::Missing => {
            report_probe(crate::audio::probe_input_device(input_device))
        }
    }
}

fn report_daemon(daemon: &Daemon) -> bool {
    match daemon {
        Daemon::Running { status, .. } | Daemon::Legacy(status) => {
            let version = field(status, "version", "unknown");
            pass(&format!("daemon running (version {version})"));
            if version != env!("CARGO_PKG_VERSION") {
                note(&format!(
                    "daemon is {} but this CLI is {}; reinstall and restart the daemon",
                    version,
                    env!("CARGO_PKG_VERSION")
                ));
            }
            true
        }
        Daemon::Silent(e) => fail(
            &format!("daemon answered the socket but status failed: {e}"),
            "restart it: banshee start",
        ),
        // Not notes: nothing records without a daemon, and a checklist that
        // passes here reports a green check it cannot back
        Daemon::Stale => fail(
            "the daemon is not running; a stale socket is left from a crash",
            "start it: banshee start",
        ),
        Daemon::Missing => fail("the daemon is not running", "start it: banshee start"),
    }
}

// Only fields the daemon reads, so a printed value is one in use.
fn report_settings(config: &Config, daemon: &Daemon) {
    let hotkey_mode = match config.audio.hotkey_mode {
        HotkeyMode::Hold => "hold",
        HotkeyMode::Toggle => "toggle",
    };
    let barge_in = match config.audio.barge_in {
        BargeInMode::Stop => "stop",
        BargeInMode::Duck => "duck",
        BargeInMode::None => "none",
    };
    let preset = match config.stt.preset {
        STTPreset::Fast => "fast",
        STTPreset::Balanced => "balanced",
        STTPreset::Quality => "quality",
    };
    let on_off = |enabled: bool| if enabled { "on" } else { "off" };

    note(&format!(
        "hotkey {} {hotkey_mode}, barge-in {barge_in}, cues {}",
        config.audio.hotkey,
        on_off(config.audio.cues.enabled)
    ));
    let vad_threshold = match daemon {
        Daemon::Running { status, .. } | Daemon::Legacy(status) => {
            status.get("vad_threshold").and_then(|v| v.as_f64())
        }
        _ => None,
    }
    .map_or(config.stt.vad_threshold, |live| live as f32);

    note(&format!(
        "stt {preset}, vad {}, endpoint {} ms, {} vocabulary {}",
        vad_threshold,
        config.stt.endpoint_silence_ms,
        config.stt.vocabulary.len(),
        if config.stt.vocabulary.len() == 1 {
            "term"
        } else {
            "terms"
        }
    ));
    note(&format!(
        "tts {} at {}x, history {}",
        config.tts.voice,
        config.tts.speed,
        on_off(config.daemon.save_history)
    ));
}

// Two install shapes exist on macOS, so status names the one that answered.
#[cfg(target_os = "macos")]
fn install_shape(exe: &std::path::Path) -> &'static str {
    if exe
        .components()
        .any(|part| part.as_os_str() == "Banshee.app")
    {
        "Banshee.app"
    } else {
        "a loose binary"
    }
}

fn pass(msg: &str) -> bool {
    println!("  ✓  {msg}");
    true
}

fn note(msg: &str) {
    println!("  -  {msg}");
}

fn fail(msg: &str, fix: &str) -> bool {
    println!("  ✗  {msg}");
    println!("     fix: {fix}");
    false
}

#[cfg(test)]
mod tests;
