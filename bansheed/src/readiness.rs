use banshee_common::{Blocker, BlockerKind};

use crate::state::{DaemonState, Pipeline, RecordingError};
use crate::{models, permissions};

pub fn blockers(state: &DaemonState, pipeline: &Pipeline) -> Vec<Blocker> {
    let names = must_be_on_disk(
        pipeline,
        state.stt_model(),
        models::stt_file(&state.config()),
        state.vad_model(),
    );
    assemble(permissions::blockers(), models::blockers(&names), pipeline)
}

// A blocker says the daemon does not work, so this answers for the listener
// that is running and never for a preset just picked: dictation carries on
// while a heavier model is still a download away.
//
// `stt_model` is seeded from the config at startup and corrected only by a load
// that succeeds, so outside an open pipeline it names a file nothing holds.
fn must_be_on_disk<'a>(
    pipeline: &Pipeline,
    loaded: Option<&'a str>,
    asked: Option<&'a str>,
    detector: &'a str,
) -> Vec<&'a str> {
    let speech = if matches!(pipeline, Pipeline::Open) {
        loaded
    } else {
        asked
    };
    speech.into_iter().chain([detector]).collect()
}

/// Split out so the present-but-dead branch is testable: a test daemon's model
/// names never exist on disk. Suppression covers model failures only, because
/// capture opens first and no absent file explains a dead microphone.
fn assemble(
    mut grants: Vec<Blocker>,
    missing_models: Vec<Blocker>,
    pipeline: &Pipeline,
) -> Vec<Blocker> {
    let recording_error = pipeline.fault();
    let models_explain_it =
        !missing_models.is_empty() && matches!(recording_error, Some(RecordingError::Model(_)));
    grants.extend(missing_models);
    if let Some(error) = recording_error
        && !models_explain_it
    {
        grants.push(Blocker {
            // A model that will not load is a model fault, whatever it stops.
            kind: match error {
                RecordingError::Model(_) => BlockerKind::Model,
                RecordingError::Microphone(_) => BlockerKind::Pipeline,
                RecordingError::Provider(_) => BlockerKind::Provider,
                RecordingError::KeyFile(_) => BlockerKind::KeyFile,
            },
            role: None,
            remedy: Some(banshee_common::Remedy::Restart),
            id: "recording_pipeline".to_string(),
            // Each carries the id above, so the name is what parts them for
            // a reader.
            name: match error {
                RecordingError::Model(_) => "Banshee needs a restart",
                RecordingError::Microphone(_) => "The microphone is not working",
                RecordingError::Provider(_) => "The remote listener is not reachable",
                RecordingError::KeyFile(_) => "The remote listener's key file is unreadable",
            }
            .to_string(),
            consequence: error.consequence(),
            fix: error.fix(),
            command: error.command().map(str::to_string),
        });
    }
    grants
}

#[cfg(test)]
mod tests {
    use super::{assemble, must_be_on_disk};
    use banshee_common::{Blocker, BlockerKind};

    use crate::state::{Pipeline, RecordingError};

    fn blocker(kind: BlockerKind, id: &str) -> Blocker {
        Blocker {
            kind,
            role: None,
            remedy: None,
            id: id.to_string(),
            name: id.to_string(),
            consequence: "recording does not work".to_string(),
            fix: "run: banshee setup".to_string(),
            command: Some("banshee setup".to_string()),
        }
    }

    const LOADED: &str = "ggml-large-v3-turbo-q5_0.bin";
    const ASKED: &str = "ggml-large-v3-q5_0.bin";
    const VAD: &str = "silero_vad.onnx";

    #[test]
    fn an_open_pipeline_is_judged_on_the_model_it_holds() {
        let names = must_be_on_disk(&Pipeline::Open, Some(LOADED), Some(ASKED), VAD);
        assert_eq!(names, vec![LOADED, VAD]);
    }

    #[test]
    fn a_broken_pipeline_is_judged_on_what_the_config_asks_for() {
        let dead = Pipeline::Broken(RecordingError::Model("absent".to_string()));
        let names = must_be_on_disk(&dead, Some(ASKED), Some(LOADED), VAD);
        assert_eq!(names, vec![LOADED, VAD]);
    }

    #[test]
    fn a_pipeline_still_opening_is_judged_on_what_the_config_asks_for() {
        let names = must_be_on_disk(&Pipeline::Opening, Some(ASKED), Some(LOADED), VAD);
        assert_eq!(names, vec![LOADED, VAD]);
    }

    #[test]
    fn a_remote_listener_leaves_only_the_detector() {
        assert_eq!(must_be_on_disk(&Pipeline::Open, None, None, VAD), vec![VAD]);
    }

    // A pipeline being built is not a fault, and a blocker asks the reader to
    // fix something. Waiting is not theirs to fix.
    #[test]
    fn a_pipeline_that_is_still_opening_raises_no_blocker() {
        assert!(assemble(vec![], vec![], &Pipeline::Opening).is_empty());
    }

    #[test]
    fn a_healthy_daemon_reports_nothing() {
        assert!(assemble(vec![], vec![], &Pipeline::Open).is_empty());
    }

    /// A client that routes by kind sends a model fault to its models step.
    /// Calling it a pipeline fault hides it behind whatever handles the
    /// microphone.
    /// A client routes on `command`, so this literal is a wire contract.
    #[test]
    fn a_dead_pipeline_names_the_command_a_client_routes_on() {
        let error = RecordingError::Model("missing file.".to_string());
        let blockers = assemble(vec![], vec![], &Pipeline::Broken(error.clone()));
        assert_eq!(blockers[0].command.as_deref(), Some("banshee start"));
    }

    #[test]
    fn a_model_that_will_not_load_reports_as_a_model_fault() {
        let error = RecordingError::Model("missing file.".to_string());
        let blockers = assemble(vec![], vec![], &Pipeline::Broken(error.clone()));
        assert_eq!(blockers[0].kind, BlockerKind::Model);
    }

    #[test]
    fn a_dead_microphone_reports_as_a_pipeline_fault() {
        let error = RecordingError::Microphone("no device".to_string());
        let blockers = assemble(vec![], vec![], &Pipeline::Broken(error.clone()));
        assert_eq!(blockers[0].kind, BlockerKind::Pipeline);
    }

    /// Every client titles the box with this, so a name that says "recording
    /// pipeline" says nothing a reader can act on.
    #[test]
    fn each_fault_names_itself_rather_than_the_pipeline_they_share() {
        let named = |error: RecordingError| {
            assemble(vec![], vec![], &Pipeline::Broken(error.clone()))[0]
                .name
                .clone()
        };
        assert_eq!(
            named(RecordingError::Microphone("no device".to_string())),
            "The microphone is not working"
        );
        assert_eq!(
            named(RecordingError::Provider("no key".to_string())),
            "The remote listener is not reachable"
        );
        assert_eq!(
            named(RecordingError::Model("missing file.".to_string())),
            "Banshee needs a restart"
        );
    }

    #[test]
    fn a_model_that_will_not_load_asks_for_a_restart() {
        let error = RecordingError::Model("missing file.".to_string());
        let blockers = assemble(vec![], vec![], &Pipeline::Broken(error.clone()));
        let [blocker] = &blockers[..] else {
            panic!("expected exactly one blocker, got {blockers:?}");
        };
        assert_eq!(blocker.kind, BlockerKind::Model);
        assert!(
            blocker.fix.contains("banshee start"),
            "the fix must name the restart: {}",
            blocker.fix
        );
        assert!(
            !blocker.consequence.ends_with('.'),
            "prose is punctuated by the renderer, not the producer: {}",
            blocker.consequence
        );
    }

    #[test]
    fn a_missing_model_is_not_dressed_up_as_a_stale_pipeline() {
        let error = RecordingError::Model("missing file".to_string());
        let blockers = assemble(
            vec![],
            vec![blocker(BlockerKind::Model, "ggml.bin")],
            &Pipeline::Broken(error.clone()),
        );
        let [blocker] = &blockers[..] else {
            panic!("the download is the whole fix, got {blockers:?}");
        };
        assert_eq!(blocker.kind, BlockerKind::Model);
    }

    // Capture opens before the models load, so both can be wrong at once.
    #[test]
    fn a_dead_microphone_survives_a_missing_model() {
        let error = RecordingError::Microphone("no device".to_string());
        let blockers = assemble(
            vec![],
            vec![blocker(BlockerKind::Model, "ggml.bin")],
            &Pipeline::Broken(error.clone()),
        );
        let kinds: Vec<_> = blockers.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![BlockerKind::Model, BlockerKind::Pipeline],
            "both faults are real and neither explains the other"
        );
    }

    #[test]
    fn a_microphone_is_not_told_to_just_restart() {
        let error = RecordingError::Microphone("no device".to_string());
        let blockers = assemble(vec![], vec![], &Pipeline::Broken(error.clone()));
        let fix = &blockers[0].fix;
        assert!(
            fix.contains("microphone"),
            "the fix must name the real cause: {fix}"
        );
        assert_ne!(fix, "restart it: banshee start");
    }

    /// A client that routes by kind headlines a pipeline fault as a dead
    /// microphone, and a rejected key is no microphone fault.
    #[test]
    fn a_dead_remote_listener_reports_as_a_provider_fault_with_the_key_command() {
        let error = RecordingError::Provider("no key for the remote listener".to_string());
        let blockers = assemble(vec![], vec![], &Pipeline::Broken(error.clone()));
        let [blocker] = &blockers[..] else {
            panic!("expected exactly one blocker, got {blockers:?}");
        };
        assert_eq!(blocker.kind, BlockerKind::Provider);
        assert_eq!(blocker.command.as_deref(), Some("banshee start"));
        assert!(
            blocker
                .fix
                .contains("banshee config set stt.remote.api_key"),
            "the fix must name the key command: {}",
            blocker.fix
        );
        assert!(!blocker.consequence.ends_with('.'));
    }

    #[test]
    fn an_unreadable_key_file_reports_as_a_key_file_fault_with_the_file_to_remove() {
        let error = RecordingError::KeyFile(
            "credentials.toml does not parse; fix it or delete it and set the keys again"
                .to_string(),
        );
        let blockers = assemble(vec![], vec![], &Pipeline::Broken(error.clone()));
        let [blocker] = &blockers[..] else {
            panic!("expected exactly one blocker, got {blockers:?}");
        };
        assert_eq!(blocker.kind, BlockerKind::KeyFile);
        assert_eq!(blocker.name, "The remote listener's key file is unreadable");
        assert!(
            blocker.fix.contains("credentials.toml"),
            "the fix must name the file to remove: {}",
            blocker.fix
        );
        assert!(
            !blocker.fix.contains("config set"),
            "a key written again reads the same file: {}",
            blocker.fix
        );
        assert!(!blocker.consequence.ends_with('.'));
    }

    #[test]
    fn a_grant_is_offered_before_a_download() {
        let blockers = assemble(
            vec![blocker(BlockerKind::Permission, "accessibility")],
            vec![blocker(BlockerKind::Model, "ggml.bin")],
            &Pipeline::Open,
        );
        let kinds: Vec<_> = blockers.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![BlockerKind::Permission, BlockerKind::Model],
            "one click should come before a gigabyte download"
        );
    }
}
