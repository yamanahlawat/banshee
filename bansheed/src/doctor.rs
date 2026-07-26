use std::path::Path;
use std::time::Duration;

use banshee_common::{KokoroTTSConfig, error::BansheeError, utils};
use cpal::traits::DeviceTrait;

use crate::config::{Config, TTSFallback};

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

    // Device presence only; a real TCC mic-permission check needs AVFoundation
    match crate::audio::resolve_input_device(&config.audio.input_device) {
        Ok(device) => {
            let name = device
                .description()
                .map(|d| d.name().to_string())
                .unwrap_or_else(|_| "default".to_string());
            pass(&format!("microphone: {name}"));
        }
        Err(e) => {
            healthy &= fail(
                &format!("input device unavailable: {e}"),
                "connect a microphone, or fix [audio] input_device in config.toml",
            );
        }
    }

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
    }

    // Two separate capabilities, and wayland only restores one of them:
    // wtype/ydotool cover typing the transcription, but nothing covers rdev's
    // global hotkey, which needs X11's XRecord extension.
    // Same cfg as `is_wayland` itself, so no unix target can disable its hotkey
    // without this block compiled in to explain why.
    #[cfg(all(unix, not(target_os = "macos")))]
    if crate::dictation::is_wayland() {
        // Reads the table dictation actually runs, so doctor cannot name a tool
        // the typing path would not reach for.
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

    match utils::get_socket_path() {
        Some(socket) if socket.exists() => {
            if crate::daemon::socket_answers(&socket) {
                healthy &= check_daemon_version().await;
            } else {
                note("stale socket from a crash; banshee start cleans it up");
            }
        }
        _ => note("daemon not running (start with: banshee start)"),
    }

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

// Walks $PATH directly: `which` is a package of its own on minimal systems,
// and spawning a process to answer a filesystem question is wasteful anyway.
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

async fn check_daemon_version() -> bool {
    let status = tokio::time::timeout(
        Duration::from_secs(2),
        utils::call_daemon(banshee_common::BANSHEE_STATUS, serde_json::json!({})),
    )
    .await;
    match status {
        Ok(Ok(status)) => {
            let version = status
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
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
        Ok(Err(e)) => fail(
            &format!("daemon answered the socket but status failed: {e}"),
            "restart it: banshee start",
        ),
        Err(_) => fail(
            "daemon did not answer status within 2s",
            "restart it: banshee start",
        ),
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
