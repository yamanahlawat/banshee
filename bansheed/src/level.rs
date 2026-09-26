use std::sync::Arc;
use std::time::Duration;

use crate::state::{DaemonState, RecordingMode};

const SAMPLE_EVERY: Duration = Duration::from_millis(33);

pub fn metered(mode: RecordingMode, answer_open: bool, speaking: bool) -> bool {
    match mode {
        RecordingMode::PushToTalk | RecordingMode::ArmedHold => true,
        // Armed is already true while the question plays, and that voice is not the user's.
        RecordingMode::Armed => answer_open && !speaking,
        RecordingMode::Idle => false,
    }
}

pub async fn sample(state: Arc<DaemonState>) {
    let mut every = tokio::time::interval(SAMPLE_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut recording = state.subscribe_recording();
    let mut watchers = state.subscribe_level_watchers();
    let mut heard_last = false;
    loop {
        if !(state.is_recording() && state.is_level_watched()) {
            if heard_last {
                state.publish_level(0.0);
                heard_last = false;
            }
            let changed = tokio::select! {
                changed = recording.changed() => changed,
                changed = watchers.changed() => changed,
            };
            if changed.is_err() {
                return;
            }
            // Built while nobody watched, so it is no watcher's level.
            state.take_peak();
            continue;
        }
        every.tick().await;
        let peak = state.take_peak();
        let heard = metered(
            state.recording_mode(),
            state.is_answer_open(),
            state.speech().is_speaking(),
        );
        if heard {
            state.publish_level(peak);
        } else if heard_last {
            state.publish_level(0.0);
        }
        heard_last = heard;
    }
}

#[cfg(test)]
mod tests {
    use super::metered;
    use crate::state::RecordingMode;

    #[test]
    fn a_press_is_metered() {
        assert!(metered(RecordingMode::PushToTalk, false, false));
        assert!(metered(RecordingMode::ArmedHold, false, true));
    }

    #[test]
    fn an_armed_session_is_metered_only_while_the_answer_is_open_and_banshee_is_quiet() {
        assert!(!metered(RecordingMode::Armed, false, false));
        assert!(!metered(RecordingMode::Armed, true, true));
        assert!(metered(RecordingMode::Armed, true, false));
    }

    #[test]
    fn idle_is_never_metered() {
        assert!(!metered(RecordingMode::Idle, true, false));
    }

    #[tokio::test]
    async fn a_new_watcher_sees_no_peak_from_before_it_watched() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        assert!(state.record_start(crate::state::TranscribeTarget::Mailbox));
        state.note_peak(0.5);
        let mut level = state.subscribe_level();
        tokio::spawn(super::sample(std::sync::Arc::clone(&state)));

        tokio::time::sleep(super::SAMPLE_EVERY * 3).await;
        assert_eq!(*level.borrow_and_update(), 0.0, "nobody watches the level");

        let _watch = state.watch_level();
        // The sampler's first tick after a watch appears completes at once, so
        // one yield runs it.
        tokio::task::yield_now().await;
        assert_eq!(
            *level.borrow_and_update(),
            0.0,
            "the peak from before anyone watched is not this watcher's"
        );
        state.note_peak(0.25);
        tokio::time::timeout(std::time::Duration::from_secs(2), level.changed())
            .await
            .expect("a peak after the watch reaches the watcher")
            .unwrap();
        assert_eq!(*level.borrow(), 0.25);
    }

    #[tokio::test]
    async fn every_sample_is_published_the_same_value_too() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        assert!(state.record_start(crate::state::TranscribeTarget::Mailbox));
        let _watch = state.watch_level();
        let mut level = state.subscribe_level();
        tokio::spawn(super::sample(std::sync::Arc::clone(&state)));

        for _ in 0..3 {
            tokio::time::timeout(std::time::Duration::from_secs(2), level.changed())
                .await
                .expect("a silent sample still reaches the watcher")
                .unwrap();
            assert_eq!(*level.borrow_and_update(), 0.0);
        }
    }

    #[tokio::test]
    async fn sampling_stops_on_one_silence_and_an_idle_sampler_publishes_nothing() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        assert!(state.record_start(crate::state::TranscribeTarget::Mailbox));
        let watch = state.watch_level();
        let mut level = state.subscribe_level();
        tokio::spawn(super::sample(std::sync::Arc::clone(&state)));
        // A voice that never stops, so the last sample before the stop is loud.
        let speaking = std::sync::Arc::clone(&state);
        let voice = tokio::spawn(async move {
            loop {
                speaking.note_peak(0.5);
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            level.wait_for(|now| *now > 0.0),
        )
        .await
        .expect("sampling runs")
        .unwrap();

        drop(watch);
        tokio::time::sleep(super::SAMPLE_EVERY * 3).await;
        voice.abort();
        level.borrow_and_update();
        assert_eq!(*level.borrow(), 0.0, "sampling ends on silence");

        drop(state.watch_level());
        drop(state.watch_level());
        tokio::time::sleep(super::SAMPLE_EVERY * 3).await;
        assert!(
            !level.has_changed().unwrap(),
            "a wake while nothing samples publishes nothing"
        );
    }

    #[tokio::test]
    async fn a_microphone_that_stops_being_heard_publishes_one_silence() {
        let state = crate::test_support::daemon_state(std::sync::mpsc::channel().0);
        state.set_recording_mode(RecordingMode::PushToTalk);
        let _watch = state.watch_level();
        let mut level = state.subscribe_level();
        tokio::spawn(super::sample(std::sync::Arc::clone(&state)));
        // A voice that never stops, so every heard sample is loud.
        let speaking = std::sync::Arc::clone(&state);
        let voice = tokio::spawn(async move {
            loop {
                speaking.note_peak(0.5);
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            level.wait_for(|now| *now > 0.0),
        )
        .await
        .expect("a press is heard")
        .unwrap();

        // Still sampled, since a question waits, but its answer is not open yet.
        state.set_recording_mode(RecordingMode::Armed);
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            level.wait_for(|now| *now == 0.0),
        )
        .await
        .expect("the level falls to silence")
        .unwrap();
        tokio::time::sleep(super::SAMPLE_EVERY * 4).await;
        assert!(
            !level.has_changed().unwrap(),
            "an unheard microphone publishes silence once"
        );
        voice.abort();
    }
}
