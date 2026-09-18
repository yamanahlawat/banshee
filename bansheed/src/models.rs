pub mod download;
use banshee_common::{Blocker, BlockerKind, error::BansheeError};

use crate::config::{Config, Provider};

pub const VAD_MODEL: &str = "silero_vad.onnx";

/// One ONNX session, built the same way for every model that needs one.
pub fn onnx_session(
    path: &std::path::Path,
    threads: usize,
    entries: &[(&str, &str)],
) -> Result<ort::session::Session, BansheeError> {
    fn fault(error: impl std::fmt::Display) -> BansheeError {
        BansheeError::Other(error.to_string())
    }
    let mut builder = ort::session::Session::builder()
        .map_err(fault)?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::All)
        .map_err(fault)?
        .with_intra_threads(threads)
        .map_err(fault)?;
    for (key, value) in entries {
        builder = builder.with_config_entry(key, value).map_err(fault)?;
    }
    builder.commit_from_file(path).map_err(fault)
}

/// Measured flat from 1 to 16 threads: a 512-sample chunk is too small for
/// threading to reach.
pub const VAD_THREADS: usize = 1;

/// The highest count measured to pay. Past it the curve is the machine's.
const KOKORO_THREAD_CAP: usize = 8;

/// Threads pay for synthesis, and a machine contends past its own core count.
pub fn kokoro_threads() -> usize {
    // A machine that cannot say how many cores it holds is asked for one
    std::thread::available_parallelism().map_or(1, |cores| threads_for(cores.get()))
}

fn threads_for(cores: usize) -> usize {
    cores.min(KOKORO_THREAD_CAP)
}

/// Where a model file is, so every engine refuses a missing one the same way.
pub fn model_path(name: &str) -> Result<std::path::PathBuf, BansheeError> {
    let path = download::models_dir()?.join(name);
    if !path.exists() {
        return Err(BansheeError::Other(format!(
            "{name} is not in the models directory. Run 'banshee setup' to download it."
        )));
    }
    Ok(path)
}

/// The Whisper file the listener loads, or none: a remote listener loads no file.
pub fn stt_file(config: &Config) -> Option<&'static str> {
    match config.stt.provider {
        Provider::Local => Some(config.stt.preset.model_name()),
        Provider::Remote => None,
    }
}

/// The models the recording pipeline loads at startup, named in one place so a
/// preflight and the daemon cannot disagree about what has to be on disk.
pub fn required(config: &Config) -> Vec<&'static str> {
    stt_file(config).into_iter().chain([VAD_MODEL]).collect()
}

/// With no home directory nothing is missing and no voice is installed: the
/// engines that load from the directory refuse through `model_path` instead.
pub fn missing(names: &[&str]) -> Vec<String> {
    let Ok(dir) = download::models_dir() else {
        return Vec::new();
    };
    missing_in(&dir, names)
}

pub fn missing_in(dir: &std::path::Path, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|name| !dir.join(name).exists())
        .map(|name| (*name).to_string())
        .collect()
}

// The speech models share this directory and the `.bin` extension, so only the
// name separates them. read_dir returns them in no order.
fn voices_among(files: impl Iterator<Item = String>) -> Vec<String> {
    let mut voices: Vec<String> = files
        .filter_map(|file| {
            let id = file.strip_suffix(".bin")?;
            crate::config::STTPreset::ALL
                .iter()
                .all(|preset| preset.model_name() != file.as_str())
                .then(|| id.to_string())
        })
        .collect();
    voices.sort();
    voices
}

/// On-disk only: an undownloaded voice cannot be spoken with.
pub fn installed_voices() -> Vec<String> {
    let Ok(dir) = download::models_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    voices_among(entries.filter_map(|entry| Some(entry.ok()?.file_name().to_str()?.to_string())))
}

pub fn blockers(names: &[&str]) -> Vec<Blocker> {
    missing(names)
        .into_iter()
        .map(|name| Blocker {
            kind: BlockerKind::Model,
            // A client cannot tell the speech model from the detector by filename without
            // re-deriving the daemon's rule
            role: Some(crate::models::download::role(&name)),
            remedy: Some(banshee_common::Remedy::Download),
            name: name.clone(),
            id: name,
            consequence: "recording, dictation, and ask_user do not work".to_string(),
            fix: "run: banshee setup".to_string(),
            command: Some("banshee setup".to_string()),
        })
        .collect()
}

#[cfg(test)]
mod required_tests {
    use super::{VAD_MODEL, required, stt_file};
    use crate::config::{Config, Provider};

    #[test]
    fn a_local_listener_needs_its_whisper_file_and_the_detector() {
        let config = Config::default();
        assert_eq!(stt_file(&config), Some(config.stt.preset.model_name()));
        assert_eq!(
            required(&config),
            vec![config.stt.preset.model_name(), VAD_MODEL]
        );
    }

    #[test]
    fn a_remote_listener_needs_the_detector_alone() {
        let mut config = Config::default();
        config.stt.provider = Provider::Remote;
        assert_eq!(stt_file(&config), None);
        assert_eq!(required(&config), vec![VAD_MODEL]);
    }
}

#[cfg(test)]
mod tests {
    use crate::config::STTPreset;
    use banshee_common::BlockerKind;

    const ABSENT: &str = "no-such-model-9f3a.bin";

    #[test]
    fn missing_reports_a_model_that_is_not_on_disk() {
        assert_eq!(super::missing(&[ABSENT]), vec![ABSENT.to_string()]);
    }

    #[test]
    fn missing_reports_nothing_for_no_models() {
        assert!(super::missing(&[]).is_empty());
    }

    #[test]
    fn no_speech_model_is_mistaken_for_a_voice() {
        let models = STTPreset::ALL.iter().map(|p| p.model_name().to_string());
        assert!(
            super::voices_among(models).is_empty(),
            "every preset's model shares the directory and the extension"
        );
    }

    // Deliberately out of order, and mixed with what the directory really holds
    #[test]
    fn voices_come_back_sorted_with_everything_else_dropped() {
        let files = [
            "am_santa.bin",
            "kokoro-v1.0.onnx",
            "af_sky.bin",
            "silero_vad.onnx",
            "af_heart.bin",
        ]
        .into_iter()
        .map(str::to_string);
        assert_eq!(
            super::voices_among(files),
            ["af_heart", "af_sky", "am_santa"]
        );
    }

    #[test]
    fn a_blocker_names_the_model_a_client_has_to_fetch() {
        let blockers = super::blockers(&[ABSENT]);
        let [blocker] = &blockers[..] else {
            panic!("one absent model must raise exactly one blocker: {blockers:?}");
        };
        assert_eq!(blocker.kind, BlockerKind::Model);
        assert_eq!(blocker.id, ABSENT, "id must name the file to download");
        assert!(
            blocker.fix.contains("banshee setup"),
            "the fix must name the command that resolves it: {}",
            blocker.fix
        );
    }

    /// A client routes on `command`, so the literal is a wire contract and not
    /// an implementation detail of the sentence beside it.
    #[test]
    fn a_missing_model_names_the_command_a_client_routes_on() {
        let blockers = super::blockers(&["no-such-model-9f3a.bin"]);
        assert_eq!(blockers[0].command.as_deref(), Some("banshee setup"));
    }

    /// Only the daemon holds the list of speech models, so a client that has to
    /// tell one from the voice detector reads this rather than the filename.
    /// Which files are on disk decides nothing here: the role rides along
    /// whatever the blocker names.
    #[test]
    fn a_model_blocker_says_what_the_file_is() {
        const ABSENT: &str = "no-such-model-9f3a.bin";
        let blockers = super::blockers(&[ABSENT]);
        assert_eq!(blockers[0].role, Some(super::download::role(ABSENT)));
    }

    /// The extra threads contend on a machine that does not hold them, so the
    /// count follows the machine up to where the measurement stops.
    #[test]
    fn the_speech_engine_asks_for_no_more_threads_than_the_machine_holds() {
        assert_eq!(
            super::threads_for(2),
            2,
            "a dual-core machine is not oversubscribed"
        );
        assert_eq!(super::threads_for(8), 8);
        assert_eq!(
            super::threads_for(24),
            8,
            "8 is the highest count both machines measured"
        );
    }
}
