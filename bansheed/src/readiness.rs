use banshee_common::{Blocker, BlockerKind};

use crate::state::{DaemonState, RecordingError};
use crate::{models, permissions};

pub fn blockers(state: &DaemonState) -> Vec<Blocker> {
    // The loaded model, not the configured one, and none for a remote listener
    let names: Vec<&str> = state
        .stt_model()
        .into_iter()
        .chain([state.vad_model()])
        .collect();
    assemble(
        permissions::blockers(),
        models::blockers(&names),
        state.recording_error().as_ref(),
    )
}

/// Split out so the present-but-dead branch is testable: a test daemon's model
/// names never exist on disk. Suppression covers model failures only, because
/// capture opens first and no absent file explains a dead microphone.
fn assemble(
    mut grants: Vec<Blocker>,
    missing_models: Vec<Blocker>,
    recording_error: Option<&RecordingError>,
) -> Vec<Blocker> {
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
            },
            role: None,
            remedy: Some(banshee_common::Remedy::Restart),
            id: "recording_pipeline".to_string(),
            // All three carry the id above, so the name is what parts them for
            // a reader.
            name: match error {
                RecordingError::Model(_) => "Banshee needs a restart",
                RecordingError::Microphone(_) => "The microphone is not working",
                RecordingError::Provider(_) => "The remote listener is not reachable",
            }
            .to_string(),
            consequence: error.consequence(),
            fix: error.fix().to_string(),
            command: error.command().map(str::to_string),
        });
    }
    grants
}

#[cfg(test)]
mod tests {
    use super::assemble;
    use banshee_common::{Blocker, BlockerKind};

    use crate::state::RecordingError;

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

    #[test]
    fn a_healthy_daemon_reports_nothing() {
        assert!(assemble(vec![], vec![], None).is_empty());
    }

    /// A client that routes by kind sends a model fault to its models step.
    /// Calling it a pipeline fault hides it behind whatever handles the
    /// microphone.
    /// A client routes on `command`, so this literal is a wire contract.
    #[test]
    fn a_dead_pipeline_names_the_command_a_client_routes_on() {
        let error = RecordingError::Model("missing file.".to_string());
        let blockers = assemble(vec![], vec![], Some(&error));
        assert_eq!(blockers[0].command.as_deref(), Some("banshee start"));
    }

    #[test]
    fn a_model_that_will_not_load_reports_as_a_model_fault() {
        let error = RecordingError::Model("missing file.".to_string());
        let blockers = assemble(vec![], vec![], Some(&error));
        assert_eq!(blockers[0].kind, BlockerKind::Model);
    }

    #[test]
    fn a_dead_microphone_reports_as_a_pipeline_fault() {
        let error = RecordingError::Microphone("no device".to_string());
        let blockers = assemble(vec![], vec![], Some(&error));
        assert_eq!(blockers[0].kind, BlockerKind::Pipeline);
    }

    /// Every client titles the box with this, so a name that says "recording
    /// pipeline" says nothing a reader can act on.
    #[test]
    fn each_fault_names_itself_rather_than_the_pipeline_they_share() {
        let named = |error: RecordingError| assemble(vec![], vec![], Some(&error))[0].name.clone();
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
        let blockers = assemble(vec![], vec![], Some(&error));
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
            Some(&error),
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
            Some(&error),
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
        let blockers = assemble(vec![], vec![], Some(&error));
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
        let blockers = assemble(vec![], vec![], Some(&error));
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
    fn a_grant_is_offered_before_a_download() {
        let blockers = assemble(
            vec![blocker(BlockerKind::Permission, "accessibility")],
            vec![blocker(BlockerKind::Model, "ggml.bin")],
            None,
        );
        let kinds: Vec<_> = blockers.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![BlockerKind::Permission, BlockerKind::Model],
            "one click should come before a gigabyte download"
        );
    }
}
