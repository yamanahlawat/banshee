use super::*;

/// A second installed voice, whichever this machine holds. Naming one would fix
/// the test to a file `banshee setup` does not fetch.
fn another_voice_than(loaded: &str) -> String {
    crate::models::installed_voices()
        .into_iter()
        .find(|id| id != loaded)
        .unwrap_or_else(|| {
            panic!(
                "this test swaps between two voices and only {loaded} is on this machine. \
                 `banshee voices` lists what is here. To fetch a second one:\n  \
                 banshee config set tts.voice af_bella\n  \
                 banshee setup\n  \
                 banshee config set tts.voice {loaded}"
            )
        })
}

/// The engine, or the reason it could not load. Every caller is `#[ignore]`d,
/// so a machine that runs one wants the failure.
fn engine_for(voice: &str) -> KokoroEngine {
    let config = KokoroTTSConfig::new(voice);
    KokoroEngine::new(&config, 1.0)
        .unwrap_or_else(|error| panic!("run `banshee setup` first: {voice} did not load: {error}"))
}

#[test]
#[ignore = "needs the Kokoro model and two voices: cargo test kokoro -- --ignored"]
fn a_voice_swap_changes_the_style_and_swaps_back() {
    let mut engine = engine_for("af_sky");
    let other = another_voice_than("af_sky");
    let first = engine.style_fingerprint();
    engine.set_voice(&other).expect("an installed voice");
    let second = engine.style_fingerprint();
    assert_ne!(first, second, "the fixture must discriminate");
    engine.set_voice("af_sky").expect("an installed voice");
    assert_eq!(engine.style_fingerprint(), first);
}

#[test]
#[ignore = "needs the Kokoro model and voices: cargo test kokoro -- --ignored"]
fn an_uninstalled_voice_is_refused_and_leaves_the_engine_speaking() {
    let mut engine = engine_for("af_sky");
    let before = engine.style_fingerprint();
    assert!(engine.set_voice("zz_nobody").is_err());
    assert_eq!(
        engine.style_fingerprint(),
        before,
        "a refused swap must not clear the voice"
    );
}

#[test]
#[ignore = "needs the Kokoro model and two voices: cargo test kokoro -- --ignored"]
fn an_ordinary_utterance_returns_the_engine_to_the_configured_voice() {
    let mut engine = engine_for("af_sky");
    let other = another_voice_than("af_sky");
    let configured = engine.style_fingerprint();
    engine.ensure_voice(&other).expect("an installed voice");
    assert_ne!(
        engine.style_fingerprint(),
        configured,
        "the fixture must discriminate"
    );
    engine.ensure_voice("af_sky").expect("an installed voice");
    assert_eq!(engine.style_fingerprint(), configured);
}

#[test]
#[ignore = "needs the Kokoro model and voices: cargo test kokoro -- --ignored"]
fn a_voice_outside_the_installed_set_is_refused_without_naming_a_path() {
    let mut engine = engine_for("af_sky");
    let models = banshee_common::utils::models_path().expect("a home directory");
    let error = engine
        .set_voice("../../../../etc/hosts")
        .unwrap_err()
        .to_string();
    assert!(
        !error.contains(models.to_str().expect("a printable path")),
        "the message must not name the path the daemon built: {error}"
    );
}

#[test]
fn a_voice_this_machine_does_not_hold_is_refused() {
    let held = vec!["af_sky".to_string(), "am_adam".to_string()];
    assert_eq!(voice_to_take("am_adam", &held), Some("am_adam".to_string()));
    assert_eq!(
        voice_to_take("am_nobody", &held),
        None,
        "a voice that is not installed would silence every later reply"
    );
}

#[test]
fn an_utterance_that_names_no_voice_takes_the_configured_one() {
    assert_eq!(voice_for(None, "am_adam"), "am_adam");
    assert_eq!(
        voice_for(Some("af_heart"), "am_adam"),
        "af_heart",
        "a named voice must win over the configured one"
    );
}

#[test]
#[ignore = "needs the Kokoro model and voices: cargo test kokoro -- --ignored"]
fn ensure_voice_on_the_loaded_voice_reads_nothing() {
    let mut engine = engine_for("af_sky");
    let before = engine.voice_ptr();
    engine.ensure_voice("af_sky").expect("already loaded");
    assert_eq!(
        engine.voice_ptr(),
        before,
        "the loaded voice must not be re-read"
    );
}

#[test]
fn flags_letter_spelled_words_only() {
    let mut spelled = MToken::new("nginx".into(), "NN".into(), " ".into());
    spelled.phonemes = Some("ˈɛn dʒˈiː ˈaɪ ɛn ɛks".into());
    assert!(is_letter_spelled(&spelled));

    let mut resolved = MToken::new("build".into(), "NN".into(), " ".into());
    resolved.phonemes = Some("bˈɪld".into());
    assert!(!is_letter_spelled(&resolved));

    let mut hyphenated = MToken::new("twenty-one".into(), "CD".into(), " ".into());
    hyphenated.phonemes = Some("twˈɛnti wˈʌn".into());
    assert!(!is_letter_spelled(&hyphenated));
}

// espeak-ng is one apt package, so CI installs it and this runs there. A
// machine without it skips, unless it asked for the fixtures.
#[test]
fn espeak_resolves_a_letter_spelled_word() {
    let Some(oov) = OovFallback::detect() else {
        assert!(
            std::env::var_os("BANSHEE_REQUIRE_FIXTURES").is_none(),
            "espeak-ng is missing where the fixtures were required"
        );
        eprintln!("espeak-ng not installed; skipping");
        return;
    };
    let word = "kustomize"; // not in misaki gold or our tables
    let mut g2p = G2P::new(Language::EnglishUS);
    let spelled = g2p.g2p(word).unwrap().1;
    assert!(
        spelled.iter().any(is_letter_spelled),
        "expected {word} to start out letter-spelled"
    );

    let phonemes = oov.phonemize(word).expect("espeak should phonemize");
    g2p.lexicon
        .golds
        .insert(word.to_string(), PhonemeEntry::Simple(phonemes));

    let after = g2p.g2p(word).unwrap().1;
    assert!(
        !after.iter().any(is_letter_spelled),
        "espeak phonemes should have resolved {word}"
    );
}

#[test]
fn g2p_output_maps_into_vocab() {
    let g2p = G2P::new(Language::EnglishUS);
    let (phonemes, _) = g2p.g2p("Hello, world!").unwrap();
    let ids: Vec<i64> = phonemes.chars().filter_map(token_id).collect();
    assert!(!ids.is_empty(), "no phonemes mapped for: {phonemes}");
    // Most of the phoneme string should map; a low ratio means vocab drift
    assert!(ids.len() * 2 >= phonemes.chars().count());
}

// Needs the real model on disk: cargo test kokoro_synthesizes -- --ignored
#[test]
#[ignore]
fn kokoro_synthesizes_audible_speech() {
    let config = KokoroTTSConfig::new("af_sky");
    let mut engine = KokoroEngine::new(&config, 1.0).unwrap();
    let samples = engine.synthesize("Kokoro is alive.").unwrap();
    // ~1s of speech at 24kHz, with actual signal in it
    assert!(samples.len() > 10_000, "too few samples: {}", samples.len());
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "output is near-silence, peak {peak}");
}

// The all-caps form is the one misaki tags NNP and spells out, so it is the one
// espeak would be asked about.
#[test]
fn a_curated_word_is_curated_in_every_casing() {
    let mut g2p = G2P::new(Language::EnglishUS);
    crate::text_to_speech::pronunciation::install_dictionary(&mut g2p);
    for word in ["webhook", "Webhook", "WEBHOOK"] {
        assert!(curated(&g2p.lexicon, word), "{word} must count as curated");
    }
    assert!(!curated(&g2p.lexicon, "Zzzq"));
}

#[test]
fn sentences_split_on_terminators() {
    let chunks: Vec<&str> = sentences("Done with the build. Tests pass! Ready?").collect();
    assert_eq!(
        chunks,
        vec!["Done with the build.", "Tests pass!", "Ready?"]
    );
    assert_eq!(
        sentences("no terminator").collect::<Vec<_>>(),
        vec!["no terminator"]
    );
}

#[test]
fn a_mid_token_terminator_does_not_end_a_sentence() {
    assert_eq!(
        sentences("Release 0.12.1 is out.").collect::<Vec<_>>(),
        vec!["Release 0.12.1 is out."]
    );
    assert_eq!(
        sentences("Speed is 1.2 now.").collect::<Vec<_>>(),
        vec!["Speed is 1.2 now."]
    );
    assert_eq!(
        sentences("I changed config.toml and main.rs.").collect::<Vec<_>>(),
        vec!["I changed config.toml and main.rs."]
    );
}

#[test]
fn a_terminator_still_ends_a_sentence_before_whitespace_or_text_end() {
    assert_eq!(
        sentences("Built 0.12.1. Tests pass! Ready?").collect::<Vec<_>>(),
        vec!["Built 0.12.1.", "Tests pass!", "Ready?"]
    );
}

#[test]
fn a_terminator_before_a_closing_quote_or_bracket_ends_the_sentence_after_it() {
    assert_eq!(
        sentences(r#"He said "Stop!" Then he left. Now go."#).collect::<Vec<_>>(),
        vec![r#"He said "Stop!""#, "Then he left.", "Now go."]
    );
    assert_eq!(
        sentences("Work is done (finally.) Next task is queued.").collect::<Vec<_>>(),
        vec!["Work is done (finally.)", "Next task is queued."]
    );
}

#[test]
fn an_initialism_is_never_split_between_its_letters() {
    assert_eq!(
        sentences("U.S.A. is here.").collect::<Vec<_>>(),
        vec!["U.S.A.", "is here."]
    );
}

#[test]
fn a_run_of_closers_stays_with_the_sentence_it_ends() {
    assert_eq!(
        sentences(r#"(He said "Stop!") Next."#).collect::<Vec<_>>(),
        vec![r#"(He said "Stop!")"#, "Next."]
    );
}

#[test]
fn every_closing_bracket_ends_the_sentence_after_it() {
    for text in [
        "Work is done (finally.) Next task is queued.",
        "Work is done [finally.] Next task is queued.",
        "Work is done {finally.} Next task is queued.",
    ] {
        let chunks = sentences(text).collect::<Vec<_>>();
        assert_eq!(chunks.len(), 2, "{text:?} gave {chunks:?}");
    }
}

#[test]
fn a_synthesis_failure_is_reported_as_a_fault() {
    let (faults, reported) = std::sync::mpsc::channel();
    let mut played = false;

    let chunk = chunk_or_fault(
        Err(BansheeError::Other("the style file is gone".into())),
        &mut played,
        &faults,
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
    let mut played = false;

    let first = chunk_or_fault(Ok(vec![0.1, 0.2]), &mut played, &faults);
    let second = chunk_or_fault(Ok(vec![0.3]), &mut played, &faults);

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
