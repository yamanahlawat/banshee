pub mod download;

use banshee_common::{Blocker, BlockerKind};

use crate::config::Config;

/// The models the recording pipeline loads at startup, named in one place so a
/// preflight and the daemon cannot disagree about what has to be on disk.
pub fn required(config: &Config) -> [&'static str; 2] {
    [config.stt.preset.model_name(), crate::VAD_MODEL]
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

pub fn blockers(names: &[&str]) -> Vec<Blocker> {
    missing(names)
        .into_iter()
        .map(|name| Blocker {
            kind: BlockerKind::Model,
            // A model has no friendlier name than its filename
            name: name.clone(),
            id: name,
            consequence: "recording, dictation, and ask_user do not work".to_string(),
            fix: "run: banshee setup".to_string(),
        })
        .collect()
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
    fn missing_reports_nothing_for_no_models() {
        assert!(super::missing(&[]).is_empty());
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
