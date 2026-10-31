pub mod download;
pub mod licences;
use banshee_common::{Blocker, BlockerKind, error::BansheeError, listening::ListeningModel};

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

/// Threads past this count do not speed up synthesis or listening.
const THREAD_CAP: usize = 8;

/// Threads pay for synthesis and for listening, and a machine contends past
/// its own core count.
pub fn thread_count() -> usize {
    // A machine that cannot say how many cores it holds is asked for one
    std::thread::available_parallelism().map_or(1, |cores| threads_for(cores.get()))
}

fn threads_for(cores: usize) -> usize {
    cores.min(THREAD_CAP)
}

pub fn model_path(name: &str) -> Result<std::path::PathBuf, BansheeError> {
    model_in(&download::models_dir()?, name)
}

/// Where a model file is, so every engine refuses a missing one the same way.
pub fn model_in(dir: &std::path::Path, name: &str) -> Result<std::path::PathBuf, BansheeError> {
    let path = dir.join(name);
    if !path.exists() {
        return Err(BansheeError::Other(format!(
            "{} is not in the models directory. Run 'banshee setup' to download it.",
            name_of(name)
        )));
    }
    Ok(path)
}

/// The model the listener loads, or none: a remote listener loads no file.
pub fn stt_model(config: &Config) -> Option<&'static ListeningModel> {
    match config.stt.provider {
        Provider::Local => config.stt.listening_model(),
        Provider::Remote => None,
    }
}

/// Names the suggestions, the three closest ids and where to see every one.
pub fn unknown_model(id: &str) -> String {
    use banshee_common::listening::{CATALOGUE, SUGGESTED};
    let asked = id.to_ascii_lowercase();
    let suggested: Vec<&str> = SUGGESTED.iter().map(|model| model.id).collect();
    let mut others: Vec<(f64, &str)> = CATALOGUE
        .iter()
        .filter(|model| !suggested.contains(&model.id))
        .map(|model| {
            let likeness = strsim::normalized_levenshtein(&asked, &model.id.to_ascii_lowercase());
            (likeness, model.id)
        })
        .collect();
    others.sort_by(|a, b| b.0.total_cmp(&a.0));
    let closest: Vec<&str> = others.into_iter().take(3).map(|(_, id)| id).collect();
    format!(
        "'{id}' is not a listening model Banshee knows. Suggested: {}. Closest: {}. Every model: banshee setup --list",
        suggested.join(", "),
        closest.join(", ")
    )
}

/// A file as a person names it: a listening model by its id, any other by its file.
pub fn name_of(file: &str) -> String {
    match banshee_common::listening::find_file(file) {
        Some(model) => format!("the listening model {}", model.id),
        None => file.to_string(),
    }
}

/// The models the recording pipeline loads at startup, named in one place so a
/// preflight and the daemon cannot disagree about what has to be on disk.
pub fn required(config: &Config) -> Vec<&'static str> {
    stt_model(config)
        .map(|model| model.file)
        .into_iter()
        .chain([VAD_MODEL])
        .collect()
}

/// Whisper files that nothing loads.
const OLD_WHISPER_FILES: [&str; 3] = [
    "ggml-base.en.bin",
    "ggml-large-v3-turbo-q5_0.bin",
    "ggml-large-v3-q5_0.bin",
];

/// Answers the files it deleted.
pub fn delete_old_whisper_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    OLD_WHISPER_FILES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| std::fs::remove_file(path).is_ok())
        .collect()
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

// An old Whisper file shares this directory and the `.bin` extension until the
// daemon's start deletes it, so only the name separates it from a voice.
// read_dir returns them in no order.
fn voices_among(files: impl Iterator<Item = String>) -> Vec<String> {
    let mut voices: Vec<String> = files
        .filter_map(|file| {
            let id = file.strip_suffix(".bin")?;
            (!OLD_WHISPER_FILES.contains(&file.as_str())).then(|| id.to_string())
        })
        .collect();
    voices.sort();
    voices
}

/// The Kokoro voices on disk: an undownloaded voice cannot be spoken with.
pub fn installed_voices_in(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    voices_among(entries.filter_map(|entry| Some(entry.ok()?.file_name().to_str()?.to_string())))
}

pub fn blockers(names: &[&str]) -> Vec<Blocker> {
    let Ok(dir) = download::models_dir() else {
        return Vec::new();
    };
    blockers_in(&dir, names)
}

pub fn blockers_in(dir: &std::path::Path, names: &[&str]) -> Vec<Blocker> {
    missing_in(dir, names)
        .into_iter()
        .map(|name| Blocker {
            kind: BlockerKind::Model,
            // A client cannot tell the speech model from the detector by filename without
            // re-deriving the daemon's rule
            role: Some(crate::models::download::role(&name)),
            remedy: Some(banshee_common::Remedy::Download),
            name: crate::audio::cues::capitalised(&name_of(&name)),
            id: name,
            consequence: "recording, dictation, and ask_user do not work".to_string(),
            fix: "run: banshee setup".to_string(),
            command: Some("banshee setup".to_string()),
        })
        .collect()
}

#[cfg(test)]
mod required_tests {
    use super::{VAD_MODEL, required, stt_model};
    use crate::config::{Config, Provider};

    #[test]
    fn a_local_listener_needs_its_model_file_and_the_detector() {
        let config = Config::default();
        assert_eq!(
            required(&config),
            vec!["parakeet-unified-en-0.6b-Q8_0.gguf", VAD_MODEL]
        );
    }

    #[test]
    fn a_listening_model_file_is_named_by_its_id() {
        assert_eq!(
            super::name_of("parakeet-unified-en-0.6b-Q8_0.gguf"),
            "the listening model parakeet-unified-en-0.6b"
        );
        assert_eq!(super::name_of(VAD_MODEL), VAD_MODEL);
    }

    #[test]
    fn a_remote_listener_needs_the_detector_alone() {
        let mut config = Config::default();
        config.stt.provider = Provider::Remote;
        assert_eq!(stt_model(&config), None);
        assert_eq!(required(&config), vec![VAD_MODEL]);
    }
}

#[cfg(test)]
mod tests {
    use banshee_common::BlockerKind;

    const ABSENT: &str = "no-such-model-9f3a.bin";

    #[test]
    fn missing_reports_a_model_that_is_not_on_disk() {
        assert_eq!(super::missing(&[ABSENT]), vec![ABSENT.to_string()]);
    }

    #[test]
    fn an_old_ggml_file_is_not_a_voice() {
        let files = ["ggml-large-v3-turbo-q5_0.bin", "af_sky.bin"]
            .into_iter()
            .map(str::to_string);
        assert_eq!(super::voices_among(files), ["af_sky"]);
    }

    #[test]
    fn the_old_whisper_files_are_deleted() {
        let dir = crate::test_support::unique_scratch("old-whisper");
        let old = [
            "ggml-base.en.bin",
            "ggml-large-v3-turbo-q5_0.bin",
            "ggml-large-v3-q5_0.bin",
        ];
        for name in old.iter().chain(&["notes.bin"]) {
            std::fs::write(dir.join(name), b"x").unwrap();
        }

        let mut deleted = super::delete_old_whisper_files(&dir);
        deleted.sort();
        let mut expected: Vec<_> = old.iter().map(|name| dir.join(name)).collect();
        expected.sort();
        assert_eq!(deleted, expected);
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(left, ["notes.bin"]);
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

    #[test]
    fn a_missing_listening_model_is_named_by_its_id_and_routed_by_its_file() {
        let file = "parakeet-unified-en-0.6b-Q8_0.gguf";
        let empty = crate::test_support::unique_scratch("blockers-in");
        let blockers = super::blockers_in(&empty, &[file]);
        let _ = std::fs::remove_dir_all(&empty);
        assert_eq!(
            blockers[0].name,
            "The listening model parakeet-unified-en-0.6b"
        );
        assert_eq!(blockers[0].id, file);

        let dir = crate::test_support::unique_scratch("model-in");
        let error = super::model_in(&dir, file).expect_err("nothing is downloaded");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            error
                .to_string()
                .contains("listening model parakeet-unified-en-0.6b"),
            "{error}"
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
