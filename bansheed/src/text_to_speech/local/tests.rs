use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use banshee_common::{COMPANION_VOICE, KOKORO_MODEL};

use super::*;
use crate::test_support::{
    Asked, Listening, companion_models, companion_models_with, ignored_faults, on_first_use,
    write_empty,
};
use crate::text_to_speech::output::Output;
use crate::text_to_speech::{ActiveUtterance, TtsBackend, lock};

#[test]
fn a_synthesis_failure_is_reported_as_a_fault() {
    let (faults, reported) = std::sync::mpsc::channel();
    let mut played = false;

    let chunk = chunk_or_fault(
        "Kokoro",
        Err(BansheeError::Other("the style file is gone".into())),
        &mut played,
        &Faults::new(faults),
    );

    assert!(chunk.is_none(), "a failed sentence plays nothing");
    match reported.try_recv() {
        Ok(Fault::Failed(reason)) => assert!(
            reason.contains("the style file is gone"),
            "the fault carries the cause: {reason}"
        ),
        other => panic!("the failure reaches the fault channel, got {other:?}"),
    }
}

#[test]
fn the_first_chunk_reports_the_utterance_played_once() {
    let (faults, reported) = std::sync::mpsc::channel();
    let faults = Faults::new(faults);
    let mut played = false;

    let first = chunk_or_fault("Kokoro", Ok(vec![0.1, 0.2]), &mut played, &faults);
    let second = chunk_or_fault("Kokoro", Ok(vec![0.3]), &mut played, &faults);

    assert_eq!(first.map(|chunk| chunk.samples), Some(vec![0.1, 0.2]));
    assert_eq!(second.map(|chunk| chunk.samples), Some(vec![0.3]));
    assert!(
        matches!(reported.try_recv(), Ok(Fault::Played)),
        "the first sentence clears the last speech error"
    );
    assert!(
        reported.try_recv().is_err(),
        "the second sentence reports nothing new"
    );
}

/// The companion stands resident without its files, so whatever reaches the
/// models folder came from a voice of the other engine.
fn companion_resident(models: &Path) -> (LocalBackend, Asked) {
    let asked = Asked::default();
    let backend = LocalBackend {
        models: models.to_path_buf(),
        output: Arc::new(Output::silent()),
        faults: ignored_faults(),
        resident: RwLock::new(Resident {
            engine: Engine::Pocket,
            backend: Box::new(Listening(Arc::clone(&asked))),
        }),
    };
    (backend, asked)
}

fn refusal(started: Result<Box<dyn ActiveUtterance>, BansheeError>) -> BansheeError {
    match started {
        Err(error) => error,
        Ok(_) => panic!("this voice must not start"),
    }
}

#[test]
fn a_voice_of_the_resident_engine_goes_to_it() {
    let models = crate::test_support::unique_scratch("local-models");
    let (backend, asked) = companion_resident(&models);

    backend.start("Hello.", None).unwrap();
    backend.start("Again.", Some(COMPANION_VOICE)).unwrap();

    assert_eq!(
        lock(&asked).as_slice(),
        [
            ("Hello.".to_string(), None),
            ("Again.".to_string(), Some(COMPANION_VOICE.to_string()))
        ]
    );
}

#[test]
fn a_per_request_kokoro_voice_names_what_is_missing() {
    let models = crate::test_support::unique_scratch("local-models");
    let (backend, asked) = companion_resident(&models);

    let no_voice = refusal(backend.start("Hello.", Some("af_sky")));
    assert!(
        matches!(no_voice, BansheeError::Rejected(_)),
        "{no_voice:?}"
    );
    assert!(no_voice.to_string().contains("af_sky"), "{no_voice}");

    std::fs::write(models.join("af_sky.bin"), b"").unwrap();
    let no_engine = refusal(backend.start("Hello.", Some("af_sky"))).to_string();
    assert!(no_engine.contains(KOKORO_MODEL), "{no_engine}");
    assert!(no_engine.contains("banshee setup"), "{no_engine}");

    assert!(lock(&asked).is_empty(), "the companion was not asked");
}

#[test]
fn a_live_voice_that_will_not_load_keeps_the_resident() {
    let models = crate::test_support::unique_scratch("local-models");
    let (backend, asked) = companion_resident(&models);
    let tts = crate::config::TTSConfig {
        voice: "af_sky".to_string(),
        ..Default::default()
    };

    assert_eq!(backend.reconfigure(&tts), None);
    backend.start("Still here.", None).unwrap();
    assert_eq!(lock(&asked).len(), 1, "the companion still speaks");
}

fn until_finished(mut utterance: Box<dyn ActiveUtterance>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !utterance.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "the utterance never ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn listening() -> Result<Box<dyn TtsBackend>, BansheeError> {
    Ok(Box::new(Listening(Asked::default())))
}

fn write(voice: &str) -> crate::config::TTSConfig {
    crate::config::TTSConfig {
        voice: voice.to_string(),
        ..Default::default()
    }
}

#[test]
fn the_fallback_builds_once_and_keeps_what_it_built() {
    let builds = Arc::new(Mutex::new(0));
    let asked = Asked::default();
    let (counted, heard) = (Arc::clone(&builds), Arc::clone(&asked));
    let fallback = on_first_use(
        &companion_models(),
        move |_voice: &str| {
            *lock(&counted) += 1;
            Ok(Box::new(Listening(Arc::clone(&heard))) as Box<dyn TtsBackend>)
        },
        ignored_faults(),
    );
    assert_eq!(*lock(&builds), 0, "nothing loads before the first reply");

    until_finished(fallback.start("One.", None).unwrap());
    until_finished(fallback.start("Two.", None).unwrap());
    assert_eq!(*lock(&builds), 1);
    assert_eq!(lock(&asked).len(), 2);
}

#[test]
fn a_load_that_fails_once_speaks_on_the_next_utterance() {
    let tries = Arc::new(Mutex::new(0));
    let asked = Asked::default();
    let (counted, heard) = (Arc::clone(&tries), Arc::clone(&asked));
    let (faults, reported) = std::sync::mpsc::channel();
    let voice = on_first_use(
        &companion_models(),
        move |_voice: &str| {
            *lock(&counted) += 1;
            if *lock(&counted) == 1 {
                return Err(BansheeError::Other("the file is still arriving".into()));
            }
            Ok(Box::new(Listening(Arc::clone(&heard))) as Box<dyn TtsBackend>)
        },
        Faults::new(faults),
    );

    until_finished(voice.start("One.", None).unwrap());
    match reported.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Fault::Failed(reason)) => assert!(reason.contains("still arriving"), "{reason}"),
        other => panic!("the failed load is a fault: {other:?}"),
    }
    until_finished(voice.start("Two.", None).unwrap());
    assert_eq!(lock(&asked).as_slice(), [("Two.".to_string(), None)]);
}

#[test]
fn a_voice_write_before_the_first_load_needs_its_files() {
    let models = companion_models();
    let voice = on_first_use(&models, |_voice: &str| listening(), ignored_faults());

    assert_eq!(voice.reconfigure(&write("am_adam")), None);
    write_empty(&models, &[KOKORO_MODEL, "am_adam.bin"]);
    assert_eq!(
        voice.reconfigure(&write("am_adam")).as_deref(),
        Some("am_adam")
    );
}

#[test]
fn a_voice_written_during_the_first_load_is_the_one_kept() {
    let models = companion_models_with(&["am_adam"]);
    let loads: Arc<Mutex<Vec<String>>> = Arc::default();
    let (entered_to, entered) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let (seen, released) = (Arc::clone(&loads), Mutex::new(released));
    let voice = on_first_use(
        &models,
        move |voice: &str| {
            lock(&seen).push(voice.to_string());
            let _ = entered_to.send(());
            let _ = lock(&released).recv_timeout(std::time::Duration::from_secs(5));
            listening()
        },
        ignored_faults(),
    );

    let first = voice.start("One.", None).unwrap();
    entered
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the first load began");
    assert_eq!(
        voice.reconfigure(&write("am_adam")).as_deref(),
        Some("am_adam")
    );
    release.send(()).unwrap();
    until_finished(first);

    release.send(()).unwrap();
    until_finished(voice.start("Two.", None).unwrap());
    assert_eq!(
        lock(&loads).as_slice(),
        [COMPANION_VOICE.to_string(), "am_adam".to_string()]
    );
}

#[test]
fn an_unknown_per_request_voice_is_refused_before_the_first_load() {
    let loads = Arc::new(Mutex::new(0));
    let counted = Arc::clone(&loads);
    let voice = on_first_use(
        &companion_models(),
        move |_voice: &str| {
            *lock(&counted) += 1;
            listening()
        },
        ignored_faults(),
    );

    let refused = refusal(voice.start("Hello.", Some("zz_nobody")));
    assert!(matches!(refused, BansheeError::Rejected(_)), "{refused:?}");
    assert_eq!(
        *lock(&loads),
        0,
        "nothing loads for a voice that is not here"
    );
}

#[test]
fn a_per_request_voice_that_will_not_load_reports_a_fault() {
    let models = crate::test_support::unique_scratch("local-models");
    crate::test_support::write_empty(&models, &[KOKORO_MODEL, "af_sky.bin"]);
    let (faults, reported) = std::sync::mpsc::channel();
    let (mut backend, _asked) = companion_resident(&models);
    backend.faults = Faults::new(faults);

    let utterance = backend
        .start("Hello.", Some("af_sky"))
        .expect("the files are here, so the load runs on a thread of its own");
    match reported.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Fault::Failed(reason)) => assert!(reason.contains("af_sky"), "{reason}"),
        other => panic!("the failed load is a fault: {other:?}"),
    }
    until_finished(utterance);
}

#[test]
#[ignore = "needs the Kokoro model and a voice: cargo test kokoro -- --ignored"]
fn a_per_request_kokoro_voice_plays_beside_the_companion() {
    let models = crate::models::download::models_dir().expect("a home directory");
    let voice = crate::models::installed_voices_in(&models)
        .into_iter()
        .next()
        .expect("run `banshee setup` with a Kokoro voice first");
    let (faults, reported) = std::sync::mpsc::channel();
    let (mut backend, asked) = companion_resident(&models);
    backend.faults = Faults::new(faults);

    let mut utterance = backend.start("A Kokoro preview.", Some(&voice)).unwrap();
    assert!(
        matches!(
            reported.recv_timeout(std::time::Duration::from_secs(30)),
            Ok(Fault::Played)
        ),
        "{voice} played"
    );
    utterance.stop();
    assert!(lock(&asked).is_empty(), "the companion was not asked");
}

// A 0.16 install upgraded to the companion default holds Kokoro's files and
// not yet Pocket's, and the window still previews its Kokoro voices.
#[test]
fn a_per_request_voice_plays_while_the_configured_voice_is_missing() {
    let models = crate::test_support::models_holding(&[KOKORO_MODEL, "af_sky.bin"]);
    let loads = Arc::new(Mutex::new(0));
    let counted = Arc::clone(&loads);
    let (faults, reported) = std::sync::mpsc::channel();
    let voice = on_first_use(
        &models,
        move |_voice: &str| {
            *lock(&counted) += 1;
            listening()
        },
        Faults::new(faults),
    );

    let utterance = voice
        .start("A preview.", Some("af_sky"))
        .expect("af_sky's files are here");
    match reported.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Fault::Failed(reason)) => assert!(
            reason.contains("af_sky"),
            "the empty files load nothing, and the fault names the voice asked for: {reason}"
        ),
        other => panic!("the load of af_sky ends in a fault: {other:?}"),
    }
    until_finished(utterance);
    assert_eq!(*lock(&loads), 0, "the missing companion is not loaded");
}

#[test]
fn the_configured_voice_kept_on_its_first_load_reports_itself_loaded() {
    let (faults, reported) = std::sync::mpsc::channel();
    let voice = on_first_use(
        &companion_models(),
        |_voice: &str| listening(),
        Faults::new(faults),
    );

    until_finished(voice.start("One.", None).unwrap());
    assert!(
        matches!(
            reported.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(Fault::Loaded)
        ),
        "the load is reported"
    );
    until_finished(voice.start("Two.", None).unwrap());
    assert!(
        reported
            .recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a voice already kept loads nothing new"
    );
}

#[test]
fn a_voice_standing_in_for_the_remote_speaker_reports_no_load() {
    let (faults, reported) = std::sync::mpsc::channel();
    let voice = on_first_use(
        &companion_models(),
        |_voice: &str| listening(),
        Faults::new(faults).standing_in(),
    );

    until_finished(voice.start_unlocked("One.", None).unwrap());
    until_finished(voice.start("Two.", None).unwrap());
    let said = reported.recv_timeout(std::time::Duration::from_millis(200));
    assert!(
        said.is_err(),
        "the remote speaker is the one the config names: {said:?}"
    );
}

#[test]
fn a_load_that_panics_ends_its_utterance() {
    let utterance = start_once_loaded(
        COMPANION_VOICE,
        || panic!("the engine panicked while it loaded"),
        "Hello.",
        None,
        ignored_faults(),
    );
    until_finished(utterance);
}
