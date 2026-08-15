use std::path::Path;
use std::time::Duration;

use banshee_common::{KokoroTTSConfig, error::BansheeError, utils};

use crate::config::{BargeInMode, Config, HotkeyMode, STTPreset, TTSFallback};

// Read-only diagnosis: doctor never mutates, it reports and names the fix.
// Returns true when every required check passed.
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
            healthy &= fail(
                &format!("config.toml invalid: {e}"),
                "fix the TOML, or delete the file to use defaults",
            );
            Config::default()
        }
    };

    report_settings(&config);

    let Some(models_dir) = utils::get_models_path() else {
        fail("home directory not found", "set $HOME");
        return false;
    };
    for name in [config.stt.preset.model_name(), crate::VAD_MODEL] {
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

    // Fetched once: the microphone check and the version check both read it
    let daemon = probe_daemon().await;
    healthy &= check_recording(&daemon, &config.audio.input_device);

    #[cfg(target_os = "macos")]
    {
        healthy &= if crate::permissions::input_granted() {
            pass("accessibility permission granted")
        } else {
            fail(
                "accessibility permission missing (hotkey and dictation will not work)",
                "grant it in System Settings > Privacy & Security > Accessibility; the daemon restarts itself once it lands. unsigned debug builds lose the grant on every rebuild",
            )
        };

        // TCC answers for the responsible process, so a doctor run from a
        // granted terminal cannot speak for a launchd-started daemon
        healthy &= match crate::permissions::hotkey_events_granted() {
            crate::permissions::Access::Granted => pass("input monitoring granted to this process"),
            crate::permissions::Access::Denied => fail(
                "input monitoring permission missing (the hotkey gets no events, silently)",
                "grant it in System Settings > Privacy & Security > Input Monitoring, then restart the daemon",
            ),
            crate::permissions::Access::Undetermined => {
                note("input monitoring not decided yet; macOS asks the first time the daemon runs");
                true
            }
        };
    }

    // wtype/ydotool restore typing, but nothing restores rdev's global hotkey.
    // Same cfg as `is_wayland`, so every unix target that loses it explains why.
    #[cfg(all(unix, not(target_os = "macos")))]
    if crate::dictation::is_wayland() {
        // The table dictation actually runs, so doctor cannot name a stale tool
        let typer = crate::dictation::WAYLAND_TYPERS
            .into_iter()
            .map(|(binary, _)| binary)
            .find(|binary| on_path(binary));
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

    healthy &= report_daemon(&daemon);

    if let Some(service) = crate::service::service_file_path() {
        if service.exists() {
            note("start-at-login service installed");
        } else {
            note("no start-at-login service (install with: banshee start)");
        }
    }

    if healthy {
        println!("All checks passed.");
    } else {
        println!("Problems found, fixes listed above.");
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

// Walks $PATH directly: `which` is its own package on minimal systems.
#[cfg(all(unix, not(target_os = "macos")))]
fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(bin).is_file()))
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
enum Daemon {
    Running(serde_json::Value),
    Silent(String),
    Stale,
    Missing,
}

// status fields are optional by protocol, so every read needs a fallback
fn field<'a>(status: &'a serde_json::Value, key: &str, fallback: &'a str) -> &'a str {
    status.get(key).and_then(|v| v.as_str()).unwrap_or(fallback)
}

async fn probe_daemon() -> Daemon {
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
                Ok(Ok(status)) => Daemon::Running(status),
                Ok(Err(e)) => Daemon::Silent(e.to_string()),
                Err(_) => Daemon::Silent("no answer within 2s".to_string()),
            }
        }
        _ => Daemon::Missing,
    }
}

// A live daemon holds the device and already knows whether capture works, so
// ask it. Opening a second stream fails on backends that allow only one, which
// would report a broken microphone on a healthy machine. `Silent` counts as
// live: something answered the socket, so something owns the device.
fn check_recording(daemon: &Daemon, input_device: &str) -> bool {
    match daemon {
        Daemon::Running(status) => match status.get("recording_error").and_then(|v| v.as_str()) {
            None => pass(&format!(
                "recording works, daemon has the microphone: {}",
                field(status, "audio_device", "unnamed device")
            )),
            Some(reason) => fail(
                &format!("the daemon cannot record: {reason}"),
                "fix the cause named above, then restart: banshee start",
            ),
        },
        Daemon::Silent(_) => {
            note("microphone unchecked: a daemon holds it but did not answer status");
            true
        }
        // Nothing holds the device, so open it here: enumeration is not proof
        Daemon::Stale | Daemon::Missing => match crate::audio::probe_input_device(input_device) {
            Ok(name) => {
                let name = name.unwrap_or_else(|| "unnamed device".to_string());
                pass(&format!("microphone opens for capture: {name}"))
            }
            Err(e) => fail(
                &format!("microphone will not open: {e}"),
                "connect a microphone, or fix [audio] input_device in config.toml",
            ),
        },
    }
}

fn report_daemon(daemon: &Daemon) -> bool {
    match daemon {
        Daemon::Running(status) => {
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
        Daemon::Stale => {
            note("stale socket from a crash; banshee start cleans it up");
            true
        }
        Daemon::Missing => {
            note("daemon not running (start with: banshee start)");
            true
        }
    }
}

// Only fields the daemon reads, so a printed value is one in use.
fn report_settings(config: &Config) {
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
        "hotkey {hotkey_mode}, barge-in {barge_in}, cues {}",
        on_off(config.audio.cues.enabled)
    ));
    note(&format!(
        "stt {preset}, vad {}, endpoint {} ms, {} vocabulary terms",
        config.stt.vad_threshold,
        config.stt.endpoint_silence_ms,
        config.stt.vocabulary.len()
    ));
    note(&format!(
        "tts {} at {}x, history {}",
        config.tts.voice,
        config.tts.speed,
        on_off(config.daemon.save_history)
    ));
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
