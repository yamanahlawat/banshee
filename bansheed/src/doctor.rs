use std::path::Path;
use std::time::Duration;

use banshee_common::{KokoroTTSConfig, error::BansheeError, utils};
use cpal::traits::{DeviceTrait, HostTrait};

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
        (false, TTSFallback::System) => note(
            "kokoro tts model missing, will fall back to system voice (run: banshee setup)",
        ),
        (false, TTSFallback::None) => {
            healthy &= fail(
                "kokoro tts model missing and [tts] fallback = \"none\"",
                "run: banshee setup",
            );
        }
    }

    // Device presence only; a real TCC mic-permission check needs AVFoundation
    match cpal::default_host().default_input_device() {
        Some(device) => {
            let name = device
                .description()
                .map(|d| d.name().to_string())
                .unwrap_or_else(|_| "default".to_string());
            pass(&format!("microphone: {name}"));
        }
        None => {
            healthy &= fail(
                "no input device found",
                "connect a microphone or check sound settings",
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        healthy &= if accessibility_trusted() {
            pass("accessibility permission granted")
        } else {
            fail(
                "accessibility permission missing (hotkey and dictation will not work)",
                "grant it in System Settings > Privacy & Security > Accessibility; unsigned debug builds lose the grant on every rebuild",
            )
        };
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

#[cfg(target_os = "macos")]
fn accessibility_trusted() -> bool {
    // Boolean in C is an unsigned char, so u8 instead of bool keeps the call sound
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    unsafe { AXIsProcessTrusted() != 0 }
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
