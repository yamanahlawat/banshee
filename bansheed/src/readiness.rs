use banshee_common::{Blocker, BlockerKind};

use crate::state::{DaemonState, RecordingError};
use crate::{models, permissions};

pub fn blockers(state: &DaemonState) -> Vec<Blocker> {
    assemble(
        permissions::blockers(),
        models::blockers(&[state.stt_model(), state.vad_model()]),
        state.recording_error(),
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
            kind: BlockerKind::Pipeline,
            id: "recording_pipeline".to_string(),
            name: "Recording pipeline".to_string(),
            consequence: error.consequence(),
            fix: error.fix().to_string(),
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
            id: id.to_string(),
            name: id.to_string(),
            consequence: "recording does not work".to_string(),
            fix: "run: banshee setup".to_string(),
        }
    }

    #[test]
    fn a_healthy_daemon_reports_nothing() {
        assert!(assemble(vec![], vec![], None).is_empty());
    }

    #[test]
    fn a_dead_pipeline_with_every_model_present_asks_for_a_restart() {
        let error = RecordingError::Model("missing file.".to_string());
        let blockers = assemble(vec![], vec![], Some(&error));
        let [blocker] = &blockers[..] else {
            panic!("expected exactly one blocker, got {blockers:?}");
        };
        assert_eq!(blocker.kind, BlockerKind::Pipeline);
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

    #[test]
    fn a_grant_is_offered_before_a_download() {
        let blockers = assemble(
            vec![blocker(BlockerKind::Permission, "input_monitoring")],
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
