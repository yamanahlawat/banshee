pub mod download;

use banshee_common::{Blocker, BlockerKind};

use crate::config::Config;

pub const VAD_MODEL: &str = "silero_vad.onnx";

/// The models the recording pipeline loads at startup, named in one place so a
/// preflight and the daemon cannot disagree about what has to be on disk.
pub fn required(config: &Config) -> [&'static str; 2] {
    [config.stt.preset.model_name(), VAD_MODEL]
}

pub fn missing(names: &[&str]) -> Vec<String> {
    let Some(dir) = banshee_common::utils::get_models_path() else {
        return Vec::new();
    };
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
    let Some(dir) = banshee_common::utils::get_models_path() else {
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
mod blocker_tests {
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
}
